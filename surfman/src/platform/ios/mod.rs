// surfman/surfman/src/platform/ios/mod.rs
//
//! iOS bindings

pub mod connection;
pub mod context;
pub mod device;
pub mod surface;

mod ffi;

#[path = "../../implementation/mod.rs"]
mod implementation;

#[cfg(test)]
#[path = "../../tests.rs"]
mod tests;
