// Wine smoke test for mt5_bridge.dll: loads the DLL under Wine and verifies
// exported functions resolve and basic calls do not crash.
//
// Compile (MinGW cross from Linux):
//   x86_64-w64-mingw32-g++ -O2 -std=c++17 -o /tmp/wine_dll_test.exe
//   wine_dll_test.cpp
// Run under Wine:
//   wine /tmp/wine_dll_test.exe

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#if defined(_WIN32)
#include <windows.h>
#endif

// Function pointer types matching mt5_bridge.h exports.
typedef int (*PfnIsConnected)(void);
typedef uint64_t (*PfnGetOrphanCount)(void);
typedef int (*PfnEventsDroppedTotal)(void);
typedef int (*PfnPing)(int64_t *, uint64_t *, uint32_t *);
typedef int (*PfnStats)(char *, int);
typedef int (*PfnSymbolsTotal)(void);
typedef int (*PfnShutdown)(void);

int main() {
#if defined(_WIN32)
  HMODULE h = LoadLibraryA("mt5_bridge.dll");
  if (!h) {
    h = LoadLibraryA("../mt5_bridge.dll");
  }
  if (!h) {
    fprintf(stderr, "FAIL: LoadLibrary(mt5_bridge.dll) returned NULL\n");
    return 1;
  }
  printf("OK: mt5_bridge.dll loaded under Wine\n");

  auto resolve = [&](const char *name) -> FARPROC {
    FARPROC p = GetProcAddress(h, name);
    if (!p)
      fprintf(stderr, "WARN: GetProcAddress(%s) = NULL\n", name);
    return p;
  };

  PfnIsConnected is_connected = (PfnIsConnected)resolve("IsConnected");
  PfnGetOrphanCount get_orphan = (PfnGetOrphanCount)resolve("GetOrphanCount");
  PfnEventsDroppedTotal events_dropped =
      (PfnEventsDroppedTotal)resolve("EventsDroppedTotal");
  PfnPing ping = (PfnPing)resolve("Ping");
  PfnStats stats = (PfnStats)resolve("Stats");
  PfnSymbolsTotal symbols_total = (PfnSymbolsTotal)resolve("SymbolsTotal");
  PfnShutdown shutdown = (PfnShutdown)resolve("Shutdown");

  if (!is_connected || !get_orphan || !events_dropped || !ping || !stats ||
      !symbols_total || !shutdown) {
    fprintf(stderr, "FAIL: one or more required exports missing\n");
    FreeLibrary(h);
    return 1;
  }
  printf("OK: all required exports resolved\n");

  int connected = is_connected();
  printf("OK: IsConnected() = %d (expected 0 without MT5)\n", connected);

  uint64_t orphans = get_orphan();
  printf("OK: GetOrphanCount() = %llu\n", (unsigned long long)orphans);

  uint64_t dropped = events_dropped();
  printf("OK: EventsDroppedTotal() = %llu\n", (unsigned long long)dropped);

  char buf[4] = {0};
  int st = stats(buf, sizeof(buf));
  printf("OK: Stats(small buf) = %d\n", st);

  int n = symbols_total();
  printf("OK: SymbolsTotal() = %d (expected -1 without MT5)\n", n);

  int64_t stime = 0;
  uint64_t mono = 0;
  uint32_t proto = 0;
  int pr = ping(&stime, &mono, &proto);
  printf("OK: Ping() = %d (expected 0 without MT5)\n", pr);

  shutdown();
  FreeLibrary(h);
  printf("OK: Shutdown + FreeLibrary completed without crash\n");
  printf("wine_dll_test: PASS\n");
#else
  printf("wine_dll_test: this test must run under Wine (Windows target).\n");
  return 0;
#endif
}
