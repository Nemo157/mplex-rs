use std::io;
use std::convert::TryFrom;

use bytes::{BufMut, BytesMut};
use tokio_io::codec::{ Decoder, Encoder };
use varmint::{self, ReadVarInt, WriteVarInt};

use message::{Flag, Message};

#[derive(Debug, Clone, Copy)]
enum State {
    /// We have not read any part of the message, waiting on the header value.
    Ready,

    /// We have read the header value, waiting on the length value.
    Header {
        stream_id: u64,
        flag: Flag,
    },

    /// We have read the header and length values, waiting on the data to be
    /// fully available.
    // TODO: Could we just start streaming out data chunks as they appear and
    // decrementing `length` instead of waiting for it to be fully available?
    Length {
        stream_id: u64,
        flag: Flag,
        length: usize,
    },
}

pub(crate) struct Codec {
    state: Result<State, String>
}

impl Codec {
    pub(crate) fn new() -> Codec {
        Codec { state: Ok(State::Ready) }
    }
}

impl State {
    fn step(self, src: &mut BytesMut) -> Result<Option<(State, Option<Message>)>, String> {
        Ok(match self {
            State::Ready => {
                // TODO: Specified to be base128, but a 61 bit stream id space
                // should be enough for anyone, right?
                match src.as_ref().try_read_u64_varint().map_err(|e| e.to_string())? {
                    Some(header) => {
                        let header_len = varmint::len_u64_varint(header);
                        let _discarded = src.split_to(header_len);

                        let stream_id = header >> 3;
                        let flag = Flag::try_from(header & 0b00000111)?;
                        Some((State::Header { stream_id, flag }, None))
                    }
                    None => None,
                }
            }

            State::Header { stream_id, flag } => {
                // TODO: Specified to be base128, but since the max message size is
                // limited to much less than that reading a 64 bit/32 bit length should
                // be fine, right?
                src.as_ref()
                    .try_read_usize_varint()
                    .map_err(|e| e.to_string())?
                    .map(|length| {
                        let length_len = varmint::len_usize_varint(length);
                        let _discarded = src.split_to(length_len);
                        (State::Length { stream_id, flag, length }, None)
                    })
            }

            State::Length { stream_id, flag, length } => {
                if src.len() < length {
                    None
                } else {
                    let data = src.split_to(length).freeze();
                    Some((State::Ready, Some(Message { stream_id, data, flag })))
                }
            }
        })
    }
}

impl Decoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            let state = self.state.clone().map_err(|s| io::Error::new(io::ErrorKind::Other, s))?;
            match state.step(src) {
                Ok(None) => {
                    // There was not enough data available to transition to the
                    // next state, return and wait till we are next invoked.
                    return Ok(None);
                }
                Ok(Some((state, msg))) => {
                    // We transitioned to the next state, and may have finished
                    // parsing a message, if so return it, otherwise loop back
                    // and see if the next state can also transition onwards.
                    self.state = Ok(state);
                    if let Some(msg) = msg {
                        return Ok(Some(msg));
                    }
                }
                Err(error) => {
                    // We encountered an error while parsing a message, there
                    // is no form of re-synchronization in the protocol so we
                    // cannot recover from this.
                    self.state = Err(error);
                }
            }
        }
    }
}

impl Encoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let header = item.header();
        let len = item.data.len();
        let prefix_len = varmint::len_u64_varint(header)
            + varmint::len_usize_varint(len);
        dst.reserve(prefix_len + len);
        dst.writer().write_u64_varint(header).unwrap();
        dst.writer().write_usize_varint(len).unwrap();
        dst.put(item.data);
        Ok(())
    }
}
