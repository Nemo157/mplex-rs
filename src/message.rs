use std::io;

use bytes::{BufMut, Bytes, BytesMut};
use tokio_io::codec::{ Decoder, Encoder };
use varmint::{ len_u64_varint, len_usize_varint, ReadVarInt, WriteVarInt };

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum Flag {
    NewStream,
    Receiver,
    Initiator,
    Close,
}

#[derive(Debug)]
pub struct Message {
    pub stream_id: u64,
    pub flag: Flag,
    pub data: Bytes,
}

#[derive(Debug)]
pub struct Codec;

impl Decoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let (flag, stream_id, len, prefix_len) = {
            // TODO: Specified to be base128, but a 61 bit stream id space should
            // be enough for anyone, right?
            let header = {
                if let Some(header) = src.as_ref().try_read_u64_varint()? {
                    header
                } else {
                    return Ok(None);
                }
            };

            let flag = match header & 0b00000111 {
                0 => Flag::NewStream,
                1 => Flag::Receiver,
                2 => Flag::Initiator,
                4 => Flag::Close,
                3 | 5 | 6 | 7 => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown flag"));
                }
                _ => unreachable!(),
            };

            let stream_id = header >> 3;

            // TODO: Specified to be base128, but since the max message size is
            // limited to much less than that reading a 64 bit/32 bit length should
            // be fine, right?
            let len = {
                if let Some(len) = (&src[len_u64_varint(header)..]).try_read_usize_varint()? {
                    len
                } else {
                    return Ok(None);
                }
            };

            (flag, stream_id, len, len_u64_varint(header) + len_usize_varint(len))
        };

        if src.len() < prefix_len + len {
            return Ok(None);
        }

        let _discarded = src.split_to(prefix_len);
        let data = src.split_to(len).freeze();

        Ok(Some(Message { stream_id, data, flag }))
    }
}

impl Encoder for Codec {
    type Item = Message;
    type Error = io::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let header = (item.stream_id << 3) & match item.flag {
            Flag::NewStream => 0,
            Flag::Receiver => 1,
            Flag::Initiator => 2,
            Flag::Close => 4,
        };
        let len = item.data.len();
        let prefix_len = len_u64_varint(header) + len_usize_varint(len);
        dst.reserve(prefix_len + len);
        dst.writer().write_u64_varint(header).unwrap();
        dst.writer().write_usize_varint(len).unwrap();
        dst.put(item.data);
        Ok(())
    }
}
