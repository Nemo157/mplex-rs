use std::{cmp, mem};
use std::io::{self, Cursor};

use bytes::{Buf, Bytes};
use futures::{ Future, Sink, Stream, Poll, Async, StartSend, AsyncSink };
use futures::unsync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};

use message::{ Message, Flag };

#[derive(Debug)]
pub struct MultiplexStream {
    done: bool,
    buffer: Cursor<Bytes>,
    inner: StreamImpl,
}

#[derive(Debug)]
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
    pub(crate) fn initiate(id: u64, incoming: mpsc::Receiver<Message>, outgoing: mpsc::Sender<Message>) -> impl Future<Item=MultiplexStream, Error=io::Error> {
        outgoing.send(Message {
                stream_id: id,
                flag: Flag::NewStream,
                data: Bytes::new(),
            })
            .map_err(other)
            .map(move |outgoing| {
                MultiplexStream {
                    done: false,
                    buffer: Cursor::new(Bytes::new()),
                    inner: StreamImpl::Active {
                        id, flag: Flag::Initiator, incoming, outgoing
                    },
                }
            })
    }

    pub(crate) fn receive(id: u64, incoming: mpsc::Receiver<Message>, outgoing: mpsc::Sender<Message>) -> MultiplexStream {
        MultiplexStream {
            done: false,
            buffer: Cursor::new(Bytes::new()),
            inner: StreamImpl::Active {
                id, flag: Flag::Receiver, incoming, outgoing
            },
        }
    }
}

impl io::Read for MultiplexStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.done {
                return Ok(0);
            }

            if self.buffer.remaining() > 0 {
                let len = cmp::min(self.buffer.remaining(), buf.len());
                self.buffer.copy_to_slice(&mut buf[..len]);
                return Ok(len);
            }

            match self.inner.poll()? {
                Async::Ready(Some(buffer)) => {
                    self.buffer = Cursor::new(buffer);
                }
                Async::Ready(None) => {
                    self.done = true;
                }
                Async::NotReady => {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "no data ready"));
                }
            }
        }
    }
}

impl io::Write for MultiplexStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.start_send(Bytes::from(buf))? {
            AsyncSink::Ready => Ok(buf.len()),
            AsyncSink::NotReady(_) => Err(io::Error::new(io::ErrorKind::WouldBlock, "stream not ready to send")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // TODO: This still doesn't ensure the message was fully sent over the session
        match self.inner.poll_complete()? {
            Async::Ready(()) => Ok(()),
            Async::NotReady => Err(io::Error::new(io::ErrorKind::WouldBlock, "stream not done sending")),
        }
    }
}

impl AsyncRead for MultiplexStream {
}

impl AsyncWrite for MultiplexStream {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.inner.close()
    }
}

impl Stream for StreamImpl {
    type Item = Bytes;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match *self {
            StreamImpl::Active { ref mut incoming, .. } => {
                match try_ready!(incoming.poll().map_err(unknown)) {
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
    type SinkItem = Bytes;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match *self {
            StreamImpl::Active { id, flag, ref mut outgoing, .. } => {
                let msg = Message {
                    stream_id: id,
                    flag: flag,
                    data: Bytes::from(item),
                };
                Ok(match outgoing.start_send(msg).map_err(other)? {
                    AsyncSink::Ready => AsyncSink::Ready,
                    AsyncSink::NotReady(msg) => {
                        AsyncSink::NotReady(msg.data)
                    }
                })
            }
            _ => panic!("Called start_send after close"),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(match *self {
            StreamImpl::Active { ref mut outgoing, .. } => {
                outgoing.poll_complete().map_err(other)?
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
                    data: Bytes::new(),
                };
                match outgoing.start_send(msg).map_err(other)? {
                    AsyncSink::Ready => {
                        StreamImpl::ClosingOutgoing { outgoing }
                    }
                    AsyncSink::NotReady(_) => {
                        StreamImpl::ClosingMessage { id, outgoing }
                    }
                }
            }
            StreamImpl::ClosingOutgoing { mut outgoing } => {
                match outgoing.close().map_err(other)? {
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

fn unknown(_: ()) -> io::Error {
    io::Error::new(io::ErrorKind::Other, "Unknown error")
}

fn other<T: ::std::error::Error + Send + Sync + 'static>(err: T) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}
