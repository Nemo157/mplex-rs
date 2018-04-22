#![feature(type_ascription)]
#![feature(try_from)]
#![feature(generators)]
#![feature(proc_macro)]

extern crate bytes;
extern crate futures_await as futures;
extern crate varmint;
extern crate tokio_io;

mod codec;
mod message;
mod multiplexer;
mod stream;

pub use multiplexer::Multiplexer;
pub use stream::MultiplexStream as Stream;
