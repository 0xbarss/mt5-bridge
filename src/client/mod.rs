#[cfg(feature = "async")]
pub mod async_client;
#[cfg(feature = "async")]
pub use async_client::AsyncMt5Client;
pub mod client_impl;
pub mod dll_path;
pub mod pipe;

#[cfg(test)]
mod tests;

pub use client_impl::{Mt5Client, DEFAULT_SYMBOL_CACHE_TTL};
pub use dll_path::validate_dll_path;
pub use pipe::validate_pipe_name;
