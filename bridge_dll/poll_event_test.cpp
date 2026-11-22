// Standalone harness for PollEvent buffer-overflow handling (audit finding A-13).
//
// It #includes mt5_bridge.cpp directly so the test can reach the file-static
// event queue (g_event_queue / g_event_mutex) and drive PollEvent through the
// same code path the Rust event pump uses.
//
// Compile (Linux, g++):
//   g++ -std=c++17 -O2 -Wall -Wextra -o /tmp/poll_event_test poll_event_test.cpp
// Run:
//   /tmp/poll_event_test
//
// Verifies:
//   * a 176-byte Mt5TradeEvent is never truncated into a 4-byte buffer:
//     PollEvent returns MT5_ERR_BUFFER_TOO_SMALL, reports the required size
//     (176) in out_len, copies nothing, and leaves the event queued;
//   * a buffer large enough receives the event whole and it parses as a valid
//     struct;
//   * an empty queue with timeout 0 returns 0.

#include <cassert>
#include <cstring>
#include <iostream>
#include <string>

#include "mt5_bridge.cpp"

static void queue_event(uint16_t event_type, const void* payload, uint32_t len) {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    g_event_queue.push({event_type, std::string(static_cast<const char*>(payload), len)});
}

int main() {
    // 1. Queue a 176-byte trade event (the largest event type) and read it with
    //    a 4-byte buffer: must report "buffer too small" without truncation.
    Mt5TradeEvent ev{};
    ev.deal = 0xDEADBEEFULL;
    ev.order = 1;
    ev.position = 2;
    ev.event_seq = 42;
    queue_event(EVENT_TRADE, &ev, static_cast<uint32_t>(sizeof(ev)));

    uint16_t event_type = 0xFFFFu;
    uint8_t small[4] = {0xAA, 0xBB, 0xCC, 0xDD};
    uint32_t out_len = 0;

    int rc = PollEvent(&event_type, small, sizeof(small), &out_len, 0);
    assert(rc == MT5_ERR_BUFFER_TOO_SMALL);
    assert(out_len == sizeof(Mt5TradeEvent)); // required size reported
    assert(small[0] == 0xAA && small[1] == 0xBB && small[2] == 0xCC && small[3] == 0xDD); // nothing copied
    assert(event_type == 0xFFFFu); // out_event_type untouched on overflow

    // The event stays at the head of the queue: repeating the undersized read
    // reports the same required size instead of delivering garbage.
    out_len = 0;
    rc = PollEvent(&event_type, small, sizeof(small), &out_len, 0);
    assert(rc == MT5_ERR_BUFFER_TOO_SMALL);
    assert(out_len == sizeof(Mt5TradeEvent));

    // 2. A buffer large enough receives the event whole; it parses as a valid struct.
    uint8_t big[sizeof(Mt5TradeEvent)] = {0};
    out_len = 0;
    rc = PollEvent(&event_type, big, sizeof(big), &out_len, 0);
    assert(rc == 1);
    assert(event_type == EVENT_TRADE);
    assert(out_len == sizeof(Mt5TradeEvent));
    const Mt5TradeEvent* got = reinterpret_cast<const Mt5TradeEvent*>(big);
    assert(got->deal == 0xDEADBEEFULL);
    assert(got->order == 1);
    assert(got->position == 2);
    assert(got->event_seq == 42);

    // Queue is drained: empty queue with timeout 0 returns 0 (no event, no block).
    rc = PollEvent(&event_type, big, sizeof(big), &out_len, 0);
    assert(rc == 0);

    std::cout << "poll_event_test: all assertions passed (no truncated event delivered)" << std::endl;
    return 0;
}
