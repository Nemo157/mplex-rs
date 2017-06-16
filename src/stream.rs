use std::mem;

use futures::{ Future, Sink, Stream, Poll, Async, StartSend, AsyncSink };
use futures::unsync::mpsc;

use message::{ Message, Flag };

pub struct Error(mpsc::SendError<Message>);

pub struct MultiplexStream(StreamImpl);

pub enum StreamImpl {
    Active {
        id: u64,
        flag: Flag,
        incoming: mpsc::Receiver<Message>,
        outgoing: mpsc::Sender<Message>,
    },
    ClosingMessage {
        id: u64,
        outgoing: mpsc::Sender<Message>,
    },
    ClosingOutgoing {
        outgoing: mpsc::Sender<Message>,
    },
    Closed,
}

impl MultiplexStream {
    pub(crate) fn new(id: u64, flag: Flag, incoming: mpsc::Receiver<Message>, outgoing: mpsc::Sender<Message>) -> impl Future<Item=MultiplexStream, Error=Error> {
        outgoing.send(Message {
                stream_id: id,
                flag: Flag::NewStream,
                data: Vec::new(),
            })
            .map_err(From::from)
            .map(move |outgoing| {
                MultiplexStream(StreamImpl::Active {
                    id, flag, incoming, outgoing
                })
            })
    }
}

impl Stream for MultiplexStream {
    type Item = Vec<u8>;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.0.poll()
    }
}

impl Sink for MultiplexStream {
    type SinkItem = Vec<u8>;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.0.start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.0.poll_complete()
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        self.0.close()
    }
}

impl Stream for StreamImpl {
    type Item = Vec<u8>;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match *self {
            StreamImpl::Active { ref mut incoming, .. } => {
                match try_ready!(incoming.poll()) {
                    None => {
                        return Ok(Async::Ready(None));
                    }
                    Some(msg) => {
                        if msg.flag != Flag::Close {
                            return Ok(Async::Ready(Some(msg.data)));
                        }
                    }
                }
            }
            _ => {
                return Ok(Async::Ready(None))
            }
        }
        // Must have received a message with flag Close, everything else early exits
        // TODO: Do we need to cleanup better here?
        *self = StreamImpl::Closed;
        Ok(Async::Ready(None))
    }
}

impl Sink for StreamImpl {
    type SinkItem = Vec<u8>;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match *self {
            StreamImpl::Active { id, flag, ref mut outgoing, .. } => {
                let msg = Message {
                    stream_id: id,
                    flag: flag,
                    data: item,
                };
                Ok(match outgoing.start_send(msg)? {
                    AsyncSink::Ready => AsyncSink::Ready,
                    AsyncSink::NotReady(msg) => AsyncSink::NotReady(msg.data),
                })
            }
            _ => panic!("Called start_send after close"),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(match *self {
            StreamImpl::Active { ref mut outgoing, .. } => {
                outgoing.poll_complete()?
            }
            _ => panic!("Called poll_complete after close"),
        })
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        *self = match mem::replace(self, StreamImpl::Closed) {
            StreamImpl::Active { id, outgoing, .. } => {
                StreamImpl::ClosingMessage { id, outgoing }
            }
            StreamImpl::ClosingMessage { id, mut outgoing } => {
                let msg = Message {
                    stream_id: id,
                    flag: Flag::Close,
                    data: Vec::new(),
                };
                match outgoing.start_send(msg)? {
                    AsyncSink::Ready => {
                        StreamImpl::ClosingOutgoing { outgoing }
                    }
                    AsyncSink::NotReady(_) => {
                        StreamImpl::ClosingMessage { id, outgoing }
                    }
                }
            }
            StreamImpl::ClosingOutgoing { mut outgoing } => {
                match outgoing.close()? {
                    Async::Ready(()) => {
                        StreamImpl::Closed
                    }
                    Async::NotReady => {
                        StreamImpl::ClosingOutgoing { outgoing }
                    }
                }
            }
            StreamImpl::Closed => {
                return Ok(Async::Ready(()));
            }
        };
        Ok(Async::NotReady)
    }
}

impl From<mpsc::SendError<Message>> for Error {
    fn from(err: mpsc::SendError<Message>) -> Error { Error(err) }
}
