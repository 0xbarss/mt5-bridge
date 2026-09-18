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

mkdir -p build

"$CXX" -O2 -shared \
    -DMT5_BRIDGE_EXPORTS \
    -std=c++17 \
    -o build/mt5_bridge.dll \
    mt5_bridge.cpp \
    -lkernel32 \
    -s \
    -fno-exceptions \
    -static

echo "Built: build/mt5_bridge.dll"
echo "Deploy to: C:\\Program Files\\MetaTrader 5\\MQL5\\Libraries\\mt5_bridge.dll"
