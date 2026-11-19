//! Synchronization utilities and lock recovery helpers.

use std::sync::{Mutex, MutexGuard};

/// Extension trait for [`Mutex`] providing panic-resilient lock recovery.
///
/// When a thread holding a [`Mutex`] panics, standard Rust poisons the lock.
/// In a trading bridge, a poisoned lock often still contains recoverable state
/// that should be inspected or safely accessed rather than cascading the panic.
pub trait LockExt<T> {
    /// Acquires the lock, recovering from poisoning if another thread panicked.
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    #[inline]
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_lock_recover_from_poisoned_mutex() {
        let lock = Arc::new(Mutex::new(42));
        let lock_clone = Arc::clone(&lock);

        let _ = thread::spawn(move || {
            let _guard = lock_clone.lock().unwrap();
            panic!("deliberate panic to poison lock");
        })
        .join();

        assert!(lock.is_poisoned());
        let guard = lock.lock_recover();
        assert_eq!(*guard, 42);
    }
}
