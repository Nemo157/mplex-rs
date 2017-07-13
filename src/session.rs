use std::io;

use bytes::BytesMut;
use futures::{ Sink, Stream, Poll, Async, StartSend, AsyncSink };
use msgio::MsgIo;
use tokio_io::codec::{Encoder, Decoder};

use message::{Codec, Message};

pub struct Session<S: MsgIo> {
    transport: S,
    buffer: BytesMut,
}

impl<S: MsgIo> Session<S> {
    pub fn new(transport: S) -> Session<S> {
        Session {
            transport: transport,
            buffer: BytesMut::new(),
        }
    }
}

impl<S: MsgIo> Stream for Session<S> {
    type Item = Message;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        while let Some(chunk) = try_ready!(self.transport.poll()) {
            self.buffer.extend_from_slice(&chunk);
            if let Some(msg) = Codec.decode(&mut self.buffer)? {
                println!("mplex session poll msg: {:?}", msg);
                return Ok(Async::Ready(Some(msg)));
            }
        }
        Ok(Async::Ready(None))
    }
}

impl<S: MsgIo> Sink for Session<S> {
    type SinkItem = Message;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let mut buffer = BytesMut::new();
        Codec.encode(item, &mut buffer)?;
        Ok(match self.transport.start_send(buffer)? {
            AsyncSink::Ready => AsyncSink::Ready,
            AsyncSink::NotReady(mut bytes) => {
                let msg = Codec.decode(&mut bytes)?
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
