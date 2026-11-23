# Getting Started

`mt5-bridge` is a fast, lightweight, unofficial native API bridge and client for **MetaTrader 5**, enabling high-performance algorithmic trading and data extraction in **Rust** (and any language supporting C ABI or Python).

It connects a client process to MetaTrader 5 through a local Windows Named Pipe:

1. An **MQL5 Expert Advisor** (`mt5_bridge.mq5`) runs inside MT5 as a named-pipe server.
2. A **C++ DLL** (`mt5_bridge.dll`) connects to the pipe and exports clean C ABI functions.
3. A **safe, idiomatic Rust crate** (`mt5-bridge`) provides typed models, async Tokio streams, retry stabilization, and full trade management.
4. Optional **Python bindings** (`mt5-bridge-py`) expose a sync `Client` and async `AsyncClient` to Python.

---

## Prerequisites

1. **MetaTrader 5 Terminal** (installed on Windows, or running via Wine on Linux).
2. **Rust toolchain** 1.81 or newer:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **C++ compiler** (to build the C FFI client DLL):
   - Linux: `mingw-w64-gcc` / `g++-mingw-w64` (`x86_64-w64-mingw32-g++`)
   - Windows: Visual Studio (MSVC) or MinGW-w64

---

## Step 1: Install the Expert Advisor in MT5

1. Open MetaTrader 5.
2. Click **File → Open Data Folder**.
3. Navigate to `MQL5/Experts/` inside the opened folder.
4. Copy [`mql5/Experts/mt5_bridge.mq5`](mql5/Experts/mt5_bridge.mq5) into that folder.
5. In MT5, press **F4** to open **MetaEditor** (or double-click `mt5_bridge.mq5`).
6. Press **F7** (Compile). Ensure `0 errors, 0 warnings`. This produces `mt5_bridge.ex5`.
7. In the main MT5 terminal:
   - **Tools → Options → Expert Advisors**: check **"Allow algorithmic trading"** and **"Allow DLL imports"** (essential — the EA uses `kernel32.dll` to create named pipes).
   - **Charts tab**: set **"Max bars in chart"** to **"Unlimited"** (or `1000000`) so deep OHLCV queries are not truncated.
   - Click **OK**.
8. Ensure the **"Algo Trading"** button in the top toolbar is **Green** (enabled).
9. From the **Navigator** (Ctrl+N), expand **Expert Advisors**, find `mt5_bridge`, and drag it onto any active chart (e.g. EURUSD).
10. Check the **Experts** tab. You should see:

    ```text
    MT5Bridge: pipe server ready — waiting for Rust client
    ```

### Expert Advisor Input Parameters

| Parameter | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `InpMagicNumber` | `int` | `20240101` | Magic number assigned to bridge trades |
| `InpPipeName` | `string` | `"mt5bridge"` | Named pipe name (override for multi-terminal setups) |
| `InpPipeSecret` | `string` | `""` | Shared secret token for client authentication |
| `InpRequireSecret` | `bool` | `true` | Require non-empty secret before allowing init (set `false` to opt out) |
| `InpEnforceMagicNumber` | `bool` | `true` | Restrict send/modify/close to `InpMagicNumber` |
| `InpPipeSDDL` | `string` | `"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"` | SDDL restricting pipe access to owner, SYSTEM, admins |
| `InpConvertToUTC` | `bool` | `true` | Convert broker history/tick timestamps to UTC |
| `InpTimerIntervalMs` | `int` | `1` | EA timer / pipe-drain loop frequency (ms) |
| `InpMaxRequestsPerTimer` | `int` | `64` | Max requests serviced per timer tick |
| `InpMaxTimerBudgetUs` | `uint` | `5000` | Max execution budget per timer tick (µs) |
| `InpAutoSelectSymbols` | `bool` | `true` | Auto-select queried symbols into Market Watch |

---

## Step 2: Build and Deploy `mt5_bridge.dll`

The Rust client dynamically loads `mt5_bridge.dll`. Compile it from source:

### Linux (cross-compile via MinGW)

```bash
cd bridge_dll
chmod +x build.sh
./build.sh
```

This generates `mt5_bridge.dll` in `bridge_dll/build/` and synchronizes it to the project root.

### Windows (MSVC)

Open **x64 Native Tools Command Prompt for VS**:

```cmd
cd bridge_dll
cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:..\mt5_bridge.dll
```

Place `mt5_bridge.dll` adjacent to your compiled binary, in your working directory, or set its exact location:

```bash
export MT5_DLL_PATH="/absolute/path/to/mt5_bridge.dll"
```

Optionally set the required SHA-256 hash for release builds:

```bash
export MT5_DLL_SHA256="<hash-of-dll>"
export MT5_DLL_REQUIRE_HASH=true   # defaults to true in release
```

---

## Step 3: Running on Linux via Wine

MetaTrader 5 runs under Wine on Linux:

1. Install and run MT5 inside your Wine prefix (`wine terminal64.exe`).
2. Attach `mt5_bridge.mq5` to a chart inside MT5.
3. Wine implements Windows Named Pipes through Unix domain sockets (`\\.\pipe\mt5bridge`).
4. To run Rust examples against Wine MT5:

   ```bash
   rustup target add x86_64-pc-windows-gnu
   cargo build --target x86_64-pc-windows-gnu --example 01_account_info
   wine target/x86_64-pc-windows-gnu/debug/examples/01_account_info.exe
   ```

For Python, run `mt5_bridge_py` under Wine Python 3.14 with `MT5_DLL_REQUIRE_HASH=true`.

---

## Step 4: Add the Rust Crate to Your Project

```bash
cargo add mt5-bridge
```

Or in `Cargo.toml`:

```toml
[dependencies]
mt5-bridge = "0.4"
tokio = { version = "1.0", features = ["full"] }
```

Enable features as needed:

```toml
mt5-bridge = { version = "0.4", features = ["async", "serde_json", "wal", "cli", "schema"] }
```

| Feature | Default? | Description |
| :--- | :--- | :--- |
| `async` | ✅ | Tokio runtime, async streams, `AsyncClient` (Python) |
| `serde_json` | ✅ | JSON serialization, `JsonFileStore` |
| `wal` | ✅ | Write-ahead log (`WalOrderStore`) |
| `cli` | ❌ | CLI binary (`mt5-bridge schema`, etc.) |
| `schema` | ❌ | JSON Schema export |
| `allow-unpinned-dll` | ❌ | Allow loading DLL without SHA-256 pin |
| `metrics` | ❌ | Prometheus metrics |

---

## Step 5: Connect (Rust example)

```rust,no_run
use mt5_bridge::Mt5Client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect using only the pipe secret token (recommended):
    let mut client = Mt5Client::connect_with_secret("my_pipe_secret")?;

    // Alternatively: login + password + server
    // let mut client = Mt5Client::connect(12345678, "password", "Broker-Demo")?;

    println!("Connected to MT5!");
    client.shutdown()?;
    Ok(())
}
```

> **One client per process.** The DLL keeps pipe connection and subscription state in process globals, so at most one `Mt5Client` may be live per process. A second `connect*` call returns `Mt5Error::AlreadyConnected`. Run one process per MT5 terminal (see [Multi-Instance](multi-instance.md)).

---

## Next Steps

- [Python API](python-api.md) — sync `Client`, async `AsyncClient`, event streaming
- [API Reference](api-reference.md) — full Rust `Mt5Client` reference
- [Architecture](architecture.md) — IPC, security, state machine, event model
- [Trading](trading.md) — order execution, positions, reconciliation, `OrderManager`
- [Streaming](streaming.md) — push ticks/bars/trade/book, backpressure
- [Benchmarks](benchmarks.md) — latency, throughput, fanout stats
- [Troubleshooting](troubleshooting.md) — common issues & fixes
