# Multi-Instance / Multi-Account Support

To connect to multiple MT5 terminals simultaneously on the same machine:

1. In MT5 Terminal #1:
   - When attaching `mt5_bridge` to the chart, set input parameter `InpPipeName = "mt5bridge_account1"`.
2. In MT5 Terminal #2:
   - Attach `mt5_bridge` with `InpPipeName = "mt5bridge_account2"`.
3. In Rust, run a separate process per terminal and direct each client to the corresponding pipe via the `MT5_PIPE_NAME` environment variable:

   ```bash
   MT5_PIPE_NAME=mt5bridge_account1 ./my_bot_account1 &
   MT5_PIPE_NAME=mt5bridge_account2 ./my_bot_account2 &
   ```

> **One client per process.** `mt5-bridge` enforces single-connection-per-process semantics via a process-level atomic guard (`CLIENT_ACTIVE`): a second client in the same process fails with `Mt5Error::AlreadyConnected`. This is required because the DLL stores its pipe connection and subscription state in process globals. To interact with multiple terminals simultaneously, run one dedicated process per MT5 terminal.
>
> `MT5_PIPE_NAME` is read at connect time (and by the DLL during initialization), not on every request. Set it before calling `connect*`; changing it after a successful connect does not move the live connection. Dropping the client or calling `shutdown()` releases the slot so a new client can connect later in the same process. `Mt5Client::process_connection_count()` returns `0` or `1` and is intended for diagnostics and health endpoints.

```rust,no_run
use mt5_bridge::Mt5Client;

# fn main() -> Result<(), mt5_bridge::Mt5Error> {
assert_eq!(Mt5Client::process_connection_count(), 0);

let client = Mt5Client::connect_with_secret("my_pipe_secret")?;
assert_eq!(Mt5Client::process_connection_count(), 1);

drop(client); // Releases the process-wide slot.
assert_eq!(Mt5Client::process_connection_count(), 0);
# Ok(())
# }
```
