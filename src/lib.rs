#![feature(conservative_impl_trait)]
#![feature(type_ascription)]
#![feature(try_from)]

extern crate bytes;
#[macro_use]
extern crate futures;
extern crate varmint;
extern crate tokio_io;

mod codec;
mod message;
mod multiplexer;
mod stream;

pub use multiplexer::Multiplexer;
pub use stream::MultiplexStream as Stream;
