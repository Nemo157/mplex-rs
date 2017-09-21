use std::{cmp, fmt, mem};
use std::io::{self, Cursor};

use bytes::{Buf, Bytes};
use futures::{Future, Sink, Stream, Poll, Async, StartSend, AsyncSink};
use futures::prelude::{async, stream_yield};
use futures::unsync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};

use message::{Message, Flag};

pub struct MultiplexStream {
    done: bool,
    buffer: Cursor<Bytes>,
    stream: Box<Stream<Item=Bytes, Error=io::Error>>,
    sink: SinkImpl,
}

pub(crate) enum SinkImpl {
    Active {
        id: u64,
        flag: Flag,
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
                    stream: stream_impl(Flag::Receiver, incoming),
                    sink: SinkImpl::Active { id, flag: Flag::Initiator, outgoing },
                }
            })
    }

    pub(crate) fn receive(id: u64, incoming: mpsc::Receiver<Message>, outgoing: mpsc::Sender<Message>) -> MultiplexStream {
        MultiplexStream {
            done: false,
            buffer: Cursor::new(Bytes::new()),
            stream: stream_impl(Flag::Initiator, incoming),
            sink: SinkImpl::Active { id, flag: Flag::Receiver, outgoing },
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

            match self.stream.poll()? {
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
        match self.sink.start_send(Bytes::from(buf))? {
            AsyncSink::Ready => Ok(buf.len()),
            AsyncSink::NotReady(_) => Err(io::Error::new(io::ErrorKind::WouldBlock, "stream not ready to send")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // TODO: This still doesn't ensure the message was fully sent over the session
        match self.sink.poll_complete()? {
            Async::Ready(()) => Ok(()),
            Async::NotReady => Err(io::Error::new(io::ErrorKind::WouldBlock, "stream not done sending")),
        }
    }
}

impl AsyncRead for MultiplexStream {
}

impl AsyncWrite for MultiplexStream {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.sink.close()
    }
}

#[async]
fn stream_impl(flag: Flag, incoming: mpsc::Receiver<Message>) -> Box<Stream<Item=Bytes, Error=io::Error>> {
    #[async]
    for msg in incoming.map_err(unknown) {
        if msg.flag == flag {
            stream_yield!(msg.data)
        } else if msg.flag == Flag::Close {
            return Ok(());
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unexpected flag value {:?}", flag)));
        }
    }
    println!("Unexpected stream closure");
    Ok(())
}

impl Sink for SinkImpl {
    type SinkItem = Bytes;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match *self {
            SinkImpl::Active { id, flag, ref mut outgoing } => {
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
            SinkImpl::Active { ref mut outgoing, .. } => {
                outgoing.poll_complete().map_err(other)?
            }
            _ => panic!("Called poll_complete after close"),
        })
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        *self = match mem::replace(self, SinkImpl::Closed) {
            SinkImpl::Active { id, outgoing, .. } => {
                SinkImpl::ClosingMessage { id, outgoing }
            }
            SinkImpl::ClosingMessage { id, mut outgoing } => {
                let msg = Message {
                    stream_id: id,
                    flag: Flag::Close,
                    data: Bytes::new(),
                };
                match outgoing.start_send(msg).map_err(other)? {
                    AsyncSink::Ready => {
                        SinkImpl::ClosingOutgoing { outgoing }
                    }
                    AsyncSink::NotReady(_) => {
                        SinkImpl::ClosingMessage { id, outgoing }
                    }
                }
            }
            SinkImpl::ClosingOutgoing { mut outgoing } => {
                match outgoing.close().map_err(other)? {
                    Async::Ready(()) => {
                        SinkImpl::Closed
                    }
                    Async::NotReady => {
                        SinkImpl::ClosingOutgoing { outgoing }
                    }
                }
            }
            SinkImpl::Closed => {
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

impl fmt::Debug for MultiplexStream {
   fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
       f.debug_struct("MultiplexStream")
           .field("done", &self.done)
           .field("buffer", &self.buffer)
           .field("stream", &"<omitted>")
           .field("sink", &self.sink)
           .finish()
   }
}

impl fmt::Debug for SinkImpl {
   fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
       match *self {
           SinkImpl::Active { ref id, ref flag, ref outgoing } =>
               f.debug_struct("Active")
                   .field("id", id)
                   .field("flag", flag)
                   .field("outgoing", outgoing)
                   .finish(),
           SinkImpl::ClosingMessage { ref id, ref outgoing } =>
               f.debug_struct("ClosingMessage")
                   .field("id", id)
                   .field("outgoing", outgoing)
                   .finish(),
           SinkImpl::ClosingOutgoing { ref outgoing } =>
               f.debug_struct("ClosingOutgoing")
                   .field("outgoing", outgoing)
                   .finish(),
           SinkImpl::Closed =>
               f.debug_struct("Closed")
                   .finish(),
       }
   }
}
