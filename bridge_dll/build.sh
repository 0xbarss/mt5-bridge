#!/usr/bin/env bash
# Build mt5_bridge.dll from Linux using MinGW-w64.
# Install: sudo pacman -S mingw-w64-gcc    (Arch)
#          sudo apt install gcc-mingw-w64  (Debian/Ubuntu)
set -e

CXX=x86_64-w64-mingw32-g++

if ! command -v "$CXX" &>/dev/null; then
    echo "Error: $CXX not found. Install with: sudo pacman -S mingw-w64-gcc"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$SCRIPT_DIR/build"

"$CXX" -O2 -shared \
    -DMT5_BRIDGE_EXPORTS \
    -std=c++17 \
    -o "$SCRIPT_DIR/build/mt5_bridge.dll" \
    "$SCRIPT_DIR/mt5_bridge.cpp" \
    -lkernel32 \
    -s \
    -fno-exceptions \
    -static

echo "Built: build/mt5_bridge.dll"
