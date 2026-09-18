cmake_minimum_required(VERSION 3.16)
project(mt5_bridge LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)

# ── Cross-compile from Linux with MinGW-w64 ──────────────────────────────────
# Install: sudo pacman -S mingw-w64-gcc   (Arch)
#          sudo apt  install gcc-mingw-w64 (Debian/Ubuntu)
#
# Configure:
#   cmake -B build -DCMAKE_TOOLCHAIN_FILE=cmake/mingw64.cmake
# Build:
#   cmake --build build
# Output:
#   build/mt5_bridge.dll

add_library(mt5_bridge SHARED mt5_bridge.cpp mt5_bridge.h)
target_compile_definitions(mt5_bridge PRIVATE MT5_BRIDGE_EXPORTS)

if (MINGW)
    # Link against kernel32 statically.
    target_link_libraries(mt5_bridge PRIVATE kernel32)
    # Strip the DLL to reduce size.
    # Statically embed MinGW runtime to avoid shipping libstdc++-6.dll and
    # libwinpthread-1.dll alongside the DLL on the target Windows machine.
    target_link_options(mt5_bridge PRIVATE
        -s
        -static-libgcc
        -static-libstdc++
        -Wl,-Bstatic,-lwinpthread,-Bdynamic
    )
    target_compile_options(mt5_bridge PRIVATE -O2 -fno-exceptions)
endif()

# ── MinGW toolchain file (cmake/mingw64.cmake) ───────────────────────────────
# Create this file to cross-compile from Linux:
#
#   set(CMAKE_SYSTEM_NAME Windows)
#   set(CMAKE_C_COMPILER   x86_64-w64-mingw32-gcc)
#   set(CMAKE_CXX_COMPILER x86_64-w64-mingw32-g++)
#   set(CMAKE_RC_COMPILER  x86_64-w64-mingw32-windres)
#   set(CMAKE_FIND_ROOT_PATH /usr/x86_64-w64-mingw32)
#   set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
#   set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
#   set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

install(TARGETS mt5_bridge RUNTIME DESTINATION .)
