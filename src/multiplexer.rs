use std::io;
use std::collections::HashMap;

use futures::{ Future, Sink, Stream, Poll };
use futures_mpsc as mpsc;

use stream::{ MultiplexStream, Error };
use message::{ Message, Flag };
use session::Session;

pub struct Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    session: Session<S>,
    initiator: bool,
    next_id: u64,
    stream_incoming: HashMap<u64, mpsc::Sender<Message>>,
    stream_outgoing: Vec<mpsc::Receiver<Message>>,
}

impl<S> Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    pub fn new(transport: S, initiator: bool) -> Multiplexer<S> {
        Multiplexer {
            session: Session::new(transport),
            initiator: initiator,
            next_id: 0,
            stream_incoming: Default::default(),
            stream_outgoing: Default::default(),
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 2;
        if self.initiator { id + 1 } else { id }
    }

    pub fn new_stream(&mut self) -> impl Future<Item=MultiplexStream, Error=Error> {
        let id = self.next_id();
        let flag = if self.initiator { Flag::Initiator } else { Flag::Receiver };
        let (in_sender, in_receiver) = mpsc::channel(1);
        let (out_sender, out_receiver) = mpsc::channel(1);
        self.stream_incoming.insert(id, in_sender);
        self.stream_outgoing.push(out_receiver);
        MultiplexStream::new(id, flag, in_receiver, out_sender)
    }
}

impl<S> Future for Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.session.poll()?;
        unimplemented!()
    }
}
