cmake_minimum_required(VERSION 3.16)
project(mt5_bridge_cpp_tests LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)

# ── Sanitizer + warning flags (shared across all test targets) ───────────────
if(MSVC)
    set(TEST_COMPILE_OPTIONS /O2 /W4 /WX /std:c++17)
else()
    set(TEST_COMPILE_OPTIONS -O2 -Wall -Wextra -Werror -std=c++17)
endif()

# Each test .cpp #includes ../mt5_bridge.cpp directly, so only the test file
# itself is compiled as a translation unit (compiling mt5_bridge.cpp separately
# would duplicate every exported symbol).

# ── Test: PacketHdr framing + Packer payload serialization ───────────────────
add_executable(test_frame test_frame.cpp)
target_compile_definitions(test_frame PRIVATE MT5_BRIDGE_EXPORTS)
target_compile_options(test_frame PRIVATE ${TEST_COMPILE_OPTIONS})
add_test(NAME test_frame COMMAND test_frame)

# ── Test: PollEvent buffer-too-small / whole-event delivery ─────
add_executable(test_poll_event test_poll_event.cpp)
target_compile_definitions(test_poll_event PRIVATE MT5_BRIDGE_EXPORTS)
target_compile_options(test_poll_event PRIVATE ${TEST_COMPILE_OPTIONS})
add_test(NAME test_poll_event COMMAND test_poll_event)

# ── Test: event ring-buffer overflow and per-type drop counters ──────────────
add_executable(test_event_queue test_event_queue.cpp)
target_compile_definitions(test_event_queue PRIVATE MT5_BRIDGE_EXPORTS)
target_compile_options(test_event_queue PRIVATE ${TEST_COMPILE_OPTIONS})
add_test(NAME test_event_queue COMMAND test_event_queue)

# ── Test: socketpair-backed request/response, multiplexing, orphans (Linux) ──
if(NOT WIN32)
    add_executable(test_connection test_connection.cpp)
    target_compile_definitions(test_connection PRIVATE MT5_BRIDGE_EXPORTS)
    target_compile_options(test_connection PRIVATE ${TEST_COMPILE_OPTIONS})
    add_test(NAME test_connection COMMAND test_connection)
endif()

# ── Run the whole C++ test suite with ctest ──────────────────────────────────
#   cmake -S bridge_dll/tests -B bridge_dll/tests/build
#   cmake --build bridge_dll/tests/build
#   ctest --test-dir bridge_dll/tests/build --output-on-failure
