use std::io;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use futures::{ future, Future, Sink, Stream, Poll, Async, AsyncSink };
use futures::unsync::mpsc;

use stream::{ self, MultiplexStream };
use message::{ Message, Flag };
use session::Session;

pub enum Error {
    Send(stream::Error),
    IO(io::Error),
}

pub struct Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    session: Session<S>,
    initiator: bool,
    next_id: u64,
    stream_senders: HashMap<u64, mpsc::Sender<Message>>,
    busy_streams: Vec<u64>,
    out_sender: mpsc::Sender<Message>,
    out_receiver: mpsc::Receiver<Message>,
    outstanding_msg: Option<Message>,
}

impl<S> Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    pub fn new(transport: S, initiator: bool) -> Multiplexer<S> {
        let (out_sender, out_receiver) = mpsc::channel(1);
        Multiplexer {
            session: Session::new(transport),
            initiator: initiator,
            next_id: 0,
            stream_senders: Default::default(),
            busy_streams: Default::default(),
            out_sender,
            out_receiver,
            outstanding_msg: None,
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
        self.stream_senders.insert(id, in_sender);
        MultiplexStream::new(id, flag, in_receiver, self.out_sender.clone())
            .map_err(Error::from)
    }

    pub fn close(self) -> impl Future<Item=(), Error=Error> {
        let mut session = self.session;
        future::poll_fn(move || session.close().map_err(Error::from))
    }
}

impl<S> Future for Multiplexer<S> where S: Sink<SinkItem=Vec<u8>, SinkError=io::Error> + Stream<Item=Vec<u8>, Error=io::Error> {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            // Check if we have an incoming message from our peer to handle
            match self.session.poll()? {
                Async::Ready(Some(msg)) => {
                    let stream_id = msg.stream_id;
                    if let Entry::Occupied(mut entry) = self.stream_senders.entry(stream_id) {
                        match entry.get_mut().start_send(msg) {
                            Ok(AsyncSink::Ready) => {
                                self.busy_streams.push(stream_id);
                            }
                            Ok(AsyncSink::NotReady(msg)) => {
                                println!("Dropping incoming message {:?} as stream is busy", msg);
                            }
                            Err(_) => {
                                // The stream was closed
                                entry.remove();
                            }
                        }
                    } else {
                        println!("Dropping incoming message {:?} as stream is closed", msg);
                    }
                    // We need to loop back to give self.session a chance to park itself
                    continue;
                }
                Async::Ready(None) => {
                    // The transport was closed, by dropping all the stream senders the streams
                    // will close
                    self.stream_senders.drain();
                    return Ok(Async::Ready(()));
                }
                Async::NotReady => (),
            }

            // For each stream that is dealing with an incoming message we need to check if they're
            // finished
            let stream_senders = &mut self.stream_senders;
            self.busy_streams.retain(|&stream_id| {
                if let Some(ref mut stream_sender) = stream_senders.get_mut(&stream_id) {
                    if let Ok(Async::NotReady) = stream_sender.poll_complete() {
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            });

            if let Some(msg) = self.outstanding_msg.take() {
                match self.session.start_send(msg)? {
                    AsyncSink::Ready => (),
                    AsyncSink::NotReady(msg) => {
                        self.outstanding_msg = Some(msg);
                    }
                }
            } else {
                match self.out_receiver.poll()? {
                    Async::Ready(Some(msg)) => {
                        self.outstanding_msg = Some(msg);
                        continue;
                    }
                    Async::Ready(None) => unreachable!(),
                    Async::NotReady => (),
                }
            }

            match self.session.poll_complete()? {
                Async::Ready(()) => (),
                Async::NotReady => (),
            }

            return Ok(Async::NotReady);
        }
    }
}

impl From<stream::Error> for Error {
    fn from(err: stream::Error) -> Error { Error::Send(err) }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error { Error::IO(err) }
}

impl From<()> for Error {
    fn from(_err: ()) -> Error { unreachable!() }
}
