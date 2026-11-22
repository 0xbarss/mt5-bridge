// End-to-end Connection tests over a UNIX socketpair (Linux only).
//
// Exercises the synchronous request/response multiplexing layer without an MT5
// terminal: a socketpair stands in for the named pipe, the test drives the
// EA side by reading request frames and writing response frames back.
//
// Covers:
//   * request() returns RESPONDED when a matching response arrives
//   * concurrent requests receive distinct request IDs (multiplexing)
//   * a request times out and returns UNKNOWN when no response arrives
//   * a late response arriving after its request is gone is counted as an
//   orphan

#include <cassert>
#include <cstring>
#include <iostream>
#include <string>
#include <sys/socket.h>
#include <sys/types.h>
#include <thread>
#include <unistd.h>

#include "../mt5_bridge.cpp"

namespace {

// Write all bytes to a file descriptor.
bool write_all(int fd, const void *data, size_t len) {
  const char *p = static_cast<const char *>(data);
  while (len > 0) {
    ssize_t n = ::write(fd, p, len);
    if (n <= 0)
      return false;
    p += n;
    len -= static_cast<size_t>(n);
  }
  return true;
}

// Read exactly len bytes from a file descriptor.
bool read_all(int fd, void *data, size_t len) {
  char *p = static_cast<char *>(data);
  while (len > 0) {
    ssize_t n = ::read(fd, p, len);
    if (n <= 0)
      return false;
    p += n;
    len -= static_cast<size_t>(n);
  }
  return true;
}

// Build a RESPONSE packet frame for a given request id and status.
std::string build_response_frame(uint32_t request_id, uint16_t cmd,
                                 int32_t status, const std::string &payload) {
  PacketHdr hdr{};
  hdr.length = static_cast<uint32_t>(payload.size());
  hdr.kind = PKT_RESPONSE;
  hdr._pad = 0;
  hdr.id = cmd;
  hdr.status = status;
  hdr.request_id = request_id;

  std::string frame;
  frame.resize(sizeof(hdr) + payload.size());
  std::memcpy(&frame[0], &hdr, sizeof(hdr));
  if (!payload.empty()) {
    std::memcpy(&frame[sizeof(hdr)], payload.data(), payload.size());
  }
  return frame;
}

// Read one frame from a socket, returning {hdr, payload}.
bool read_frame(int fd, PacketHdr &hdr, std::string &payload) {
  if (!read_all(fd, &hdr, sizeof(hdr)))
    return false;
  if (hdr.length > MT5_MAX_FRAME_BYTES)
    return false;
  payload.resize(hdr.length);
  if (hdr.length > 0) {
    if (!read_all(fd, &payload[0], hdr.length))
      return false;
  }
  return true;
}

} // namespace

int main() {
  // Shorten query timeouts so the timeout test does not block for seconds.
  setenv("MT5_QUERY_TIMEOUT_SEC", "1", 1);

  // ── Create a socketpair to stand in for the named pipe ──────────────────
  int fds[2];
  assert(socketpair(AF_UNIX, SOCK_STREAM, 0, fds) == 0);
  int test_fd = fds[0]; // test side: plays the EA
  int dll_fd = fds[1];  // DLL side: the Connection's pipe

  // ── Create and start the Connection ─────────────────────────────────────
  auto conn = std::make_shared<Connection>((HANDLE)(intptr_t)dll_fd);
  {
    std::lock_guard<std::mutex> lk(g_conn_mu);
    g_conn = conn;
  }
  conn->start_reader();

  // ── Test 1: NOT_CONNECTED with an invalid handle ────────────────────────
  auto dead = std::make_shared<Connection>(INVALID_HANDLE_VALUE);
  int32_t st = 0;
  std::string data;
  Outcome out = dead->request(CMD_PING, {}, st, data, 1);
  assert(out == Outcome::NOT_CONNECTED);
  assert(st == MT5_ERR_PIPE_DISCONNECTED);

  // ── Test 2: successful request/response round trip ──────────────────────
  int32_t st2 = 0;
  std::string data2;
  // Launch the request on the DLL side (writes a REQUEST frame to dll_fd).
  bool request_done = false;
  Outcome out2 = Outcome::NOT_CONNECTED;
  std::thread request_thread([&]() {
    out2 = conn->request(CMD_PING, {}, st2, data2, 5);
    request_done = true;
  });

  // EA side: read the REQUEST frame.
  PacketHdr req_hdr{};
  std::string req_payload;
  assert(read_frame(test_fd, req_hdr, req_payload));
  assert(req_hdr.kind == PKT_REQUEST);
  assert(req_hdr.id == static_cast<uint16_t>(CMD_PING));
  uint32_t req_id_a = req_hdr.request_id;

  // EA side: write a matching RESPONSE frame back.
  std::string resp_payload(20, '\0');
  std::string resp_frame =
      build_response_frame(req_id_a, CMD_PING, 1, resp_payload);
  assert(write_all(test_fd, resp_frame.data(), resp_frame.size()));

  request_thread.join();
  assert(out2 == Outcome::RESPONDED);
  assert(st2 == 1);

  // ── Test 3: concurrent requests get distinct request IDs (multiplexing) ──
  int32_t st3a = 0, st3b = 0;
  std::string data3a, data3b;
  std::atomic<bool> done_a{false}, done_b{false};
  Outcome out3a = Outcome::NOT_CONNECTED, out3b = Outcome::NOT_CONNECTED;

  std::thread t_a([&]() {
    out3a = conn->request(CMD_ACCOUNT, {}, st3a, data3a, 5);
    done_a = true;
  });
  std::thread t_b([&]() {
    out3b = conn->request(CMD_POSITIONS_GET, {}, st3b, data3b, 5);
    done_b = true;
  });

  // Read both request frames (order may vary).
  PacketHdr h1{}, h2{};
  std::string p1, p2;
  assert(read_frame(test_fd, h1, p1));
  assert(read_frame(test_fd, h2, p2));

  // The two request IDs must be distinct.
  assert(h1.request_id != h2.request_id);

  // Respond to each request with its own request id.
  for (const PacketHdr *h : {&h1, &h2}) {
    std::string frame = build_response_frame(h->request_id, h->id, 1, {});
    assert(write_all(test_fd, frame.data(), frame.size()));
  }

  t_a.join();
  t_b.join();
  assert(out3a == Outcome::RESPONDED);
  assert(out3b == Outcome::RESPONDED);

  // ── Test 4: timeout → UNKNOWN, late response → orphan ───────────────────
  int32_t st4 = 0;
  std::string data4;
  std::atomic<bool> done4{false};
  Outcome out4 = Outcome::NOT_CONNECTED;
  std::thread t4([&]() {
    out4 = conn->request(CMD_RATES, {}, st4, data4, 5);
    done4 = true;
  });

  // EA side: read the REQUEST frame but do NOT respond.
  PacketHdr h4{};
  std::string p4;
  assert(read_frame(test_fd, h4, p4));
  assert(h4.kind == PKT_REQUEST);

  // Wait for the request to time out (1 second via MT5_QUERY_TIMEOUT_SEC).
  while (!done4) {
    usleep(10000);
  }
  t4.join();
  assert(out4 == Outcome::UNKNOWN);
  assert(st4 == MT5_ERR_UNKNOWN_EXECUTION);

  // EA side: now send a late response for the already-gone request.
  std::string late_frame =
      build_response_frame(h4.request_id, CMD_RATES, 1, {});
  assert(write_all(test_fd, late_frame.data(), late_frame.size()));
  usleep(100000); // let the reader loop process the late response

  // The late response has no matching pending request → counted as an orphan.
  assert(GetOrphanCount() == 1);

  // ── Cleanup ─────────────────────────────────────────────────────────────
  close(test_fd);
  close(dll_fd);

  std::cout << "test_connection: all assertions passed (round trip, "
               "multiplexing, orphan)"
            << std::endl;
  return 0;
}
