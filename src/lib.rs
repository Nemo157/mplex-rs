#![feature(conservative_impl_trait)]

extern crate bytes;
#[macro_use]
extern crate futures;
extern crate varmint;
extern crate msgio;
extern crate tokio_io;

mod message;
mod session;
mod stream;
mod multiplexer;

pub use multiplexer::Multiplexer;
pub use stream::MultiplexStream as Stream;
