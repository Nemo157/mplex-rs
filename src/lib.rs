#![feature(field_init_shorthand)]
#![feature(pub_restricted)]
#![feature(conservative_impl_trait)]

#[macro_use]
extern crate futures;
extern crate futures_mpsc;
extern crate varmint;

mod message;
mod session;
mod stream;
mod multiplexer;

pub use multiplexer::Multiplexer;
