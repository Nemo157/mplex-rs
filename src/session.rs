use std::io;

use futures::{ Sink, Stream, Poll, Async, StartSend, AsyncSink };
use msgio::MsgIo;

use message::Message;

pub struct Session<S: MsgIo> {
    transport: S,
}

impl<S: MsgIo> Session<S> {
    pub fn new(transport: S) -> Session<S> {
        Session {
            transport: transport,
        }
    }
}

impl<S: MsgIo> Stream for Session<S> {
    type Item = Message;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(buffer) = try_ready!(self.transport.poll()) {
            let msg = Message::try_from(buffer)?;
            Ok(Async::Ready(Some(msg)))
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<S: MsgIo> Sink for Session<S> {
    type SinkItem = Message;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        Ok(match self.transport.start_send(item.into_bytes())? {
            AsyncSink::Ready => AsyncSink::Ready,
            AsyncSink::NotReady(bytes) => {
                let msg = Message::try_from(bytes)
                    .expect("We created it, it has to be good");
                AsyncSink::NotReady(msg)
            }
        })
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.transport.poll_complete()
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        self.transport.close()
    }
}
