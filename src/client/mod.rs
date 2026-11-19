pub mod client_impl;
pub mod pipe;

#[cfg(test)]
mod tests;

pub use client_impl::{Mt5Client, DEFAULT_SYMBOL_CACHE_TTL};
pub use pipe::{validate_dll_path, validate_pipe_name};
