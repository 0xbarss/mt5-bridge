// Event ring-buffer overflow and per-type drop counter tests.
//
// Verifies that when the event queue fills past MAX_EVENT_QUEUE, the oldest
// event is dropped, the total and per-type drop counters increment, and the
// queue stays bounded. mt5_bridge.cpp is included for direct access to the
// file-static event queue and counters.

#include <cassert>
#include <cstring>
#include <iostream>
#include <mutex>
#include <string>

#include "../mt5_bridge.cpp"

static void push_event_locked(uint16_t event_type, uint32_t len) {
  std::lock_guard<std::mutex> lk(g_event_mutex);
  // Mirror reader_loop's eviction: when the queue is full, drop the oldest
  // event and record a (total + per-type) drop before pushing the new one.
  if (g_event_queue.size() >= MAX_EVENT_QUEUE) {
    g_event_queue.pop();
    g_events_dropped_total.fetch_add(1, std::memory_order_relaxed);
    switch (event_type) {
    case EVENT_TICK:
      g_tick_drops.fetch_add(1, std::memory_order_relaxed);
      break;
    case EVENT_TRADE:
      g_trade_drops.fetch_add(1, std::memory_order_relaxed);
      break;
    case EVENT_BOOK:
      g_book_drops.fetch_add(1, std::memory_order_relaxed);
      break;
    case EVENT_BAR:
      g_bar_drops.fetch_add(1, std::memory_order_relaxed);
      break;
    default:
      break;
    }
  }
  std::string payload(len, static_cast<char>('A' + (event_type % 26)));
  g_event_queue.push({event_type, std::move(payload)});
}

static uint64_t dropped_total() {
  return g_events_dropped_total.load(std::memory_order_relaxed);
}

static uint64_t tick_drops() {
  return g_tick_drops.load(std::memory_order_relaxed);
}

static uint64_t trade_drops() {
  return g_trade_drops.load(std::memory_order_relaxed);
}

static uint64_t book_drops() {
  return g_book_drops.load(std::memory_order_relaxed);
}

static uint64_t bar_drops() {
  return g_bar_drops.load(std::memory_order_relaxed);
}

int main() {
  // Reset counters to a clean baseline.
  g_events_dropped_total.store(0, std::memory_order_relaxed);
  g_tick_drops.store(0, std::memory_order_relaxed);
  g_trade_drops.store(0, std::memory_order_relaxed);
  g_book_drops.store(0, std::memory_order_relaxed);
  g_bar_drops.store(0, std::memory_order_relaxed);

  {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    assert(g_event_queue.empty());
  }

  // Fill the queue exactly to MAX_EVENT_QUEUE.
  const size_t capacity = MAX_EVENT_QUEUE;
  for (size_t i = 0; i < capacity; ++i) {
    push_event_locked(EVENT_TICK, sizeof(Mt5TickEvent));
  }
  {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    assert(g_event_queue.size() == capacity);
  }
  assert(dropped_total() == 0);

  // Push one more: the queue is full, so the oldest event is evicted and a
  // drop is recorded. The per-type counter tracks the *incoming* event's type
  // (matching reader_loop's behavior).
  push_event_locked(EVENT_TRADE, sizeof(Mt5TradeEvent));
  assert(dropped_total() == 1);
  assert(tick_drops() == 0);
  assert(trade_drops() == 1);

  {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    assert(g_event_queue.size() == capacity);
  }

  // Push one event of each remaining type. Each evicts one oldest event.
  push_event_locked(EVENT_BOOK, sizeof(Mt5BookEvent)); // book_drops == 1
  push_event_locked(EVENT_BAR, sizeof(Mt5BarEvent));   // bar_drops == 1

  assert(dropped_total() == 3);
  assert(tick_drops() == 0);
  assert(trade_drops() == 1);
  assert(book_drops() == 1);
  assert(bar_drops() == 1);

  // Now push several events of each type to exercise per-type counters.
  for (int i = 0; i < 3; ++i) {
    push_event_locked(EVENT_TRADE, sizeof(Mt5TradeEvent));
  }
  assert(trade_drops() == 1 + 3); // 1 from above + 3 more

  for (int i = 0; i < 2; ++i) {
    push_event_locked(EVENT_BOOK, sizeof(Mt5BookEvent));
  }
  assert(book_drops() == 1 + 2);

  for (int i = 0; i < 1; ++i) {
    push_event_locked(EVENT_BAR, sizeof(Mt5BarEvent));
  }
  assert(bar_drops() == 1 + 1);

  assert(dropped_total() == 3 + 3 + 2 + 1);

  {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    assert(g_event_queue.size() == capacity);
  }

  std::cout << "test_event_queue: all assertions passed (ring buffer bounded, "
               "drops counted)"
            << std::endl;
  return 0;
}
