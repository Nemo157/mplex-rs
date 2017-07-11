use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io;

use futures::{ self, Future, Sink, Stream, Poll, Async, AsyncSink };
use futures::unsync::mpsc;
use msgio::MsgIo;

use stream::MultiplexStream;
use message::{ Message, Flag };
use session::Session;

pub struct Multiplexer<S: MsgIo> {
    session_stream: futures::stream::SplitStream<Session<S>>,
    initiator: bool,
    next_id: u64,
    stream_senders: HashMap<u64, mpsc::Sender<Message>>,
    busy_streams: Vec<u64>,
    out_sender: mpsc::Sender<Message>,
    forward: futures::stream::Forward<futures::stream::MapErr<mpsc::Receiver<Message>, fn(()) -> io::Error>, futures::stream::SplitSink<Session<S>>>,
}

impl<S: MsgIo> Multiplexer<S> {
    pub fn new(transport: S, initiator: bool) -> Multiplexer<S> {
        fn unreachable(_: ()) -> io::Error { unreachable!() }
        let (out_sender, out_receiver) = mpsc::channel(16);
        let session = Session::new(transport);
        let (session_sink, session_stream) = session.split();
        let forward = out_receiver.map_err(unreachable as _).forward(session_sink);
        Multiplexer {
            next_id: 0,
            stream_senders: Default::default(),
            busy_streams: Default::default(),
            session_stream, initiator, out_sender, forward,
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 2;
        if self.initiator { id } else { id + 1 }
    }

    pub fn new_stream(&mut self) -> impl Future<Item=MultiplexStream, Error=io::Error> {
        let id = self.next_id();
        let (in_sender, in_receiver) = mpsc::channel(16);
        self.stream_senders.insert(id, in_sender);
        MultiplexStream::initiate(id, in_receiver, self.out_sender.clone())
    }

    pub fn close(self) -> impl Future<Item=(), Error=io::Error> {
        // Maybe? I think a wave of closure should propagate through from dropping everything else
        // in self.
        self.forward.map(|_| ())
    }
}


impl<S: MsgIo> Stream for Multiplexer<S> {
    type Item = MultiplexStream;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            // Check if we have an incoming message from our peer to handle
            match self.session_stream.poll()? {
                Async::Ready(Some(msg)) => {
                    let stream_id = msg.stream_id;
                    match self.stream_senders.entry(stream_id) {
                        Entry::Occupied(mut entry) => {
                            match entry.get_mut().start_send(msg) {
                                Ok(AsyncSink::Ready) => {
                                    self.busy_streams.push(stream_id);
                                }
                                Ok(AsyncSink::NotReady(msg)) => {
                                    println!("Dropping incoming message {:?} as stream is busy", msg);
                                }
                                Err(err) => {
                                    println!("Multiplexer stream {} was closed: {:?}", stream_id, err);
                                    entry.remove();
                                }
                            }
                        }
                        Entry::Vacant(entry) => {
                            if msg.flag == Flag::NewStream {
                                let (in_sender, in_receiver) = mpsc::channel(16);
                                entry.insert(in_sender);
                                return Ok(Async::Ready(Some(MultiplexStream::receive(msg.stream_id, in_receiver, self.out_sender.clone()))));
                            } else {
                                println!("Dropping incoming message {:?} as stream was not opened/is closed", msg);
                            }
                        }
                    }
                    // We need to loop back to give self.session a chance to park itself
                    continue;
                }
                Async::Ready(None) => {
                    // The transport was closed, by dropping all the stream senders the streams
                    // will close
                    self.stream_senders.drain();
                    return Ok(Async::Ready(None));
                }
                Async::NotReady => (),
            }

            // For each stream that is dealing with an incoming message we need to check if they're
            // finished
            let stream_senders = &mut self.stream_senders;
            self.busy_streams.retain(|&stream_id| {
                if let Some(ref mut stream_sender) = stream_senders.get_mut(&stream_id) {
                    if let Ok(Async::NotReady) = stream_sender.poll_complete() {
                        // Not finished yet, will have been parked and we need to check it again
                        // next loop.
                        true
                    } else {
                        // Either it finished and we no longer need to poll, or it has errored...,
                        // I think we're ok to ignore the error here and it will be handled
                        // elsewhere.
                        false
                    }
                } else {
                    // The stream has closed and been removed elsewhere
                    false
                }
            });

            // Let the forwarder propagate any message from the streams to the session
            match self.forward.poll()? {
                Async::Ready(_) => {
                    // Incoming stream was closed?
                    return Ok(Async::Ready(None));
                }
                Async::NotReady => {
                    return Ok(Async::NotReady);
                }
            }
        }
    }
}
