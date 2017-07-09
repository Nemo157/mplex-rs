use std::io;
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
    pub data: Vec<u8>,
}

impl Message {
    pub fn try_from(mut bytes: Vec<u8>) -> io::Result<Message> {
        let (flag, stream_id, len, prefix_len) = {
            // TODO: Specified to be base128, but a 61 bit stream id space should
            // be enough for anyone, right?
            let header = {
                if let Some(header) = (&bytes[..]).try_read_u64_varint()? {
                    header
                } else {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Underfull message: missing header"));
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
                if let Some(len) = (&bytes[len_u64_varint(header)..]).try_read_usize_varint()? {
                    len
                } else {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Underfull message: missing length"));
                }
            };

            (flag, stream_id, len, len_u64_varint(header) + len_usize_varint(len))
        };

        if bytes.len() < prefix_len + len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Underfull message: short data"));
        } else if bytes.len() > prefix_len + len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Overfull message"));
        }

        Ok(Message {
            stream_id: stream_id,
            data: bytes.split_off(prefix_len),
            flag: flag
        })
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
