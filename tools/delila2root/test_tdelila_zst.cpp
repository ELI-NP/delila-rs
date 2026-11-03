// test_tdelila_zst.cpp — TDelila reads a .delila.zst exactly like the .delila (TODO 81).
//
//   g++ -O2 -std=c++17 test_tdelila_zst.cpp -o test_tdelila_zst                    # `zstd -dc`
//   g++ -O2 -std=c++17 -DTDELILA_WITH_ZSTD test_tdelila_zst.cpp -lzstd -o ...     # libzstd
//   zstd -k run.delila && ./test_tdelila_zst run.delila run.delila.zst
//
// Compares every event (scalars, user_info and every waveform probe), the
// header and the footer; also checks that a broken / missing .zst fails cleanly.
#include <cstdio>
#include <fstream>
#include <string>

#include "TDelila.hpp"

static int failures = 0;
#define CHECK(cond, ...)                                              \
  do {                                                                \
    if (!(cond)) {                                                    \
      std::printf("FAIL %s:%d: %s — ", __FILE__, __LINE__, #cond);   \
      std::printf(__VA_ARGS__);                                       \
      std::printf("\n");                                              \
      if (++failures > 20) return 1;                                  \
    }                                                                 \
  } while (0)

static bool same_waveform(const tdelila::Waveform& a, const tdelila::Waveform& b) {
  for (int k = 1; k <= 3; ++k) {
    std::string n = "analog_probe" + std::to_string(k);
    if (a.int16_array(n) != b.int16_array(n)) return false;
  }
  for (int k = 1; k <= 16; ++k) {
    std::string n = "digital_probe" + std::to_string(k);
    if (a.uint8_array(n) != b.uint8_array(n)) return false;
  }
  return a.ns_per_sample() == b.ns_per_sample() && a.trigger_threshold() == b.trigger_threshold();
}

int main(int argc, char** argv) {
  if (argc != 3) {
    std::fprintf(stderr, "usage: %s run.delila run.delila.zst\n", argv[0]);
    return 2;
  }
  tdelila::TDelila a(argv[1]), b(argv[2]);
  CHECK(a.good(), "%s", a.error().c_str());
  CHECK(b.good(), "%s", b.error().c_str());
  if (failures) return 1;

  CHECK(a.header().run_number == b.header().run_number, "run number");
  CHECK(a.header().exp_name == b.header().exp_name, "exp name");
  CHECK(a.footer().present == b.footer().present, "footer present");
  CHECK(a.footer().total_events == b.footer().total_events, "footer total_events");

  tdelila::Event ea, eb;
  uint64_t n = 0, n_wf = 0;
  while (true) {
    bool ga = a.next(ea), gb = b.next(eb);
    CHECK(ga == gb, "event %llu: one file ended early", (unsigned long long)n);
    if (!ga || !gb) break;
    CHECK(ea.module() == eb.module() && ea.channel() == eb.channel(), "event %llu: module/channel",
          (unsigned long long)n);
    CHECK(ea.energy() == eb.energy() && ea.energy_short() == eb.energy_short(),
          "event %llu: energy", (unsigned long long)n);
    CHECK(ea.timestamp_ns() == eb.timestamp_ns(), "event %llu: timestamp", (unsigned long long)n);
    CHECK(ea.flags() == eb.flags(), "event %llu: flags", (unsigned long long)n);
    for (int s = 0; s < 4; ++s)
      CHECK(ea.user_info(s) == eb.user_info(s), "event %llu: user_info[%d]", (unsigned long long)n, s);
    CHECK(ea.has_waveform() == eb.has_waveform(), "event %llu: has_waveform", (unsigned long long)n);
    if (ea.has_waveform() && eb.has_waveform()) {
      CHECK(same_waveform(ea.waveform(), eb.waveform()), "event %llu: waveform", (unsigned long long)n);
      ++n_wf;
    }
    ++n;
  }
  CHECK(a.error().empty() && b.error().empty(), "read errors: '%s' / '%s'", a.error().c_str(),
        b.error().c_str());
  if (a.footer().present) CHECK(n == a.footer().total_events, "events %llu vs footer", (unsigned long long)n);

  // Failure paths: no silent empty reader.
  tdelila::TDelila missing("/nonexistent/run.delila.zst");
  CHECK(!missing.good() && !missing.error().empty(), "missing .zst must fail");
  const std::string junk = std::string(argv[2]) + ".junk.zst";
  { std::ofstream(junk) << "not zstd at all"; }
  tdelila::TDelila broken(junk);
  CHECK(!broken.good() && !broken.error().empty(), "broken .zst must fail");
  std::remove(junk.c_str());
  std::printf("missing: %s\nbroken:  %s\n", missing.error().c_str(), broken.error().c_str());

  std::printf("%s: %llu events (%llu with waveforms) identical, header/footer identical\n",
              failures ? "FAILED" : "OK", (unsigned long long)n, (unsigned long long)n_wf);
  return failures ? 1 : 0;
}
