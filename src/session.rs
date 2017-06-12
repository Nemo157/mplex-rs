use std::io;

use futures::{ Sink, Stream, Poll, Async, StartSend, AsyncSink };
use futures_mpsc as mpsc;

use message::Message;

pub struct Session<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    transport: S,
    buffer: Option<Vec<u8>>,
    incoming: mpsc::Sender<Message>,
    max_msg_size: usize,
}

impl<S> Session<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    pub fn create(transport: S) -> (Session<S>, mpsc::Receiver<Message>) {
        let max_msg_size = 1 * 1024 * 1024;
        let (sender, receiver) = mpsc::channel(0);
        let session = Session {
            transport: transport,
            buffer: Some(Vec::with_capacity(max_msg_size)),
            incoming: sender,
            max_msg_size: max_msg_size,
        };
        (session, receiver)
    }
}

impl<S> Stream for Session<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    type Item = Message;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        while let Some(mut block) = try_ready!(self.transport.poll()) {
            let mut buffer = self.buffer.take().expect("We always replace it after use");
            buffer.append(&mut block);
            let (msg, buffer) = Message::try_from(buffer)?;
            if let Some(msg) = msg {
                self.buffer = Some(buffer);
                return Ok(Async::Ready(Some(msg)));
            } else if buffer.len() > self.max_msg_size + 20 { // TODO: Must be enough of a buffer for 2 128 bit varints
                self.buffer = Some(buffer);
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Exceeded max message size"));
            }
        }
        if self.buffer.as_ref().expect("We always replace it after use").is_empty() {
            Ok(Async::Ready(None))
        } else {
            Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
        }
    }
}

impl<S> Sink for Session<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    type SinkItem = Message;
    type SinkError = io::Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        Ok(match self.transport.start_send(item.into_bytes())? {
            AsyncSink::Ready => AsyncSink::Ready,
            AsyncSink::NotReady(bytes) => {
                let msg = Message::try_from(bytes)?.0
                    .expect("We created it, it has to be good");
                AsyncSink::NotReady(msg)
            }
        })
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.transport.poll_complete()
    }
}
