// Unit tests for PacketHdr wire framing and the Packer payload serializer.
//
// These tests exercise the DLL's packet-layer logic directly. mt5_bridge.cpp is
// included so the test binary shares the same translation unit as the
// implementation (no exported-function dependency).

#include <cassert>
#include <cstring>
#include <iostream>
#include <string>

#include "../mt5_bridge.cpp"

static void test_packet_hdr_round_trip() {
  // Build a REQUEST packet: CMD_ORDER_SEND (5), request_id 42, 20-byte payload.
  PacketHdr hdr{};
  hdr.length = 20;
  hdr.kind = PKT_REQUEST;
  hdr._pad = 0;
  hdr.id = static_cast<uint16_t>(CMD_ORDER_SEND);
  hdr.status = 0;
  hdr.request_id = 42;

  assert(sizeof(hdr) == 16);

  // Serialize to a byte buffer.
  uint8_t buf[16];
  std::memcpy(buf, &hdr, sizeof(hdr));

  // Deserialize back.
  PacketHdr hdr2{};
  std::memcpy(&hdr2, buf, sizeof(hdr2));

  assert(hdr2.length == 20);
  assert(hdr2.kind == PKT_REQUEST);
  assert(hdr2._pad == 0);
  assert(hdr2.id == static_cast<uint16_t>(CMD_ORDER_SEND));
  assert(hdr2.status == 0);
  assert(hdr2.request_id == 42);

  // RESPONSE packet with error status.
  PacketHdr resp{};
  resp.length = 0;
  resp.kind = PKT_RESPONSE;
  resp._pad = 0;
  resp.id = static_cast<uint16_t>(CMD_PING);
  resp.status = MT5_ERR_SEND_FAILED;
  resp.request_id = 7;

  std::memcpy(buf, &resp, sizeof(resp));
  PacketHdr resp2{};
  std::memcpy(&resp2, buf, sizeof(resp2));

  assert(resp2.length == 0);
  assert(resp2.kind == PKT_RESPONSE);
  assert(resp2.id == static_cast<uint16_t>(CMD_PING));
  assert(resp2.status == MT5_ERR_SEND_FAILED);
  assert(resp2.request_id == 7);

  // EVENT packet: request_id must be 0.
  PacketHdr evt{};
  evt.length = 88;
  evt.kind = PKT_EVENT;
  evt._pad = 0;
  evt.id = static_cast<uint16_t>(EVENT_TICK);
  evt.status = 0;
  evt.request_id = 0;

  std::memcpy(buf, &evt, sizeof(evt));
  PacketHdr evt2{};
  std::memcpy(&evt2, buf, sizeof(evt2));

  assert(evt2.kind == PKT_EVENT);
  assert(evt2.id == static_cast<uint16_t>(EVENT_TICK));
  assert(evt2.request_id == 0);
}

static void test_packer_primitives() {
  Packer p;

  p.u8(0xAB);
  p.u32(0xDEADBEEFu);
  p.i32(-12345);
  p.i64(-9876543210LL);
  p.u64(0x0123456789ABCDEFull);
  p.f64(3.141592653589793);
  p.str("EURUSD");

  const std::string &out = p.buf;

  // 1 + 4 + 4 + 8 + 8 + 8 + (4 + 6) = 43 bytes.
  assert(out.size() == 43);

  size_t off = 0;
  uint8_t u8v;
  std::memcpy(&u8v, out.data() + off, 1);
  assert(u8v == 0xAB);
  off += 1;

  uint32_t u32v;
  std::memcpy(&u32v, out.data() + off, 4);
  assert(u32v == 0xDEADBEEFu);
  off += 4;

  int32_t i32v;
  std::memcpy(&i32v, out.data() + off, 4);
  assert(i32v == -12345);
  off += 4;

  int64_t i64v;
  std::memcpy(&i64v, out.data() + off, 8);
  assert(i64v == -9876543210LL);
  off += 8;

  uint64_t u64v;
  std::memcpy(&u64v, out.data() + off, 8);
  assert(u64v == 0x0123456789ABCDEFull);
  off += 8;

  double f64v;
  std::memcpy(&f64v, out.data() + off, 8);
  assert(f64v == 3.141592653589793);
  off += 8;

  uint32_t str_len;
  std::memcpy(&str_len, out.data() + off, 4);
  assert(str_len == 6);
  off += 4;
  assert(std::memcmp(out.data() + off, "EURUSD", 6) == 0);
  off += 6;

  assert(off == out.size());
}

static void test_packer_empty_string() {
  Packer p;
  p.str(nullptr);
  assert(p.buf.size() == 4);
  uint32_t n = 0;
  std::memcpy(&n, p.buf.data(), 4);
  assert(n == 0);

  p.str("");
  assert(p.buf.size() == 8);
  std::memcpy(&n, p.buf.data() + 4, 4);
  assert(n == 0);
}

int main() {
  test_packet_hdr_round_trip();
  test_packer_primitives();
  test_packer_empty_string();

  std::cout << "test_frame: all assertions passed" << std::endl;
  return 0;
}
