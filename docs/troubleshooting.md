# Troubleshooting & FAQ

## 1. "CreateNamedPipe failed — check DLL imports are enabled"

- **Cause**: MT5 blocked `kernel32.dll` access.
- **Fix**: Open MT5 → **Tools** → **Options** → **Expert Advisors** → check **"Allow DLL imports"**. Recompile and reattach the EA.

## 2. "OrderSend failed (retcode 10027)"

- **Cause**: Algorithmic trading is disabled.
- **Fix**: Click the **"Algo Trading"** button in the main MT5 toolbar (it should turn green), and ensure "Allow algorithmic trading" is checked in the EA inputs.

## 3. "Failed to load MT5 bridge DLL"

- **Cause**: `mt5_bridge.dll` was not found in the search path or current working directory.
- **Fix**: Set the `MT5_DLL_PATH` environment variable:

  ```bash
  export MT5_DLL_PATH="/absolute/path/to/mt5_bridge.dll"
  ```

## 4. "Retcode 10014: Invalid volume in the request"

- **Cause**: The lot volume requested does not conform to the broker's minimum or lot step size.
- **Fix**: Use `info.round_lot(requested_volume)` to automatically normalize volume.

## 5. "Retcode 10016: Invalid stops (SL/TP)"

- **Cause**: Stop loss or take profit is placed too close to the current price (within broker freeze levels or stops level).
- **Fix**: Check `SymbolInfoDouble(sym, SYMBOL_TRADE_STOPS_LEVEL)` and place stops outside that distance using `info.round_price()`.

## 6. "Historical data is truncated or `copy_rates` returns fewer bars than requested"

- **Cause**: MetaTrader 5 limits the maximum number of bars saved and cached per chart by default (often 100,000 or fewer).
- **Fix**: Open MT5 → **Tools** → **Options** → **Charts** tab. Set **"Max bars in chart"** to **"Unlimited"** (or at least `1000000` / `1_000_000`), click **OK**, and restart the MT5 terminal.

## 7. "Mt5Error::AlreadyConnected"

- **Cause**: A second `Mt5Client` was created in the same process while one is already connected.
- **Fix**: The DLL enforces one client per process. Drop the existing client or call `shutdown()` before connecting a new one. For multiple MT5 terminals, run one process per terminal with a different `MT5_PIPE_NAME`.

## 8. "mt5_bridge_py: DLL not found" (Python)

- **Cause**: `mt5_bridge.dll` is not on the PATH or adjacent to the Python process.
- **Fix**: Place `mt5_bridge.dll` next to your Python script or set `MT5_DLL_PATH`. Under Wine, ensure the DLL path is accessible from the Wine prefix.

## 9. Python async tests fail when run in the same process as sync tests

- **Cause**: The DLL allows only one client per process.
- **Fix**: Run sync and async Python test suites in separate Wine Python processes:

  ```bash
  # Sync tests (process 1)
  MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_all_features.py

  # Async tests (process 2)
  MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_async_only.py
  ```

## 10. Python `order_send` rejected: "a stop loss is required for every order"

- **Cause**: Bridge safety policy requires `sl > 0` on every order submission.
- **Fix**: Always pass a non-zero `sl` (stop loss) when calling `order_send`. Orders without SL are intentionally rejected.

## 11. Wheel build fails with `maturin build`

- **Cause**: `maturin.toml` is not a valid maturin config; maturin reads `pyproject.toml`.
- **Fix**: Use `crates/mt5-bridge-py/pyproject.toml` (with `[tool.maturin] module-name = "mt5_bridge_py"`), not `maturin.toml`.

## 12. MSRV check fails on `clap_lex` edition2024

- **Cause**: `--all-features` enables `cli` → `clap` → `clap_lex 1.1.2`, which requires edition2024 unavailable in Cargo 1.81.
- **Fix**: `clap` is pinned to 4.4.18 (clap_lex 0.6.0) in `Cargo.lock` for MSRV compatibility. CI uses `cargo +1.81.0 check --all-features`.

## 13. `ctest` finds 0 C++ tests

- **Cause**: `bridge_dll/tests/CMakeLists.txt` is missing `enable_testing()`.
- **Fix**: The test binaries pass when run directly (`test_frame`, `test_poll_event`, `test_event_queue`, `test_connection`). Add `enable_testing()` to restore `ctest` discovery.

## 14. Fuzz targets not discoverable

- **Cause**: `fuzz/fuzz_targetors/` (typo) lacks `[[bin]]` entries and `[package.metadata] cargo-fuzz = true`.
- **Fix**: Rename to `fuzz/fuzz_targets/`, add `[[bin]]` entries, add cargo-fuzz metadata, and run `cargo fuzz` from the repo root (not `fuzz/`).

---

## Related Documents

- [Getting Started](getting-started.md) — installation prerequisites and setup
- [Architecture](architecture.md) — security model, IPC, state machine
- [API Reference](api-reference.md) — error variants and method signatures
