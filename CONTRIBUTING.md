# Contributing to mt5-bridge

Thank you for your interest in contributing to **mt5-bridge**! Whether you are reporting a bug, improving the documentation, submitting a pull request, or adding language bindings, your help is greatly appreciated.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How Can I Contribute?](#how-can-i-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Features](#suggesting-features)
  - [Submitting Pull Requests](#submitting-pull-requests)
- [Development Setup](#development-setup)
  - [Prerequisites](#prerequisites)
  - [Cloning & Building](#cloning--building)
  - [Testing](#testing)
  - [Building the C++ DLL](#building-the-c-dll)
- [Commit Message Conventions](#commit-message-conventions)
- [Code Style Guidelines](#code-style-guidelines)
  - [Rust](#rust)
  - [C++ / DLL](#c--dll)
  - [MQL5](#mql5)
- [Pull Request Process](#pull-request-process)

---

## Code of Conduct

We aim to foster an open, welcoming, and respectful community. Please be considerate and constructive in all issues, pull requests, and discussions.

---

## How Can I Contribute?

### Reporting Bugs

If you find a bug or unexpected behavior:
1. Search existing [GitHub Issues](https://github.com/0xbarss/mt5-bridge/issues) to verify it hasn't already been reported.
2. If not found, open a new issue with:
   - A clear, descriptive title.
   - Steps to reproduce the issue.
   - Expected vs. actual behavior.
   - Your environment details: OS version, MT5 build/version, Rust version, broker name (if relevant).
   - Any relevant logs from the MT5 Experts tab or Rust tracing output.

### Suggesting Features

Feature requests are welcome! When opening an issue for a feature idea, please describe:
- The problem you are trying to solve or the motivation behind the feature.
- Proposed solution or API design.
- Any trade-offs, edge cases, or backwards-compatibility considerations.

### Submitting Pull Requests

1. Fork the repository on GitHub.
2. Create a feature or bugfix branch off `main`.
3. Implement your changes with corresponding tests and documentation updates.
4. Ensure all tests pass (`cargo test && cargo check --examples`).
5. Commit your changes using our [Commit Message Conventions](#commit-message-conventions).
6. Push to your fork and open a Pull Request against `main`.

---

## Development Setup

### Prerequisites

- **Rust**: 1.75 or newer (via `rustup`).
- **MetaTrader 5**: Windows desktop terminal or running under Wine on Linux.
- **C++ Cross-Compiler** (only if modifying `bridge_dll/`):
  - On Linux: `mingw-w64-gcc` (`sudo apt install gcc-mingw-w64 g++-mingw-w64` or `sudo pacman -S mingw-w64-gcc`).
  - On Windows: Visual Studio 2019/2022 (MSVC) or MinGW-w64.

### Cloning & Building

```bash
git clone git@github.com:0xbarss/mt5-bridge.git
cd mt5-bridge

# Build the Rust crate
cargo build

# Check all example programs
cargo check --examples
```

### Testing

Run the test suite:

```bash
cargo test
```

This verifies:
- C ABI struct memory layouts (`Mt5SymInfo`, `Mt5Rate`, `Mt5Tick`, `Mt5TradeResult`).
- Timeframe mappings, conversions, and serialization.
- Mathematical helpers (lot rounding, typical price, profit calculation).

### Building the C++ DLL

If you make modifications to [`bridge_dll/mt5_bridge.cpp`](bridge_dll/mt5_bridge.cpp) or [`bridge_dll/mt5_bridge.h`](bridge_dll/mt5_bridge.h):

```bash
cd bridge_dll

# Cross-compile from Linux (outputs to build/mt5_bridge.dll and project root)
./build.sh
```

On Windows with MSVC:
```cmd
cd bridge_dll
cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:..\mt5_bridge.dll
```

---

## Commit Message Conventions

This repository strictly follows the **Conventional Commits** specification. Please ensure all commit messages adhere to this standard.

### Format

```text
<type>: <lowercase description in imperative mood>
```

### Allowed Types

| Type | Description |
| :--- | :--- |
| `feat:` | A new feature or capability |
| `fix:` | A bug fix |
| `docs:` | Documentation changes or additions |
| `test:` | Adding or updating tests and examples |
| `refactor:` | Code changes that neither fix a bug nor add a feature |
| `perf:` | Performance improvements |
| `chore:` | Maintenance, build scripts, dependency updates |

### Examples

```text
feat: add pending order cancellation helper
fix: handle partial fill retcode in TradeResult and add buffer limits to CopyRates
docs: add comprehensive contribution guide and update setup instructions
test: add unit tests for stop loss calculation
refactor: simplify pipe reconnect polling logic
```

- Use lowercase for the prefix and description.
- Use the imperative mood ("add", "fix", "implement", not "added" or "fixing").
- Do not end the commit title with a period.

---

## Code Style Guidelines

### Rust

- Run `cargo fmt` before submitting your changes.
- Ensure `cargo clippy` runs without warnings:
  ```bash
  cargo clippy --all-targets --all-features
  ```
- **Error Handling**: Avoid `unwrap()` or `expect()` in library code (`src/`). Return `Result<T, Mt5Error>`.
- **ABI Safety**: Never modify the field layouts of structs in `src/ffi.rs` without updating `bridge_dll/mt5_bridge.h` and `mql5/Experts/mt5_bridge.mq5` correspondingly.

### C++ / DLL

- Adhere to C++17.
- Preserve `#pragma pack(push, 1)` and 1-byte alignment across all wire structs.
- Statically link runtime libraries (`-static-libgcc -static-libstdc++`) so the generated DLL avoids external dependencies on MinGW GCC runtime DLLs (`libgcc_s_seh-1.dll`, `libstdc++-6.dll`, `libwinpthread-1.dll`), relying only on standard Windows system libraries (`KERNEL32.dll` and UCRT/`msvcrt.dll`).

### MQL5

- Keep code compatible with `#property strict`.
- Ensure named-pipe operations remain non-blocking in `OnTimer` to avoid freezing MT5 terminal chart threads.
- Test in the MT5 MetaEditor to verify `0 errors, 0 warnings`.

---

## Pull Request Process

1. **Keep PRs Focused**: A pull request should ideally address one feature or one bug. Avoid bundling unrelated changes.
2. **Update Documentation**: If your PR modifies public APIs, CLI parameters, or setup instructions, update [`README.md`](README.md) and related docstrings accordingly.
3. **Maintain Test Coverage**: Add unit tests or integration examples for new functionality.
4. **CI Verification**: Make sure all checks pass before requesting review.

Thank you for contributing to **mt5-bridge**!
