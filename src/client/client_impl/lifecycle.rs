use crate::error::{Mt5Error, Result};

use super::client::Mt5Client;

impl Mt5Client {
    /// Gracefully shutdown the named pipe connection to MetaTrader 5.
    pub fn shutdown(&self) -> Result<()> {
        if self
            .is_shutdown
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Ok(());
        }
        super::client::CLIENT_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        #[cfg(feature = "async")]
        self.event_loop_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let ret = unsafe { (self.fn_shut)() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "Shutdown returned non-1 status code: {ret}"
            )));
        }
        Ok(())
    }
}

impl Drop for Mt5Client {
    fn drop(&mut self) {
        if self
            .is_shutdown
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            super::client::CLIENT_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            #[cfg(feature = "async")]
            self.event_loop_running
                .store(false, std::sync::atomic::Ordering::SeqCst);
            // Shutdown is intentionally idempotent in the DLL (returns 1 if already disconnected).
            // This sets g_running to false and signals g_event_cv in the DLL, waking up poll_fn.
            unsafe {
                let _ = (self.fn_shut)();
            }
        }

        #[cfg(feature = "async")]
        if let Some(handle) = self.event_pump_handle.take() {
            let _ = handle.join();
        }
    }
}
