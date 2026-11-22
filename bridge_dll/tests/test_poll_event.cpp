// PollEvent behavior tests.
//
// Verifies that PollEvent never truncates an event into an undersized buffer:
// it reports MT5_ERR_BUFFER_TOO_SMALL, writes the required size to out_len,
// copies nothing, and leaves the event at the head of the queue. A buffer that
// is large enough receives the event whole and parses it as a valid struct.
//
// mt5_bridge.cpp is included so the test can drive the file-static event
// queue (g_event_queue / g_event_mutex) through the same code path the Rust
// event pump uses.

#include <cassert>
#include <cstring>
#include <iostream>

#include "../mt5_bridge.cpp"

static void queue_event(uint16_t event_type, const void *payload,
                        uint32_t len) {
  std::lock_guard<std::mutex> lk(g_event_mutex);
  g_event_queue.push(
      {event_type, std::string(static_cast<const char *>(payload), len)});
}

static void test_empty_queue_timeout_zero() {
  uint8_t buf[16] = {0};
  uint32_t out_len = 0;
  uint16_t event_type = 0;
  assert(PollEvent(&event_type, buf, sizeof(buf), &out_len, 0) == 0);
}

static void test_buffer_too_small() {
  // Queue a 176-byte trade event (the largest event type) and read it with a
  // 4-byte buffer: must report "buffer too small" without truncation.
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
  assert(out_len == sizeof(Mt5TradeEvent));      // required size reported
  assert(small[0] == 0xAA && small[1] == 0xBB && // nothing copied
         small[2] == 0xCC && small[3] == 0xDD);
  assert(event_type == 0xFFFFu); // out_event_type untouched

  // The event stays at the head of the queue: repeating the undersized read
  // reports the same required size instead of delivering garbage.
  out_len = 0;
  rc = PollEvent(&event_type, small, sizeof(small), &out_len, 0);
  assert(rc == MT5_ERR_BUFFER_TOO_SMALL);
  assert(out_len == sizeof(Mt5TradeEvent));
}

static void test_whole_event_delivery() {
  // A buffer large enough receives the event whole; it parses as a valid
  // struct. (The event left queued by test_buffer_too_small is still at the
  // head.)
  uint8_t big[sizeof(Mt5TradeEvent)] = {0};
  uint32_t out_len = 0;
  uint16_t event_type = 0;

  int rc = PollEvent(&event_type, big, sizeof(big), &out_len, 0);
  assert(rc == 1);
  assert(event_type == EVENT_TRADE);
  assert(out_len == sizeof(Mt5TradeEvent));
  const Mt5TradeEvent *got = reinterpret_cast<const Mt5TradeEvent *>(big);
  assert(got->deal == 0xDEADBEEFULL);
  assert(got->order == 1);
  assert(got->position == 2);
  assert(got->event_seq == 42);

  // Queue is drained: empty queue with timeout 0 returns 0 (no event, no
  // block).
  rc = PollEvent(&event_type, big, sizeof(big), &out_len, 0);
  assert(rc == 0);
}

int main() {
  test_empty_queue_timeout_zero();
  test_buffer_too_small();
  test_whole_event_delivery();

  std::cout
      << "test_poll_event: all assertions passed (no truncated event delivered)"
      << std::endl;
  return 0;
}
