//! Shared helpers for the `mt5-bridge-fuzz` targets.
//!
//! The wire/WAL formats are binary-packed (`#[repr(C, packed)]`) so individual
//! fields can be unaligned. `read_packed` uses `ptr::read_unaligned`, which is
//! the correct primitive for deserialising packed structs from raw input bytes
//! without invoking undefined behaviour.

pub fn read_packed<T: Copy>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < core::mem::size_of::<T>() {
        return None;
    }
    // SAFETY: `read_unaligned` is defined for unaligned addresses and the length
    // check above guarantees `size_of::<T>()` bytes are available for reading.
    Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const T) })
}
