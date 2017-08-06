use std::io;
use std::convert::TryFrom;

use bytes::Bytes;

#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub(crate) enum Flag {
    NewStream,
    Receiver,
    Initiator,
    Close,
}

#[derive(Debug)]
pub(crate) struct Message {
    pub(crate) stream_id: u64,
    pub(crate) flag: Flag,
    pub(crate) data: Bytes,
}

impl Into<u64> for Flag {
    fn into(self) -> u64 {
        match self {
            Flag::NewStream => 0,
            Flag::Receiver => 1,
            Flag::Initiator => 2,
            Flag::Close => 4,
        }
    }
}

impl TryFrom<u64> for Flag {
    type Error = io::Error;
    fn try_from(val: u64) -> Result<Self, Self::Error> {
        Ok(match val {
            0 => Flag::NewStream,
            1 => Flag::Receiver,
            2 => Flag::Initiator,
            4 => Flag::Close,
            3 | 5 | 6 | 7 => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown flag"));
            }
            _ => {
                panic!("Flag should only be converted from a 3-bit value")
            }
        })
    }
}

impl Message {
    pub fn header(&self) -> u64 {
        (self.stream_id << 3) | self.flag.into(): u64
    }
}
