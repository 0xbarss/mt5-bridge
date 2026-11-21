# Security Model & Threat Assumptions — `mt5-bridge`

This document defines the security architecture, trust boundaries, threat assumptions, and security non-goals for `mt5-bridge`.

---

## 1. Trust Boundaries & Assumptions

`mt5-bridge` operates across an Inter-Process Communication (IPC) boundary between a client process (the trading algorithm) and the MetaTrader 5 terminal process running the `mt5_bridge.mq5` Expert Advisor.

### Threat Model
1. **Local IPC Boundary:**
   - Communication occurs via Windows Named Pipes (`\\.\pipe\<name>`).
   - The pipe endpoint is strictly local to the machine running the MT5 terminal.
   - Cross-network transport is not supported directly; remote access must be tunneled via SSH, VPN, or mTLS proxies.
2. **Pipe Access Control (SDDL ACL):**
   - The EA creates the named pipe with an explicit Win32 Security Descriptor:
     `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)`
   - This restricts pipe access exclusively to:
     - `SY` (Local System)
     - `BA` (Builtin Administrators)
     - `OW` (Creator Owner)
   - Other unprivileged local user accounts or cross-session processes on multi-user systems cannot connect to or read from the pipe.
3. **Application Authentication Token:**
   - The EA requires a secret handshake token (`InpPipeSecret`).
   - `InpRequireSecret` is enabled by default (`true`). The EA refuses connection initialization if a secret is missing or mismatched.
4. **Account & Server Binding:**
   - During `CMD_INIT`, the client supplies the expected `login` and `server`.
   - The EA verifies these against `ACCOUNT_LOGIN` and `ACCOUNT_SERVER` of the active terminal session, preventing accidental trading against the wrong account.
5. **Memory & Bounds Safety:**
   - All packet deserialization in C++ and MQL5 utilizes bounds-checked unpack routines (`SafeUnpack*`).
   - Over-sized or truncated payloads are rejected cleanly without buffer overruns or crashes.
6. **DLL Hijacking Defenses:**
   - The client refuses relative DLL paths (`MT5_DLL_PATH` must be absolute).
   - On Unix/Linux systems (Wine), DLLs located in world-writable directories are rejected.
   - Optional SHA-256 hash pinning (`MT5_DLL_SHA256`) can be enforced via `MT5_DLL_REQUIRE_HASH=1`.
7. **Preflight Validation & Defense in Depth:**
   - Both client and EA enforce strict preflight safety checks:
     - Symbol trade mode must not be disabled (`SYMBOL_TRADE_MODE_DISABLED`).
     - Lot sizes must satisfy min, max, and step boundaries (`SYMBOL_VOLUME_STEP`).
     - Stop Loss is required by default on new order submissions to prevent unbounded downside.

---

## 2. Explicit Security Non-Goals

1. **Not a Remote Network Protocol:**
   - The binary IPC protocol does not include encryption (TLS) or network authentication.
   - The named pipe must **never** be exposed directly over an unencrypted, untrusted network.
2. **Static Secret is Not a Key Exchange:**
   - `InpPipeSecret` is a shared authentication token designed to prevent unauthorized local processes from interacting with the EA. It does not provide cryptographic confidentiality or forward secrecy.
3. **Broker Credential Storage:**
   - `mt5-bridge` does not store or manage broker account master/investor passwords. Account login and authentication with the broker are handled exclusively by the MetaTrader 5 terminal application.
4. **Single-Terminal Multi-Tenancy:**
   - The bridge does not support multi-tenant isolation within a single MT5 terminal instance. One terminal instance corresponds to one broker account and one active bridge connection.
