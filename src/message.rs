use std::mem;
use std::io;
use varmint::{ len_u64_varint, len_usize_varint, ReadVarInt, WriteVarInt };

pub enum Flag {
    NewStream,
    Receiver,
    Initiator,
    Close,
}

pub struct Message {
    stream_id: u64,
    flag: Flag,
    data: Vec<u8>,
}

impl Message {
    pub fn try_from(mut bytes: Vec<u8>) -> io::Result<(Option<Message>, Vec<u8>)> {
        let (flag, stream_id, len, prefix_len) = {
            // TODO: Specified to be base128, but a 61 bit stream id space should
            // be enough for anyone, right?
            let header = {
                if let Some(header) = (&bytes[..]).try_read_u64_varint()? {
                    header
                } else {
                    return Ok((None, bytes));
                }
            };

            let flag = match header & 0b00000111 {
                0 => Flag::NewStream,
                1 => Flag::Receiver,
                2 => Flag::Initiator,
                4 => Flag::Close,
                3 | 5 | 6 | 7 => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown flag"))
                }
                _ => unreachable!(),
            };

            let stream_id = header >> 3;

            // TODO: Specified to be base128, but since the max message size is
            // limited to much less than that reading a 64 bit/32 bit length should
            // be fine, right?
            let len = {
                if let Some(len) = (&bytes[len_u64_varint(header)..]).try_read_usize_varint()? {
                    len
                } else {
                    return Ok((None, bytes));
                }
            };

            (flag, stream_id, len, len_u64_varint(header) + len_usize_varint(len))
        };

        if bytes.len() < prefix_len + len {
            return Ok((None, bytes));
        }

        let mut leftover = bytes.split_off(prefix_len + len);
        mem::swap(&mut leftover, &mut bytes);
        let data = bytes.split_off(prefix_len);

        Ok((Some(Message {
            stream_id: stream_id,
            data: data,
            flag: flag
        }), leftover))
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        let header = (self.stream_id << 3) & match self.flag {
            Flag::NewStream => 0,
            Flag::Receiver => 1,
            Flag::Initiator => 2,
            Flag::Close => 4,
        };
        let len = self.data.len();
        let prefix_len = len_u64_varint(header) + len_usize_varint(len);
        let mut bytes = Vec::with_capacity(prefix_len + len);
        bytes.write_u64_varint(header).unwrap();
        bytes.write_usize_varint(len).unwrap();
        bytes.append(&mut self.data);
        bytes
    }
}
