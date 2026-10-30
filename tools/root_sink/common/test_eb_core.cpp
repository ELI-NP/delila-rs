// test_eb_core.cpp — unit tests for eb_core.hpp (no ROOT, no ZMQ, no network).
//
// Covers the multithreaded event-builder core: Channel, SeqReorder, the
// SortedChunk splitter (including the TODO 58 H10 chunk-boundary rules ported
// from src/event_builder/chunk_builder.rs), the data-time-driven Sorter, the
// ThGEM built-event builder, and the generic event layer (TODO 71:
// TriggerGrouper + build_generic_events, checked against a brute-force
// reference of the rules over randomized chunkings).
//
// Build & run (Channel tests spawn threads, hence -pthread):
//   g++ -std=c++17 -O0 -g -pthread test_eb_core.cpp -o /tmp/teb && /tmp/teb
//
// Uses the same tiny CHECK harness as test_sink_core.cpp (deliberate copy —
// the two binaries stay independent so the 215 existing checks are never
// touched by eb work).

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <memory>
#include <random>
#include <thread>
#include <vector>

#include "eb_core.hpp"
#include "time_offsets.hpp"

using namespace rootsink;
using namespace rootsink::eb;

static int g_pass = 0;
static int g_fail = 0;

#define CHECK(cond)                                                     \
  do {                                                                  \
    if (cond) {                                                         \
      ++g_pass;                                                         \
    } else {                                                            \
      ++g_fail;                                                         \
      std::printf("FAIL %s:%d  CHECK(%s)\n", __FILE__, __LINE__, #cond); \
    }                                                                   \
  } while (0)

static bool near(double a, double b, double eps = 1e-9) {
  double d = a - b;
  if (d < 0) d = -d;
  return d <= eps;
}

// Convenience: a hit with just channel + timestamp (energies default 0).
static ScalarHit hit(int ch, double t, uint16_t e = 0) {
  ScalarHit h;
  h.channel = static_cast<uint8_t>(ch);
  h.timestamp_ns = t;
  h.energy = e;
  return h;
}

// ---------------------------------------------------------------------------
// Channel
// ---------------------------------------------------------------------------

static void test_channel_fifo() {
  Channel<int> ch;
  int a = 1, b = 2, c = 3;
  ch.push(std::move(a));
  ch.push(std::move(b));
  ch.push(std::move(c));
  CHECK(ch.size() == 3);
  int out = 0;
  CHECK(ch.pop(out) && out == 1);
  CHECK(ch.pop(out) && out == 2);
  CHECK(ch.pop(out) && out == 3);
  CHECK(ch.size() == 0);
}

static void test_channel_blocking_pop() {
  Channel<int> ch;
  int got = 0;
  std::thread consumer([&] {
    int v = 0;
    if (ch.pop(v)) got = v;  // blocks until the push below
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(20));
  int v = 42;
  ch.push(std::move(v));
  consumer.join();
  CHECK(got == 42);
}

static void test_channel_close_drains_then_false() {
  Channel<int> ch;
  int a = 7, b = 8;
  ch.push(std::move(a));
  ch.push(std::move(b));
  ch.close();
  int out = 0;
  CHECK(ch.pop(out) && out == 7);  // pending items still poppable after close
  CHECK(ch.pop(out) && out == 8);
  CHECK(!ch.pop(out));  // closed AND drained -> false

  // A pop blocked on an empty channel must wake on close and return false.
  Channel<int> ch2;
  std::atomic<bool> returned_false{false};
  std::thread blocked([&] {
    int v = 0;
    if (!ch2.pop(v)) returned_false = true;
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(20));
  ch2.close();
  blocked.join();
  CHECK(returned_false);
}

static void test_channel_bounded_try_push() {
  Channel<int> ch(2);  // bounded(2) — the display-tee shape
  int a = 1, b = 2, c = 3;
  CHECK(ch.try_push(std::move(a)));
  CHECK(ch.try_push(std::move(b)));
  CHECK(!ch.try_push(std::move(c)));  // full -> false...
  CHECK(c == 3);                      // ...and the value was NOT consumed
  int out = 0;
  CHECK(ch.pop(out) && out == 1);
  CHECK(ch.try_push(std::move(c)));  // space again -> accepted
  CHECK(ch.size() == 2);
}

static void test_channel_pop_for_timeout() {
  Channel<int> ch;
  int out = 0;
  auto t0 = std::chrono::steady_clock::now();
  CHECK(ch.pop_for(out, 30) == PopResult::Timeout);  // open + empty -> Timeout
  auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now() - t0)
                .count();
  CHECK(ms >= 25);  // actually waited (allow scheduler slop)
  int v = 5;
  ch.push(std::move(v));
  CHECK(ch.pop_for(out, 30) == PopResult::Value && out == 5);
}

// Issue #26: pop_for must distinguish Timeout from Closed, and must only
// report Closed once the queue is drained — otherwise a consumer that exits
// on an external flag can race a producer's final push(msg); close() and
// lose the last message.
static void test_channel_pop_for_three_state() {
  // Drain-then-Closed: pending items outlive close(), Closed comes last.
  Channel<int> ch;
  int a = 7, b = 8;
  ch.push(std::move(a));
  ch.push(std::move(b));
  ch.close();
  int out = 0;
  CHECK(ch.pop_for(out, 30) == PopResult::Value && out == 7);
  CHECK(ch.pop_for(out, 30) == PopResult::Value && out == 8);
  CHECK(ch.pop_for(out, 30) == PopResult::Closed);
  CHECK(ch.pop_for(out, 0) == PopResult::Closed);  // stays Closed, 0-timeout too

  // A pop_for blocked on an empty channel must wake on close() -> Closed
  // (not sit out its full timeout).
  Channel<int> ch2;
  std::atomic<bool> got_closed{false};
  std::thread blocked([&] {
    int v = 0;
    if (ch2.pop_for(v, 10000) == PopResult::Closed) got_closed = true;
  });
  std::this_thread::sleep_for(std::chrono::milliseconds(20));
  ch2.close();
  blocked.join();
  CHECK(got_closed);
}

// The issue #26 race shape, end to end: a consumer that exits ONLY on Closed
// receives every message even when the producer's last push and close() land
// mid-poll. With the old bool contract + external stop flag this
// intermittently lost the final message.
static void test_channel_pop_for_close_race() {
  const int kRounds = 50;  // the original race was intermittent — hammer it
  for (int round = 0; round < kRounds; ++round) {
    Channel<int> ch;
    const int kMsgs = 3;
    std::thread producer([&] {
      for (int i = 0; i < kMsgs; ++i) {
        int v = i;
        ch.push(std::move(v));
      }
      ch.close();  // close() IS the termination signal here — no pill
    });
    int received = 0;
    for (;;) {
      int v = 0;
      PopResult r = ch.pop_for(v, 1);
      if (r == PopResult::Closed) break;
      if (r == PopResult::Value) ++received;
    }
    producer.join();
    if (received != kMsgs) {
      CHECK(received == kMsgs);  // report the losing round
      return;
    }
  }
  CHECK(true);  // all rounds delivered every message
}

static void test_channel_move_only() {
  // The zero-copy contract: payloads move through, never copy.
  Channel<std::unique_ptr<int>> ch;
  ch.push(std::make_unique<int>(99));
  std::unique_ptr<int> out;
  CHECK(ch.pop(out) && out && *out == 99);
}

static void test_channel_multi_producer() {
  Channel<int> ch;
  const int kProducers = 4, kEach = 250;
  std::vector<std::thread> producers;
  for (int p = 0; p < kProducers; ++p) {
    producers.emplace_back([&ch, p] {
      for (int i = 0; i < kEach; ++i) {
        int v = p * kEach + i;
        ch.push(std::move(v));
      }
    });
  }
  long long sum = 0;
  int count = 0;
  std::thread consumer([&] {
    int v = 0;
    while (count < kProducers * kEach && ch.pop(v)) {
      sum += v;
      ++count;
    }
  });
  for (auto& t : producers) t.join();
  consumer.join();
  CHECK(count == kProducers * kEach);  // conservation: nothing lost
  CHECK(sum == 1000LL * 999 / 2);      // sum of 0..999
}

// ---------------------------------------------------------------------------
// SeqReorder
// ---------------------------------------------------------------------------

static void test_seq_reorder_in_order() {
  SeqReorder<int> r;
  int out = 0;
  r.push(0, 10);
  CHECK(r.pop_ready(out) && out == 10);
  r.push(1, 11);
  CHECK(r.pop_ready(out) && out == 11);
  CHECK(!r.pop_ready(out));
}

static void test_seq_reorder_out_of_order() {
  SeqReorder<int> r;
  int out = 0;
  r.push(2, 12);
  r.push(0, 10);
  r.push(1, 11);
  CHECK(r.pop_ready(out) && out == 10);
  CHECK(r.pop_ready(out) && out == 11);
  CHECK(r.pop_ready(out) && out == 12);
  CHECK(!r.pop_ready(out));
}

static void test_seq_reorder_gap_holds() {
  SeqReorder<int> r;
  int out = 0;
  r.push(1, 11);  // seq 0 missing
  CHECK(!r.pop_ready(out));
  CHECK(r.pending() == 1);
  r.push(0, 10);
  CHECK(r.pop_ready(out) && out == 10);
  CHECK(r.pop_ready(out) && out == 11);
  CHECK(r.pending() == 0);
}

// ---------------------------------------------------------------------------
// sort_and_split / sort_and_flush / core_range
// ---------------------------------------------------------------------------

static const double kInf = std::numeric_limits<double>::infinity();

static void test_split_empty() {
  std::vector<ScalarHit> buf;
  SortedChunk c;
  CHECK(!sort_and_split(buf, 50000.0, 500.0, -kInf, c));
}

static void test_split_insufficient_span() {
  // core_end = latest - horizon must clear the earliest hit, else no split.
  std::vector<ScalarHit> buf = {hit(0, 3000.0), hit(0, 1000.0), hit(0, 2000.0)};
  SortedChunk c;
  CHECK(!sort_and_split(buf, 50000.0, 500.0, -kInf, c));
  CHECK(buf.size() == 3);  // buffer kept...
  CHECK(near(buf[0].timestamp_ns, 1000.0) && near(buf[2].timestamp_ns, 3000.0));
  // ...and sorted (documented side effect)
}

static void test_split_retained_from_lookback() {
  // latest=51100, horizon=50000 -> core_end=1100; lookback=500 -> retained
  // begins at 600 (the H10 fix: context BEFORE core_end is carried too).
  std::vector<ScalarHit> buf = {hit(0, 100.0), hit(1, 700.0), hit(2, 1000.0),
                                hit(3, 51100.0)};
  SortedChunk c;
  CHECK(sort_and_split(buf, 50000.0, 500.0, -kInf, c));
  CHECK(near(c.core_end, 1100.0));
  CHECK(c.core_start == -kInf);
  CHECK(c.hits.size() == 4);  // chunk carries ALL hits incl. the overlap
  CHECK(buf.size() == 3);     // retained: 700, 1000 (>= 600) and 51100
  CHECK(near(buf[0].timestamp_ns, 700.0));
  CHECK(near(buf[1].timestamp_ns, 1000.0));
  CHECK(near(buf[2].timestamp_ns, 51100.0));
}

static void test_split_chain_invariant() {
  // chunk[N+1].core_start == chunk[N].core_end, maintained by the caller
  // passing prev core_end in — exactly what Sorter does.
  std::vector<ScalarHit> buf = {hit(0, 0.0), hit(0, 60000.0)};
  SortedChunk c1;
  CHECK(sort_and_split(buf, 50000.0, 500.0, -kInf, c1));
  CHECK(near(c1.core_end, 10000.0));
  buf.push_back(hit(0, 130000.0));
  SortedChunk c2;
  CHECK(sort_and_split(buf, 50000.0, 500.0, c1.core_end, c2));
  CHECK(near(c2.core_start, c1.core_end));
  CHECK(near(c2.core_end, 80000.0));
}

static void test_flush() {
  std::vector<ScalarHit> buf = {hit(0, 2000.0), hit(0, 1000.0)};
  SortedChunk c;
  CHECK(sort_and_flush(buf, 1100.0, c));
  CHECK(buf.empty());
  CHECK(c.hits.size() == 2 && near(c.hits[0].timestamp_ns, 1000.0));
  CHECK(near(c.core_start, 1100.0));  // floor honored: 1000 was emitted before
  CHECK(c.core_end == kInf);
  std::vector<ScalarHit> empty;
  SortedChunk c2;
  CHECK(!sort_and_flush(empty, -kInf, c2));
}

static void test_split_shuffled_sorts() {
  std::vector<ScalarHit> buf;
  for (int i = 0; i < 200; ++i) buf.push_back(hit(0, (i * 37) % 200 * 10.0));
  buf.push_back(hit(0, 100000.0));
  SortedChunk c;
  CHECK(sort_and_split(buf, 50000.0, 500.0, -kInf, c));
  bool sorted = true;
  for (std::size_t i = 1; i < c.hits.size(); ++i)
    if (c.hits[i - 1].timestamp_ns > c.hits[i].timestamp_ns) sorted = false;
  CHECK(sorted);
}

static void test_core_range_conservation() {
  // Over consecutive chunks + final flush, the core ranges partition the full
  // hit set: every hit written exactly once (guards the hits tree).
  std::mt19937 rng(12345);
  std::uniform_real_distribution<double> jitter(0.0, 5000.0);
  std::vector<double> all_ts;
  for (int i = 0; i < 500; ++i) all_ts.push_back(i * 1000.0 + jitter(rng));

  std::vector<ScalarHit> buf;
  double prev_core_end = -kInf;
  std::size_t written = 0;
  std::size_t feed = 0;
  double max_written_ts = -kInf;
  bool monotone = true;
  auto write_core = [&](const SortedChunk& c) {
    auto range = core_range(c);
    for (std::size_t i = range.first; i < range.second; ++i) {
      if (c.hits[i].timestamp_ns < max_written_ts) monotone = false;
      max_written_ts = c.hits[i].timestamp_ns;
      ++written;
    }
  };
  // Feed in batches of 50, splitting whenever possible (tiny horizon/span so
  // several chunks happen).
  while (feed < all_ts.size()) {
    for (int i = 0; i < 50 && feed < all_ts.size(); ++i, ++feed)
      buf.push_back(hit(0, all_ts[feed]));
    SortedChunk c;
    while (sort_and_split(buf, 20000.0, 1000.0, prev_core_end, c)) {
      prev_core_end = c.core_end;
      write_core(c);
      c = SortedChunk{};
    }
  }
  SortedChunk fin;
  if (sort_and_flush(buf, prev_core_end, fin)) write_core(fin);
  CHECK(written == all_ts.size());  // no loss, no duplication
  CHECK(monotone);                  // and globally time-sorted across chunks
}

static void test_core_range_bounds() {
  SortedChunk c;
  c.hits = {hit(0, 100.0), hit(0, 600.0), hit(0, 1000.0), hit(0, 1100.0),
            hit(0, 51100.0)};
  c.core_start = 700.0;
  c.core_end = 1100.0;
  auto r = core_range(c);
  // [700, 1100): excludes the lookback head (100, 600) and the boundary hit
  // at exactly core_end (owned by the NEXT chunk) and the tail.
  CHECK(r.first == 2 && r.second == 3);
  CHECK(near(c.hits[r.first].timestamp_ns, 1000.0));
}

// ---------------------------------------------------------------------------
// Sorter
// ---------------------------------------------------------------------------

static Sorter::Config small_cfg() {
  Sorter::Config cfg;
  cfg.chunk_span_ns = 10000.0;   // 10 us span
  cfg.safe_horizon_ns = 5000.0;  // 5 us horizon
  cfg.lookback_ns = 1000.0;
  return cfg;
}

static void test_sorter_data_time_only() {
  // Many hits inside a tiny time span: count alone must NOT trigger a chunk.
  Sorter s(small_cfg());
  int chunks = 0;
  std::vector<ScalarHit> batch;
  for (int i = 0; i < 10000; ++i) batch.push_back(hit(0, 100.0 + i * 0.01));
  s.push_batch(std::move(batch), [&](SortedChunk&&) { ++chunks; });
  CHECK(chunks == 0);
  CHECK(s.buffered() == 10000);
}

static void test_sorter_first_chunk_floor() {
  // First chunk: floor = earliest hit. Emit once (max - horizon) - earliest
  // >= span, i.e. max >= earliest + 15000 with the small config.
  Sorter s(small_cfg());
  int chunks = 0;
  SortedChunk last;
  std::vector<ScalarHit> b1 = {hit(0, 1000.0), hit(0, 15999.0)};
  s.push_batch(std::move(b1), [&](SortedChunk&&) { ++chunks; });
  CHECK(chunks == 0);  // candidate 10999 - 1000 = 9999 < 10000
  std::vector<ScalarHit> b2 = {hit(0, 16001.0)};
  s.push_batch(std::move(b2), [&](SortedChunk&& c) {
    ++chunks;
    last = std::move(c);
  });
  CHECK(chunks == 1);  // candidate 11001 - 1000 = 10001 >= 10000
  CHECK(near(last.core_end, 11001.0));
  CHECK(last.core_start == -kInf);
}

static void test_sorter_chain() {
  Sorter s(small_cfg());
  std::vector<SortedChunk> chunks;
  auto grab = [&](SortedChunk&& c) { chunks.push_back(std::move(c)); };
  std::vector<ScalarHit> b1 = {hit(0, 0.0), hit(0, 20000.0)};
  s.push_batch(std::move(b1), grab);
  CHECK(chunks.size() == 1);
  std::vector<ScalarHit> b2 = {hit(0, 40000.0)};
  s.push_batch(std::move(b2), grab);
  CHECK(chunks.size() == 2);
  CHECK(near(chunks[1].core_start, chunks[0].core_end));  // chain invariant
  // The hit at 20000 sat in chunk 0's tail (past core_end=15000, not emitted);
  // it must reappear in chunk 1 AND fall inside chunk 1's core range.
  CHECK(near(chunks[1].hits.front().timestamp_ns, 20000.0));
  auto r = core_range(chunks[1]);
  CHECK(r.second > r.first && near(chunks[1].hits[r.first].timestamp_ns, 20000.0));
}

static void test_sorter_flush_remainder() {
  Sorter s(small_cfg());
  std::vector<SortedChunk> chunks;
  auto grab = [&](SortedChunk&& c) { chunks.push_back(std::move(c)); };
  std::vector<ScalarHit> b = {hit(0, 0.0), hit(0, 20000.0)};
  s.push_batch(std::move(b), grab);
  CHECK(chunks.size() == 1);
  s.flush(grab);
  CHECK(chunks.size() == 2);
  CHECK(chunks[1].core_end == kInf);
  CHECK(near(chunks[1].core_start, chunks[0].core_end));
  CHECK(s.buffered() == 0);
  s.flush(grab);  // nothing left -> no chunk
  CHECK(chunks.size() == 2);
}

static void test_sorter_reset_between_runs() {
  // R11: after reset(), a new run with SMALL timestamps must emit normally.
  // A stale prev_core_end floor from run 1 would silently swallow run 2.
  Sorter s(small_cfg());
  int chunks = 0;
  auto count = [&](SortedChunk&&) { ++chunks; };
  std::vector<ScalarHit> run1 = {hit(0, 1e9), hit(0, 1e9 + 20000.0)};
  s.push_batch(std::move(run1), count);
  CHECK(chunks == 1);
  s.flush(count);
  CHECK(chunks == 2);

  s.reset();  // RunOpen of the next run — digitizer clock restarted
  SortedChunk c;
  std::vector<ScalarHit> run2 = {hit(0, 100.0), hit(0, 20100.0)};
  s.push_batch(std::move(run2), [&](SortedChunk&& cc) {
    ++chunks;
    c = std::move(cc);
  });
  CHECK(chunks == 3);            // run 2 emits despite tiny timestamps
  CHECK(c.core_start == -kInf);  // floor was reset
  auto r = core_range(c);
  CHECK(r.second > r.first);  // and its core actually contains hits
}

static void test_sorter_counts_hits_behind_core() {
  // A hit older than the last emitted core_end can never enter any core range
  // (H10 floor): it reaches neither the hits tree nor a built event. That is
  // the safe-horizon contract, but crossing it must be COUNTED, never silent:
  // with independent per-board clocks the oscillator drift alone crosses a
  // 50 ms horizon within tens of minutes (pf-qaq ThGEM setup, 2026-10-05).
  Sorter s(small_cfg());
  std::vector<SortedChunk> chunks;
  auto grab = [&](SortedChunk&& c) { chunks.push_back(std::move(c)); };
  std::vector<ScalarHit> b1 = {hit(0, 0.0), hit(0, 20000.0)};
  s.push_batch(std::move(b1), grab);
  CHECK(chunks.size() == 1);
  CHECK(near(chunks[0].core_end, 15000.0));
  CHECK(s.late_hits() == 0);
  // 9000 and 14999 are behind core_end; 15000 is the next core_start (inclusive).
  std::vector<ScalarHit> b2 = {hit(1, 9000.0), hit(1, 14999.0), hit(1, 15000.0),
                               hit(1, 16000.0)};
  s.push_batch(std::move(b2), grab);
  CHECK(s.late_hits() == 2);
  s.flush(grab);
  std::vector<double> emitted;
  for (const SortedChunk& c : chunks) {
    auto r = core_range(c);
    for (std::size_t i = r.first; i < r.second; ++i) emitted.push_back(c.hits[i].timestamp_ns);
  }
  auto has = [&](double t) {
    for (double e : emitted)
      if (near(e, t)) return true;
    return false;
  };
  CHECK(!has(9000.0) && !has(14999.0));  // the counted hits are really gone
  CHECK(has(15000.0) && has(16000.0) && has(20000.0) && has(0.0));
  s.reset();
  CHECK(s.late_hits() == 0);
}

static void test_sorter_late_hit_lands_sorted() {
  // A hit arriving late (but within the safe horizon) must appear in the
  // correct sorted position of the next chunk, not be lost or misplaced.
  Sorter s(small_cfg());
  std::vector<SortedChunk> chunks;
  auto grab = [&](SortedChunk&& c) { chunks.push_back(std::move(c)); };
  std::vector<ScalarHit> b1 = {hit(0, 0.0), hit(0, 12000.0), hit(0, 20000.0)};
  s.push_batch(std::move(b1), grab);
  CHECK(chunks.size() == 1);
  CHECK(near(chunks[0].core_end, 15000.0));
  // 13000 is OLDER than max seen (20000) but younger than core_end - lookback?
  // No: 13000 < 15000 (already-emitted core_end) but >= 14000 lookback? It is
  // 13000 < 14000 — deliberately choose 14500: inside the retained lookback.
  std::vector<ScalarHit> b2 = {hit(0, 14500.0), hit(0, 40000.0)};
  s.push_batch(std::move(b2), grab);
  CHECK(chunks.size() == 2);
  // 14500 must sit sorted between the retained 12000-tail context and 20000.
  bool found = false, sorted = true;
  const auto& h2 = chunks[1].hits;
  for (std::size_t i = 0; i < h2.size(); ++i) {
    if (near(h2[i].timestamp_ns, 14500.0)) found = true;
    if (i > 0 && h2[i - 1].timestamp_ns > h2[i].timestamp_ns) sorted = false;
  }
  CHECK(found && sorted);
  // And it belongs to chunk 2's core: core_start = 15000 ... no — 14500 is
  // BELOW core_start (15000): it was late enough to miss chunk 1's core, and
  // the H10 floor excludes it from chunk 2's core too. That is exactly the
  // safe-horizon contract: 14500 arrived AFTER max_ts had reached 20000, i.e.
  // disorder of 5500 ns > safe_horizon 5000 — out of contract, context-only.
  auto r = core_range(chunks[1]);
  CHECK(h2[r.first].timestamp_ns >= 15000.0);
}

// ---------------------------------------------------------------------------
// Shared helpers (Sorter integration, generic layer)
// ---------------------------------------------------------------------------

// Reorder `hits` into a realistic arrival order: true time plus a bounded
// per-hit displacement. The Sorter's contract only covers disorder up to
// safe_horizon_ns — a full shuffle would exceed it by construction (hits
// arriving later than their chunk's core are context-only, by design), so
// integration tests must model the real, BOUNDED network disorder.
static void jitter_arrival(std::vector<ScalarHit>& hits, double max_disp,
                           std::mt19937& rng) {
  std::uniform_real_distribution<double> disp(-max_disp, max_disp);
  std::vector<std::pair<double, std::size_t>> order;
  order.reserve(hits.size());
  for (std::size_t i = 0; i < hits.size(); ++i)
    order.push_back({hits[i].timestamp_ns + disp(rng), i});
  std::sort(order.begin(), order.end());
  std::vector<ScalarHit> arrival;
  arrival.reserve(hits.size());
  for (const auto& p : order) arrival.push_back(hits[p.second]);
  hits = std::move(arrival);
}

// The side3 channel map: det1 = trig ch1 + arms ch2-5, det2 = trig ch6 +
// arms ch7-10, LaBr3 = ch0.
static SortedChunk mkchunk(std::vector<ScalarHit> hits, double cs = -kInf,
                           double ce = kInf) {
  std::sort(hits.begin(), hits.end(), [](const ScalarHit& a, const ScalarHit& b) {
    return a.timestamp_ns < b.timestamp_ns;
  });
  SortedChunk c;
  c.hits = std::move(hits);
  c.core_start = cs;
  c.core_end = ce;
  return c;
}

// ---------------------------------------------------------------------------
// Time offsets upstream of the Sorter (TODO 70)
// ---------------------------------------------------------------------------

static void test_sorter_offset_spread_beyond_horizon_counts_late() {
  // Why reload_offsets rejects spread >= safe_horizon: aligning shifts ch2 by
  // 20 us while the Sorter only waits 5 us, so aligned ch2 hits land behind
  // chunk cores that were already emitted -> counted late (= missing from the
  // .root). Spread far below the horizon (tens of ns of cable) is harmless.
  OffsetTable t;
  t.format = "tree";
  t.abs_ns[mc_key(0, 1)] = 0.0;
  t.abs_ns[mc_key(0, 2)] = 20000.0;
  Sorter s(small_cfg());  // horizon 5000
  std::set<uint16_t> reported;
  for (int i = 0; i < 100; i += 5) {
    std::vector<ScalarHit> batch;
    for (int k = i; k < i + 5; ++k) {
      batch.push_back(hit(1, 1000.0 * k));
      batch.push_back(hit(2, 1000.0 * k));  // raw: simultaneous
    }
    apply_time_offsets(batch, t, reported, [](uint16_t) {});
    s.push_batch(std::move(batch), [](SortedChunk&&) {});
  }
  CHECK(s.late_hits() > 0);
}

// ---------------------------------------------------------------------------
// Generic event layer (TODO 71 Phase A): TriggerGrouper + build_generic_events
// ---------------------------------------------------------------------------

static ScalarHit mhit(int mod, int ch, double t, uint16_t e = 0) {
  ScalarHit h = hit(ch, t, e);
  h.module = static_cast<uint8_t>(mod);
  return h;
}

struct TrigSpec {
  const char* name;
  int priority;
  std::vector<std::pair<int, int>> channels;  // (module, channel)
};

static std::shared_ptr<const EventDef> mkdef(double pre, double post, double group,
                                             const std::vector<TrigSpec>& trigs,
                                             bool priorities = true) {
  auto d = std::make_shared<EventDef>();
  d->pre_ns = pre;
  d->post_ns = post;
  d->group_ns = group;
  d->priorities_given = priorities;
  for (const TrigSpec& t : trigs) {
    TriggerDef td;
    td.name = t.name;
    td.priority = priorities ? t.priority : 0;
    for (const auto& mc : t.channels)
      td.channels.push_back(mc_key(static_cast<uint8_t>(mc.first),
                                   static_cast<uint8_t>(mc.second)));
    d->triggers.push_back(td);
  }
  d->index_channels();
  return d;
}

// ThGEM-like: ThGEM1 = m0c1 (priority 1), ThGEM2 = m0c6 (priority 2).
static std::shared_ptr<const EventDef> thgem_def(double pre = 1000, double post = 1000,
                                                 double group = 1000) {
  return mkdef(pre, post, group, {{"ThGEM1", 1, {{0, 1}}}, {"ThGEM2", 2, {{0, 6}}}});
}

// The whole hit set as ONE chunk (core = everything): grouping + assignment
// with no chunk boundary in play.
static std::vector<GenericEvent> one_chunk(const std::vector<ScalarHit>& hits,
                                           std::shared_ptr<const EventDef> def,
                                           GenericStats& st) {
  TriggerGrouper g;
  g.reset(def);
  SortedChunk c = mkchunk(hits);
  EventPlan p = g.plan_chunk(c);
  return build_generic_events(c, p, st);
}

// Find the hit of (ch) in an event; nullptr when absent.
static const EventHit* find_ch(const GenericEvent& ev, int ch) {
  for (const EventHit& h : ev.hits)
    if (h.ch == ch) return &h;
  return nullptr;
}

static void test_generic_disabled() {
  TriggerGrouper g;
  g.reset(nullptr);
  CHECK(!g.enabled());
  SortedChunk c = mkchunk({hit(1, 100.0)});
  EventPlan p = g.plan_chunk(c);
  CHECK(p.def == nullptr);
  CHECK(p.refs.empty());
}

static void test_generic_single_event() {
  // One trigger + two partners inside the window + one hit far away.
  GenericStats st;
  auto evs = one_chunk({hit(1, 10000.0, 7), hit(2, 10030.0, 8), hit(0, 9900.0, 9),
                        hit(3, 50000.0, 1)},
                       thgem_def(), st);
  CHECK(evs.size() == 1);
  if (evs.size() != 1) return;
  const GenericEvent& e = evs[0];
  CHECK(near(e.t, 10000.0));
  CHECK(e.mod == 0 && e.ch == 1 && e.trig_idx == 0 && e.n_triggers == 1);
  CHECK(e.hits.size() == 3);
  // Time order, reference included, dt relative to the reference.
  CHECK(e.hits.size() == 3 && e.hits[0].ch == 0 && near(e.hits[0].dt, -100.0));
  CHECK(e.hits.size() == 3 && e.hits[1].ch == 1 && near(e.hits[1].dt, 0.0) &&
        e.hits[1].is_trigger && e.hits[1].energy == 7);
  CHECK(e.hits.size() == 3 && e.hits[2].ch == 2 && near(e.hits[2].dt, 30.0) &&
        !e.hits[2].is_trigger);
  CHECK(st.events == 1 && st.hits_in_events == 3 && st.orphans == 1 && st.contested == 0);
  CHECK(st.ntrig[0] == 1);
}

static void test_generic_group_does_not_extend() {
  // Rule 1: triggers 800 ns apart with G = 1000 — the group [0, 1000] takes
  // 0 and 800, and 1600 opens the next group (no extension to 1800).
  GenericStats st;
  auto evs = one_chunk({hit(1, 0.0), hit(1, 800.0), hit(1, 1600.0)}, thgem_def(), st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(near(evs[0].t, 0.0) && evs[0].n_triggers == 2);
  CHECK(near(evs[1].t, 1600.0) && evs[1].n_triggers == 1);
}

static void test_generic_abc_chain_no_loss() {
  // ELIFANT's ID-priority rule loses C in A->B->C at 0.8 W spacing. Here every
  // trigger lands in exactly one event.
  GenericStats st;
  std::vector<ScalarHit> hits;
  for (int i = 0; i < 10; ++i) hits.push_back(hit(1, 1000.0 + 800.0 * i, static_cast<uint16_t>(i)));
  auto evs = one_chunk(hits, thgem_def(), st);
  uint32_t ntrig = 0;
  std::size_t trig_hits = 0;
  for (const auto& e : evs) {
    ntrig += e.n_triggers;
    for (const auto& h : e.hits) trig_hits += h.is_trigger ? 1 : 0;
  }
  CHECK(ntrig == 10);
  CHECK(trig_hits == 10);
  CHECK(st.orphans == 0);
}

static void test_generic_priority_picks_reference() {
  // ThGEM2 (p2) fires first, ThGEM1 (p1) 300 ns later in the same group: the
  // reference is ThGEM1, and the ThGEM2 trigger sits at dt = -300.
  GenericStats st;
  auto evs = one_chunk({hit(6, 5000.0), hit(1, 5300.0), hit(2, 5400.0)}, thgem_def(), st);
  CHECK(evs.size() == 1);
  if (evs.size() != 1) return;
  CHECK(near(evs[0].t, 5300.0) && evs[0].ch == 1 && evs[0].trig_idx == 0);
  CHECK(evs[0].n_triggers == 2);
  const EventHit* t2 = find_ch(evs[0], 6);
  CHECK(t2 && near(t2->dt, -300.0) && t2->is_trigger);
}

static void test_generic_priority_ties() {
  // Same priority -> earliest; same priority and same time -> smallest
  // (module, channel). Trigger A = m0c1, B = m1c0, both priority 1.
  auto def = mkdef(1000, 1000, 1000, {{"A", 1, {{0, 1}}}, {"B", 1, {{1, 0}}}});
  {
    GenericStats st;
    auto evs = one_chunk({mhit(1, 0, 100.0), mhit(0, 1, 200.0)}, def, st);
    CHECK(evs.size() == 1 && near(evs[0].t, 100.0) && evs[0].mod == 1 && evs[0].ch == 0 &&
          evs[0].trig_idx == 1);
  }
  {
    GenericStats st;
    auto evs = one_chunk({mhit(1, 0, 100.0), mhit(0, 1, 100.0)}, def, st);
    CHECK(evs.size() == 1 && evs[0].mod == 0 && evs[0].ch == 1 && evs[0].trig_idx == 0);
  }
}

static void test_generic_no_priorities_first_trigger() {
  auto def = mkdef(1000, 1000, 1000, {{"A", 0, {{0, 1}}}, {"B", 0, {{0, 6}}}}, false);
  GenericStats st;
  auto evs = one_chunk({hit(6, 100.0), hit(1, 150.0)}, def, st);
  CHECK(evs.size() == 1 && evs[0].ch == 6 && near(evs[0].t, 100.0));
}

static void test_generic_nearest_reference_wins() {
  // Two separate groups (refs 0 and 1500, windows ±1000 overlap on
  // [500, 1000]). A hit at 800 is 800 from ref 0 and 700 from ref 1500 ->
  // second event. A hit at 750 is equidistant -> the EARLIER reference.
  GenericStats st;
  auto evs = one_chunk({hit(1, 0.0), hit(1, 1500.0), hit(2, 800.0), hit(3, 750.0)},
                       thgem_def(), st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(find_ch(evs[1], 2) && near(find_ch(evs[1], 2)->dt, -700.0));
  CHECK(!find_ch(evs[0], 2));
  CHECK(find_ch(evs[0], 3) && near(find_ch(evs[0], 3)->dt, 750.0));
  CHECK(!find_ch(evs[1], 3));
  CHECK(st.contested == 2);
  CHECK(st.hits_in_events == 4 && st.orphans == 0);
}

static void test_generic_trigger_follows_its_group() {
  // Group 1 = {0 (ThGEM1, p1), 1000 (ThGEM2, p2)} -> ref 0. Group 2 opens at
  // 1001 (ThGEM1) -> ref 1001. The ThGEM2 trigger at 1000 is 1 ns from ref
  // 1001 but belongs to group 1 (rule 1 owns triggers), at dt = +1000 = post.
  GenericStats st;
  auto evs = one_chunk({hit(1, 0.0), hit(6, 1000.0), hit(1, 1001.0)}, thgem_def(), st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(evs[0].n_triggers == 2 && evs[0].hits.size() == 2);
  const EventHit* t2 = find_ch(evs[0], 6);
  CHECK(t2 && near(t2->dt, 1000.0) && t2->is_trigger);
  CHECK(evs[1].n_triggers == 1 && evs[1].hits.size() == 1);
  CHECK(st.contested == 0);
}

static void test_generic_window_edges_inclusive() {
  // [t_ref - pre, t_ref + post] is inclusive on both ends; asymmetric window.
  auto def = thgem_def(200.0, 1000.0, 200.0);
  GenericStats st;
  auto evs = one_chunk({hit(1, 10000.0), hit(2, 9800.0), hit(3, 9799.0), hit(4, 11000.0),
                        hit(5, 11001.0)},
                       def, st);
  CHECK(evs.size() == 1);
  if (evs.size() != 1) return;
  CHECK(find_ch(evs[0], 2) && near(find_ch(evs[0], 2)->dt, -200.0));
  CHECK(!find_ch(evs[0], 3));
  CHECK(find_ch(evs[0], 4) && near(find_ch(evs[0], 4)->dt, 1000.0));
  CHECK(!find_ch(evs[0], 5));
  CHECK(st.orphans == 2);
}

static void test_generic_asymmetric_nearest_out_of_window() {
  // pre = 100, post = 1000. Refs at 0 and 1300. A hit at 950 is NEARER to
  // 1300 (350 before it) but outside that window's pre side -> it goes to ref
  // 0 (950 after it, inside post). Not contested: only one window holds it.
  auto def = thgem_def(100.0, 1000.0, 100.0);
  GenericStats st;
  auto evs = one_chunk({hit(1, 0.0), hit(1, 1300.0), hit(2, 950.0)}, def, st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(find_ch(evs[0], 2) && near(find_ch(evs[0], 2)->dt, 950.0));
  CHECK(st.contested == 0);
}

static void test_generic_contested_counts_any_two_windows() {
  // References can sit closer than G: group 1 = {0 (p2), 1000 (p1)} -> ref
  // 1000; group 2 = {1000.5}; group 3 = {2800}. A hit at 1700 is in the
  // windows of 1000 and 1000.5 only (2800 is 1100 away > pre): contested via
  // the SECOND predecessor, not the pred/succ pair -> nearest 1000.5. A hit at
  // 2000 is in all three windows (1000, 999.5, -800) -> nearest 2800.
  GenericStats st;
  auto evs = one_chunk({hit(6, 0.0), hit(1, 1000.0), hit(1, 1000.5), hit(1, 2800.0),
                        hit(2, 1700.0), hit(3, 2000.0)},
                       thgem_def(), st);
  CHECK(evs.size() == 3);
  if (evs.size() != 3) return;
  CHECK(near(evs[0].t, 1000.0) && near(evs[1].t, 1000.5) && near(evs[2].t, 2800.0));
  CHECK(find_ch(evs[1], 2) != nullptr);
  CHECK(find_ch(evs[2], 3) != nullptr);
  CHECK(st.contested == 2);
}

static void test_generic_group_zero() {
  // G = 0: only triggers at exactly the same time share a group.
  auto def = thgem_def(500.0, 500.0, 0.0);
  GenericStats st;
  auto evs = one_chunk({hit(1, 100.0), hit(6, 100.0), hit(1, 100.5)}, def, st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(evs[0].n_triggers == 2 && evs[0].ch == 1);
  CHECK(evs[1].n_triggers == 1);
}

static void test_generic_ntrig_histogram() {
  GenericStats st;
  std::vector<ScalarHit> hits;
  // 1 group of 9 triggers (bin 8+), then 1 single.
  for (int i = 0; i < 9; ++i) hits.push_back(hit(1, 100.0 + i * 10.0));
  hits.push_back(hit(1, 100000.0));
  auto evs = one_chunk(hits, thgem_def(), st);
  CHECK(evs.size() == 2);
  CHECK(st.ntrig[7] == 1 && st.ntrig[0] == 1);
  CHECK(st.events == 2);
}

static void test_generic_reset_between_runs() {
  // A stale ev_lo / open group from run N must not leak into run N+1 (whose
  // clock restarted near 0).
  TriggerGrouper g;
  auto def = thgem_def();
  g.reset(def);
  SortedChunk c1 = mkchunk({hit(1, 5e9), hit(2, 5e9 + 10)}, -kInf, 5e9 + 5);  // open group
  GenericStats st;
  build_generic_events(c1, g.plan_chunk(c1), st);
  g.reset(def);
  SortedChunk c2 = mkchunk({hit(1, 100.0), hit(2, 130.0)});
  GenericStats st2;
  auto evs = build_generic_events(c2, g.plan_chunk(c2), st2);
  CHECK(evs.size() == 1 && near(evs[0].t, 100.0) && evs[0].hits.size() == 2);
}

static void test_sorter_set_lookback() {
  Sorter s(small_cfg());
  CHECK(near(s.lookback_ns(), 1000.0));
  s.set_lookback_ns(4000.0);
  CHECK(near(s.lookback_ns(), 4000.0));
  std::vector<SortedChunk> out;
  std::vector<ScalarHit> b;
  for (int i = 0; i <= 20; ++i) b.push_back(hit(0, i * 1000.0));
  s.push_batch(std::move(b), [&](SortedChunk&& c) { out.push_back(std::move(c)); });
  CHECK(out.size() == 1);
  // core_end = 20000 - 5000 = 15000; retained from 15000 - 4000 = 11000.
  CHECK(s.buffered() == 10);
}

// --- Brute-force reference: rules 1-4 straight from TODO 71 §4, no chunks ---

struct RefResult {
  std::vector<GenericEvent> events;
  uint64_t orphans = 0, contested = 0;
  uint64_t ac_flagged = 0, l2_accepted = 0;
  std::vector<uint64_t> flag_true, accept_true;
};

static bool in_list(const std::vector<uint16_t>& v, uint16_t k) {
  return std::find(v.begin(), v.end(), k) != v.end();
}

static bool hit_order(const ScalarHit& a, const ScalarHit& b) {
  if (a.timestamp_ns != b.timestamp_ns) return a.timestamp_ns < b.timestamp_ns;
  if (a.module != b.module) return a.module < b.module;
  return a.channel < b.channel;
}

static RefResult reference_build(std::vector<ScalarHit> hits, const EventDef& d) {
  std::sort(hits.begin(), hits.end(), hit_order);
  RefResult r;
  // Rule 1: non-extending groups over the time-ordered triggers.
  std::vector<std::vector<std::size_t>> groups;  // indices into hits
  double start = 0.0;
  for (std::size_t i = 0; i < hits.size(); ++i) {
    if (d.trigger_index(hits[i].module, hits[i].channel) < 0) continue;
    if (groups.empty() || hits[i].timestamp_ns > start + d.group_ns) {
      groups.push_back({});
      start = hits[i].timestamp_ns;
    }
    groups.back().push_back(i);
  }
  // Rule 2: reference = best (priority, time, module, channel).
  std::vector<std::size_t> refs;
  for (const auto& g : groups) {
    std::size_t best = g[0];
    for (std::size_t i : g) {
      const ScalarHit& a = hits[i];
      const ScalarHit& b = hits[best];
      int pa = d.triggers[d.trigger_index(a.module, a.channel)].priority;
      int pb = d.triggers[d.trigger_index(b.module, b.channel)].priority;
      if (pa < pb || (pa == pb && hit_order(a, b))) best = i;
    }
    refs.push_back(best);
    GenericEvent e;
    e.t = hits[best].timestamp_ns;
    e.mod = hits[best].module;
    e.ch = hits[best].channel;
    e.trig_idx = static_cast<uint8_t>(d.trigger_index(e.mod, e.ch));
    e.n_triggers = static_cast<uint32_t>(g.size());
    r.events.push_back(e);
  }
  std::vector<int> group_of(hits.size(), -1);
  for (std::size_t k = 0; k < groups.size(); ++k)
    for (std::size_t i : groups[k]) group_of[i] = static_cast<int>(k);
  // Rules 3-4: triggers follow their group; other hits -> nearest window.
  for (std::size_t i = 0; i < hits.size(); ++i) {
    const ScalarHit& h = hits[i];
    int ev = group_of[i];
    if (ev < 0) {
      int n_in = 0;
      double best_d = 0.0;
      for (std::size_t k = 0; k < refs.size(); ++k) {
        double dt = h.timestamp_ns - r.events[k].t;
        if (dt < -d.pre_ns || dt > d.post_ns) continue;
        ++n_in;
        double ad = dt < 0 ? -dt : dt;
        if (ev < 0 || ad < best_d) {  // strict: ties keep the earlier ref
          ev = static_cast<int>(k);
          best_d = ad;
        }
      }
      if (n_in >= 2) ++r.contested;
      if (ev < 0) {
        ++r.orphans;
        continue;
      }
    }
    EventHit eh;
    eh.mod = h.module;
    eh.ch = h.channel;
    eh.energy = h.energy;
    eh.energy_short = h.energy_short;
    eh.dt = h.timestamp_ns - r.events[ev].t;
    eh.is_trigger = group_of[i] >= 0;
    // AC (Phase C): any shield hit of a pair this channel is a detector of,
    // within [t - ac_pre, t + ac_post] of THIS hit — over all hits, events
    // play no part.
    const uint16_t key = mc_key(h.module, h.channel);
    for (const AcPair& p : d.ac_pairs) {
      if (!in_list(p.detector, key)) continue;
      for (const ScalarHit& o : hits) {
        double dt = o.timestamp_ns - h.timestamp_ns;
        if (in_list(p.shield, mc_key(o.module, o.channel)) && dt >= -d.ac_pre_ns &&
            dt <= d.ac_post_ns)
          eh.with_ac = true;
      }
    }
    if (eh.with_ac) ++r.ac_flagged;
    r.events[ev].hits.push_back(eh);
  }
  // L2 (Phase C), straight from the definition's lists (no bit masks).
  r.flag_true.assign(d.flags.size(), 0);
  r.accept_true.assign(d.accepts.size(), 0);
  for (GenericEvent& e : r.events) {
    if (d.counters.empty()) continue;
    e.l2_counts.assign(d.counters.size(), 0);
    for (std::size_t c = 0; c < d.counters.size(); ++c)
      for (const EventHit& h : e.hits) {
        if (!in_list(d.counters[c].channels, mc_key(h.mod, h.ch))) continue;
        L2Ac m = d.counters[c].ac;
        if (m == L2Ac::Any || (m == L2Ac::Clean && !h.with_ac) || (m == L2Ac::Vetoed && h.with_ac))
          ++e.l2_counts[c];
      }
    for (std::size_t f = 0; f < d.flags.size(); ++f) {
      long long v = e.l2_counts[static_cast<std::size_t>(d.flags[f].counter)];
      long long x = d.flags[f].value;
      bool b = false;
      switch (d.flags[f].op) {
        case L2Op::Eq: b = v == x; break;
        case L2Op::Ne: b = v != x; break;
        case L2Op::Lt: b = v < x; break;
        case L2Op::Le: b = v <= x; break;
        case L2Op::Gt: b = v > x; break;
        case L2Op::Ge: b = v >= x; break;
      }
      if (b) {
        e.l2_flags |= uint64_t(1) << f;
        ++r.flag_true[f];
      }
    }
    for (std::size_t a = 0; a < d.accepts.size(); ++a) {
      const L2Accept& acc = d.accepts[a];
      bool b = acc.is_and;
      for (int f : acc.flags) {
        bool fb = (e.l2_flags >> f) & 1;
        b = acc.is_and ? (b && fb) : (b || fb);
      }
      if (b) {
        e.l2_accepts |= uint64_t(1) << a;
        ++r.accept_true[a];
        e.l2_accept = true;
      }
    }
    if (e.l2_accept) ++r.l2_accepted;
  }
  return r;
}

// Hits inside an event compared as a canonical multiset (equal timestamps may
// come out of std::sort in either order).
static std::vector<EventHit> canon(std::vector<EventHit> v) {
  std::sort(v.begin(), v.end(), [](const EventHit& a, const EventHit& b) {
    if (a.dt != b.dt) return a.dt < b.dt;
    if (a.mod != b.mod) return a.mod < b.mod;
    if (a.ch != b.ch) return a.ch < b.ch;
    return a.energy < b.energy;
  });
  return v;
}

static bool same_events(const std::vector<GenericEvent>& a, const std::vector<GenericEvent>& b,
                        std::string& why) {
  if (a.size() != b.size()) {
    why = "event count " + std::to_string(a.size()) + " vs " + std::to_string(b.size());
    return false;
  }
  for (std::size_t i = 0; i < a.size(); ++i) {
    const GenericEvent& x = a[i];
    const GenericEvent& y = b[i];
    if (x.t != y.t || x.mod != y.mod || x.ch != y.ch || x.trig_idx != y.trig_idx ||
        x.n_triggers != y.n_triggers || x.hits.size() != y.hits.size()) {
      char buf[160];
      std::snprintf(buf, sizeof buf, "event %zu: t %.3f/%.3f ntrig %u/%u nhits %zu/%zu", i, x.t,
                    y.t, x.n_triggers, y.n_triggers, x.hits.size(), y.hits.size());
      why = buf;
      return false;
    }
    auto cx = canon(x.hits), cy = canon(y.hits);
    for (std::size_t j = 0; j < cx.size(); ++j)
      if (cx[j].dt != cy[j].dt || cx[j].mod != cy[j].mod || cx[j].ch != cy[j].ch ||
          cx[j].energy != cy[j].energy || cx[j].is_trigger != cy[j].is_trigger ||
          cx[j].with_ac != cy[j].with_ac) {
        why = "event " + std::to_string(i) + " hit " + std::to_string(j) + " differs" +
              (cx[j].with_ac != cy[j].with_ac ? " (with_ac)" : "");
        return false;
      }
    if (x.l2_counts != y.l2_counts || x.l2_flags != y.l2_flags ||
        x.l2_accepts != y.l2_accepts || x.l2_accept != y.l2_accept) {
      why = "event " + std::to_string(i) + " L2 differs";
      return false;
    }
  }
  return true;
}

// The production path: bounded-disorder arrival -> Sorter (lookback from the
// definition) -> TriggerGrouper on every chunk in order -> builder per chunk.
struct PipeResult {
  std::vector<GenericEvent> events;
  GenericStats stats;
  uint64_t core_hits = 0;
  std::size_t chunks = 0;
  uint64_t late = 0;
};

static PipeResult run_generic_pipeline(std::vector<ScalarHit> arrival,
                                       std::shared_ptr<const EventDef> def,
                                       Sorter::Config scfg, std::size_t batch) {
  scfg.lookback_ns = std::max(scfg.lookback_ns, def->lookback_ns());
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  PipeResult r;
  auto on_chunk = [&](SortedChunk&& c) {
    EventPlan p = g.plan_chunk(c);
    auto cr = core_range(c);
    r.core_hits += cr.second - cr.first;
    GenericStats st;
    auto evs = build_generic_events(c, p, st);
    r.stats.add(st);
    for (auto& e : evs) r.events.push_back(std::move(e));
    ++r.chunks;
  };
  std::size_t fed = 0;
  while (fed < arrival.size()) {
    std::vector<ScalarHit> b;
    for (std::size_t i = 0; i < batch && fed < arrival.size(); ++i, ++fed)
      b.push_back(arrival[fed]);
    s.push_batch(std::move(b), on_chunk);
  }
  s.flush(on_chunk);
  r.late = s.late_hits();
  return r;
}

static void test_generic_matches_reference_randomized() {
  // Many random scenarios: overlapping bursts, ties on integer times, three
  // trigger kinds over two modules (one sharing a priority), asymmetric
  // windows, chunk spans comparable to the window, bounded arrival disorder.
  // The chunked pipeline must reproduce the no-chunk reference EXACTLY, and
  // hits_in_events + orphans must equal the hits written to the core ranges.
  int mismatches = 0, scenarios = 0;
  std::string first_why;
  for (int seed = 1; seed <= 200; ++seed) {
    std::mt19937 rng(static_cast<unsigned>(seed));
    std::uniform_int_distribution<int> u(0, 1000000);
    double pre = 100.0 + (u(rng) % 1500);
    double post = 100.0 + (u(rng) % 1500);
    double group = (u(rng) % 4 == 0) ? 0.0 : static_cast<double>(u(rng) % static_cast<int>(std::min(pre, post) + 1));
    auto def = mkdef(pre, post, group,
                     {{"A", 1, {{0, 1}}}, {"B", 2, {{0, 6}, {1, 2}}}, {"C", 1, {{1, 0}}}});
    std::vector<ScalarHit> hits;
    double t = 1000.0;
    int nburst = 40 + u(rng) % 60;
    for (int b = 0; b < nburst; ++b) {
      t += static_cast<double>(u(rng) % 4000);  // bursts often overlap windows
      int n = 1 + u(rng) % 8;
      for (int k = 0; k < n; ++k) {
        int mod = u(rng) % 2;
        int ch = u(rng) % 8;
        double ht = t + static_cast<double>(u(rng) % 2400) - 600.0;
        if (u(rng) % 3 == 0) ht = std::floor(ht / 50.0) * 50.0;  // force exact ties
        if (ht < 0) ht = 0;
        hits.push_back(mhit(mod, ch, ht, static_cast<uint16_t>(u(rng) & 0xffff)));
      }
    }
    RefResult ref = reference_build(hits, *def);
    Sorter::Config scfg;
    scfg.chunk_span_ns = 2000.0 + (u(rng) % 20000);
    scfg.safe_horizon_ns = 3000.0 + (u(rng) % 3000);
    scfg.lookback_ns = 0.0;
    std::vector<ScalarHit> arrival = hits;
    std::sort(arrival.begin(), arrival.end(), hit_order);
    jitter_arrival(arrival, scfg.safe_horizon_ns * 0.4, rng);
    PipeResult got = run_generic_pipeline(arrival, def, scfg, 1 + u(rng) % 25);
    ++scenarios;
    std::string why;
    bool ok = got.late == 0 && same_events(got.events, ref.events, why) &&
              got.stats.orphans == ref.orphans && got.stats.contested == ref.contested &&
              got.stats.hits_in_events + got.stats.orphans == got.core_hits &&
              got.core_hits == hits.size() && got.stats.events == ref.events.size();
    if (!ok) {
      if (mismatches == 0) {
        char buf[256];
        std::snprintf(buf, sizeof buf,
                      "seed %d (pre %g post %g G %g, %zu chunks, late %llu): %s | orphans %llu/%llu "
                      "contested %llu/%llu in+orph %llu core %llu",
                      seed, pre, post, group, got.chunks,
                      static_cast<unsigned long long>(got.late), why.c_str(),
                      static_cast<unsigned long long>(got.stats.orphans),
                      static_cast<unsigned long long>(ref.orphans),
                      static_cast<unsigned long long>(got.stats.contested),
                      static_cast<unsigned long long>(ref.contested),
                      static_cast<unsigned long long>(got.stats.hits_in_events + got.stats.orphans),
                      static_cast<unsigned long long>(got.core_hits));
        first_why = buf;
      }
      ++mismatches;
    }
  }
  if (mismatches) std::printf("  randomized: %d/%d mismatch, first: %s\n", mismatches, scenarios,
                              first_why.c_str());
  CHECK(mismatches == 0);
}

static void test_generic_boundary_reference_after_core_end() {
  // Directed H10-style case. Sorter span 10 us / horizon 5 us: the first batch
  // (hits up to 20000) splits at core_end = 15000. A group opens at 14990
  // (ThGEM2, p2) and its reference ThGEM1 at 15200 lies BEYOND core_end; arms
  // sit on both sides. The event must come out exactly once, complete.
  auto def = thgem_def();
  Sorter::Config scfg = small_cfg();
  std::vector<ScalarHit> b1;
  for (int i = 0; i <= 20; ++i) b1.push_back(hit(3, i * 1000.0 + 500.0));  // filler
  b1.push_back(hit(6, 14990.0));
  b1.push_back(hit(2, 14800.0, 11));
  b1.push_back(hit(1, 15200.0));
  b1.push_back(hit(4, 15900.0, 12));
  b1.push_back(hit(0, 20000.0));
  std::vector<ScalarHit> b2;
  for (int i = 21; i <= 40; ++i) b2.push_back(hit(3, i * 1000.0 + 500.0));
  std::vector<ScalarHit> all = b1;
  all.insert(all.end(), b2.begin(), b2.end());
  RefResult ref = reference_build(all, *def);

  scfg.lookback_ns = std::max(scfg.lookback_ns, def->lookback_ns());
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  std::vector<GenericEvent> evs;
  GenericStats st;
  std::size_t chunks = 0;
  auto on_chunk = [&](SortedChunk&& c) {
    ++chunks;
    EventPlan p = g.plan_chunk(c);
    for (auto& e : build_generic_events(c, p, st)) evs.push_back(std::move(e));
  };
  s.push_batch(std::move(b1), on_chunk);
  s.push_batch(std::move(b2), on_chunk);
  s.flush(on_chunk);
  CHECK(chunks >= 2);
  CHECK(evs.size() == 1);
  if (evs.size() != 1) return;
  CHECK(near(evs[0].t, 15200.0) && evs[0].n_triggers == 2);
  CHECK(find_ch(evs[0], 2) && find_ch(evs[0], 2)->energy == 11);
  CHECK(find_ch(evs[0], 4) && find_ch(evs[0], 4)->energy == 12);
  CHECK(find_ch(evs[0], 6) && near(find_ch(evs[0], 6)->dt, -210.0));
  std::string why;
  CHECK(same_events(evs, ref.events, why));
  CHECK(st.orphans == ref.orphans);
}

static void test_generic_group_span_ends_at_core_end() {
  // The group [14000, 15000] ends EXACTLY at the first chunk's core_end
  // (20000 - 5000): the trigger at 15000 belongs to it (t <= start + G) but is
  // only fed with the NEXT chunk's core. The group must stay open across the
  // boundary (close only when start + G < core_end, strictly).
  auto def = thgem_def();
  Sorter::Config scfg = small_cfg();
  scfg.lookback_ns = def->lookback_ns();
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  std::vector<GenericEvent> evs;
  GenericStats st;
  std::size_t chunks = 0;
  auto on_chunk = [&](SortedChunk&& c) {
    ++chunks;
    for (auto& e : build_generic_events(c, g.plan_chunk(c), st)) evs.push_back(std::move(e));
  };
  s.push_batch({hit(3, 0.0), hit(1, 14000.0), hit(6, 15000.0), hit(3, 20000.0)}, on_chunk);
  s.push_batch({hit(3, 40000.0)}, on_chunk);
  s.flush(on_chunk);
  CHECK(chunks >= 2);
  CHECK(evs.size() == 1);
  CHECK(evs.size() == 1 && evs[0].n_triggers == 2 && near(evs[0].t, 14000.0));
}

static void test_sorter_orders_on_aligned_time() {
  // The Receiver aligns every batch BEFORE the Sorter, so the Sorter, its core
  // ranges and the event layer all see one aligned axis. ch2 runs 30 ns late
  // (offset +30): raw ch2 sorts 20 ns AFTER its trigger, aligned 10 ns BEFORE.
  // Each event's ch2 arrives one batch after its trigger (real arrival disorder).
  OffsetTable t;
  t.format = "tree";
  t.abs_ns[mc_key(0, 1)] = 0.0;
  t.abs_ns[mc_key(0, 2)] = 30.0;
  auto def = mkdef(500.0, 500.0, 500.0, {{"T", 1, {{0, 1}}}});
  Sorter::Config scfg;
  scfg.chunk_span_ns = 10000.0;
  scfg.safe_horizon_ns = 5000.0;
  scfg.lookback_ns = def->lookback_ns();
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  std::vector<SortedChunk> chunks;
  std::vector<GenericEvent> events;
  GenericStats st;
  auto grab = [&](SortedChunk&& c) {
    for (auto& e : build_generic_events(c, g.plan_chunk(c), st)) events.push_back(std::move(e));
    chunks.push_back(std::move(c));
  };
  std::set<uint16_t> reported;
  int missing = 0;
  const int n = 20;
  for (int i = 0; i <= n; ++i) {
    std::vector<ScalarHit> batch;
    if (i < n) batch.push_back(hit(1, 1000.0 + 3000.0 * i, static_cast<uint16_t>(i)));
    if (i > 0) batch.push_back(hit(2, 1000.0 + 3000.0 * (i - 1) + 20.0, 1));
    apply_time_offsets(batch, t, reported, [&](uint16_t) { ++missing; });
    s.push_batch(std::move(batch), grab);
  }
  s.flush(grab);
  CHECK(missing == 0);
  std::vector<ScalarHit> emitted;
  for (const SortedChunk& c : chunks) {
    auto r = core_range(c);
    for (std::size_t i = r.first; i < r.second; ++i) emitted.push_back(c.hits[i]);
  }
  CHECK(emitted.size() == 2u * n);  // each hit exactly once
  bool monotone = true, paired = true;
  for (std::size_t i = 1; i < emitted.size(); ++i)
    if (emitted[i].timestamp_ns < emitted[i - 1].timestamp_ns) monotone = false;
  for (int i = 0; i < n && emitted.size() == 2u * n; ++i) {
    const ScalarHit& a = emitted[2 * i];
    const ScalarHit& b = emitted[2 * i + 1];
    double T = 1000.0 + 3000.0 * i;
    if (!(a.channel == 2 && near(a.timestamp_ns, T - 10.0) && b.channel == 1 &&
          near(b.timestamp_ns, T)))
      paired = false;
  }
  CHECK(monotone);
  CHECK(paired);
  CHECK(s.late_hits() == 0);
  CHECK(events.size() == static_cast<std::size_t>(n));
  bool aligned_dt = true;
  for (const GenericEvent& e : events) {
    const EventHit* h = find_ch(e, 2);
    if (!h || !near(h->dt, -10.0)) aligned_dt = false;  // raw would be +20
  }
  CHECK(aligned_dt);
}

// --- Phase C: AC flag + L2 ---

// Copy a definition, let `f` add Phase C sections, rebuild the lookups.
template <class F>
static std::shared_ptr<const EventDef> with_c(std::shared_ptr<const EventDef> base, F f) {
  auto d = std::make_shared<EventDef>(*base);
  f(*d);
  d->index_channels();
  return d;
}

static uint16_t K(int m, int c) { return mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c)); }

// ThGEM-like def + "Ge" = m0c3 (detector), "BGO" = m0c7 (shield), window
// [-300, +200] around the flagged hit.
static std::shared_ptr<const EventDef> ac_def() {
  return with_c(thgem_def(), [](EventDef& d) {
    d.ac_pre_ns = 300;
    d.ac_post_ns = 200;
    AcPair p;
    p.name = "Ge";
    p.detector = {K(0, 3)};
    p.shield = {K(0, 7)};
    d.ac_pairs.push_back(p);
  });
}

static void test_generic_ac_window_edges() {
  // Shield hits at exactly -300 and +200 of a Ge hit flag it (inclusive);
  // -300.5 / +200.5 do not. The shield hit itself is never flagged.
  auto def = ac_def();
  CHECK(near(def->lookback_ns(), 4000.0 + 500.0));
  struct Case { double bgo; bool flagged; };
  for (Case cs : {Case{-300.0, true}, Case{200.0, true}, Case{-300.5, false},
                  Case{200.5, false}, Case{0.0, true}}) {
    GenericStats st;
    auto evs = one_chunk({hit(1, 1000.0), hit(3, 1100.0), hit(7, 1100.0 + cs.bgo)}, def, st);
    CHECK(evs.size() == 1);
    if (evs.size() != 1) continue;
    const EventHit* ge = find_ch(evs[0], 3);
    const EventHit* bgo = find_ch(evs[0], 7);
    CHECK(ge && ge->with_ac == cs.flagged);
    CHECK(bgo && !bgo->with_ac);
    CHECK(st.ac_flagged == (cs.flagged ? 1u : 0u));
  }
}

static void test_generic_ac_partner_in_other_event_or_none() {
  // The Ge hit (ch3, 600) sits in event 1 (ref 0); its BGO partner 180 ns
  // later (780) is nearer to event 2's reference (1500) and is assigned THERE
  // (rule 4) — still flags. A second Ge in event 2 whose BGO lies outside every window (an
  // orphan) is flagged too: AC is independent of events.
  auto def = with_c(thgem_def(1000, 1000, 1000), [](EventDef& d) {
    d.ac_pre_ns = 200;
    d.ac_post_ns = 200;
    AcPair p;
    p.name = "Ge";
    p.detector = {K(0, 3)};
    p.shield = {K(0, 7)};
    d.ac_pairs.push_back(p);
  });
  GenericStats st;
  auto evs = one_chunk({hit(1, 0.0), hit(3, 600.0), hit(7, 780.0), hit(1, 1500.0),
                        hit(3, 2400.0), hit(7, 2550.0)},
                       def, st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK(find_ch(evs[0], 3) && find_ch(evs[0], 3)->with_ac);
  CHECK(find_ch(evs[1], 7) != nullptr);  // the partner went to event 2
  CHECK(find_ch(evs[1], 3) && find_ch(evs[1], 3)->with_ac);
  CHECK(st.orphans == 1);                // the second BGO (2550 > 1500 + 1000)
  CHECK(st.ac_flagged == 2);
}

static void test_generic_l2_counts_flags_accepts() {
  // Counters count HITS (one channel twice = 2), with the three AC modes;
  // flags compare; accepts AND/OR; L2Accept = OR of the accepts.
  auto def = with_c(ac_def(), [](EventDef& d) {
    L2Counter all, clean, vetoed, arms;
    all.name = "nGe"; all.channels = {K(0, 3)};
    clean.name = "nGeClean"; clean.channels = {K(0, 3)}; clean.ac = L2Ac::Clean;
    vetoed.name = "nGeVeto"; vetoed.channels = {K(0, 3)}; vetoed.ac = L2Ac::Vetoed;
    arms.name = "nArms"; arms.channels = {K(0, 2), K(0, 4), K(0, 5)};
    d.counters = {all, clean, vetoed, arms};
    L2Flag f0, f1, f2;
    f0.name = "TwoGe"; f0.counter = 0; f0.op = L2Op::Eq; f0.value = 2;
    f1.name = "AnyClean"; f1.counter = 1; f1.op = L2Op::Ge; f1.value = 1;
    f2.name = "ManyArms"; f2.counter = 3; f2.op = L2Op::Gt; f2.value = 3;
    d.flags = {f0, f1, f2};
    L2Accept a0, a1;
    a0.name = "Both"; a0.is_and = true; a0.flags = {0, 1};
    a1.name = "Either"; a1.is_and = false; a1.flags = {2};
    d.accepts = {a0, a1};
  });
  GenericStats st;
  // Event 1: two Ge hits on ch3 (one vetoed by BGO at +50), 2 arms.
  // Event 2 (far away): one clean Ge, 4 arm hits (ch2 twice).
  auto evs = one_chunk({hit(1, 0.0), hit(3, 10.0), hit(7, 60.0), hit(3, 600.0), hit(2, 20.0),
                        hit(4, 30.0), hit(1, 100000.0), hit(3, 100010.0), hit(2, 100020.0),
                        hit(2, 100025.0), hit(4, 100030.0), hit(5, 100040.0)},
                       def, st);
  CHECK(evs.size() == 2);
  if (evs.size() != 2) return;
  CHECK((evs[0].l2_counts == std::vector<uint32_t>{2, 1, 1, 2}));
  CHECK(evs[0].l2_flags == 0b011u);  // TwoGe, AnyClean
  CHECK(evs[0].l2_accepts == 0b01u && evs[0].l2_accept);
  CHECK((evs[1].l2_counts == std::vector<uint32_t>{1, 1, 0, 4}));
  CHECK(evs[1].l2_flags == 0b110u);  // AnyClean, ManyArms
  CHECK(evs[1].l2_accepts == 0b10u && evs[1].l2_accept);
  CHECK(st.flag_true[0] == 1 && st.flag_true[1] == 2 && st.flag_true[2] == 1);
  CHECK(st.accept_true[0] == 1 && st.accept_true[1] == 1 && st.l2_accepted == 2);
  // No l2 section: nothing computed, nothing counted.
  GenericStats st2;
  auto plain = one_chunk({hit(1, 0.0), hit(3, 10.0)}, ac_def(), st2);
  CHECK(plain.size() == 1 && plain[0].l2_counts.empty() && !plain[0].l2_accept);
  CHECK(st2.l2_accepted == 0);
}

static void test_generic_ac_late_shield_after_core_end() {
  // Chunk contract with AC: the first batch splits at core_end = 15000. The
  // event with reference 14500 holds a Ge hit at 14600 whose BGO partner
  // (15500, inside ac_post = 1000) is legitimately later than core_end and
  // only arrives with the next batch. The chunk must not emit that event yet
  // (ev_hi = D - pre - post - ac_post = 13600): the next chunk, which has the
  // BGO, emits it — flagged.
  auto def = with_c(thgem_def(200.0, 200.0, 100.0), [](EventDef& d) {
    d.ac_pre_ns = 0;
    d.ac_post_ns = 1000;
    AcPair p;
    p.name = "Ge";
    p.detector = {K(0, 3)};
    p.shield = {K(0, 7)};
    d.ac_pairs.push_back(p);
  });
  Sorter::Config scfg = small_cfg();
  scfg.lookback_ns = def->lookback_ns();
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  std::vector<GenericEvent> evs;
  GenericStats st;
  std::size_t chunks = 0;
  auto on_chunk = [&](SortedChunk&& c) {
    ++chunks;
    for (auto& e : build_generic_events(c, g.plan_chunk(c), st)) evs.push_back(std::move(e));
  };
  std::vector<ScalarHit> b1 = {hit(1, 14500.0), hit(3, 14600.0)};
  for (int i = 0; i <= 20; ++i) b1.push_back(hit(9, i * 1000.0));
  std::vector<ScalarHit> b2 = {hit(7, 15500.0)};
  for (int i = 21; i <= 40; ++i) b2.push_back(hit(9, i * 1000.0));
  s.push_batch(std::move(b1), on_chunk);
  s.push_batch(std::move(b2), on_chunk);
  s.flush(on_chunk);
  CHECK(chunks >= 2);
  CHECK(evs.size() == 1);
  CHECK(evs.size() == 1 && find_ch(evs[0], 3) && find_ch(evs[0], 3)->with_ac);
  CHECK(s.late_hits() == 0);
}

static void test_generic_ac_l2_matches_reference_randomized() {
  // As test_generic_matches_reference_randomized, plus AC pairs (a trigger
  // channel is also a detector; one channel is detector of one pair and
  // shield of another) and L2 with all three AC modes, both accept kinds.
  // Chunk spans are small so AC partners straddle chunk boundaries.
  int mismatches = 0;
  std::string first_why;
  for (int seed = 1; seed <= 200; ++seed) {
    std::mt19937 rng(static_cast<unsigned>(seed) * 7919u);
    std::uniform_int_distribution<int> u(0, 1000000);
    double pre = 100.0 + (u(rng) % 1200);
    double post = 100.0 + (u(rng) % 1200);
    double group = static_cast<double>(u(rng) % static_cast<int>(std::min(pre, post) + 1));
    auto base = mkdef(pre, post, group,
                      {{"A", 1, {{0, 1}}}, {"B", 2, {{0, 6}, {1, 2}}}, {"C", 1, {{1, 0}}}});
    double acpre = static_cast<double>(u(rng) % 900), acpost = static_cast<double>(u(rng) % 900);
    int op0 = u(rng) % 6, op1 = u(rng) % 6;
    long long v0 = u(rng) % 3, v1 = u(rng) % 3;
    bool and0 = u(rng) % 2;
    auto def = with_c(base, [&](EventDef& d) {
      d.ac_pre_ns = acpre;
      d.ac_post_ns = acpost;
      AcPair p1, p2;
      p1.name = "P1"; p1.detector = {K(0, 1), K(0, 3), K(0, 4)}; p1.shield = {K(1, 5), K(1, 6)};
      p2.name = "P2"; p2.detector = {K(1, 5), K(1, 3)}; p2.shield = {K(0, 7)};
      d.ac_pairs = {p1, p2};
      L2Counter c0, c1, c2, c3;
      c0.name = "nA"; c0.channels = {K(0, 1)};
      c1.name = "nDetClean"; c1.channels = {K(0, 3), K(0, 4), K(1, 3)}; c1.ac = L2Ac::Clean;
      c2.name = "nDetVeto"; c2.channels = {K(0, 3), K(0, 4), K(1, 3)}; c2.ac = L2Ac::Vetoed;
      c3.name = "nShield"; c3.channels = {K(1, 5), K(1, 6), K(0, 7)};
      d.counters = {c0, c1, c2, c3};
      L2Flag f0, f1, f2;
      f0.name = "F0"; f0.counter = 1; f0.op = static_cast<L2Op>(op0); f0.value = v0;
      f1.name = "F1"; f1.counter = 2; f1.op = static_cast<L2Op>(op1); f1.value = v1;
      f2.name = "F2"; f2.counter = 3; f2.op = L2Op::Ge; f2.value = 1;
      d.flags = {f0, f1, f2};
      L2Accept a0, a1;
      a0.name = "X0"; a0.is_and = and0; a0.flags = {0, 1};
      a1.name = "X1"; a1.is_and = !and0; a1.flags = {1, 2};
      d.accepts = {a0, a1};
    });
    std::vector<ScalarHit> hits;
    double t = 1000.0;
    int nburst = 30 + u(rng) % 40;
    for (int b = 0; b < nburst; ++b) {
      t += static_cast<double>(u(rng) % 4000);
      int n = 1 + u(rng) % 9;
      for (int k = 0; k < n; ++k) {
        double ht = t + static_cast<double>(u(rng) % 2400) - 600.0;
        if (u(rng) % 3 == 0) ht = std::floor(ht / 50.0) * 50.0;
        if (ht < 0) ht = 0;
        hits.push_back(mhit(u(rng) % 2, u(rng) % 8, ht, static_cast<uint16_t>(u(rng) & 0xffff)));
      }
    }
    RefResult ref = reference_build(hits, *def);
    Sorter::Config scfg;
    scfg.chunk_span_ns = 1500.0 + (u(rng) % 8000);
    scfg.safe_horizon_ns = 3000.0 + (u(rng) % 3000);
    scfg.lookback_ns = 0.0;
    std::vector<ScalarHit> arrival = hits;
    std::sort(arrival.begin(), arrival.end(), hit_order);
    jitter_arrival(arrival, scfg.safe_horizon_ns * 0.4, rng);
    PipeResult got = run_generic_pipeline(arrival, def, scfg, 1 + u(rng) % 15);
    std::string why;
    bool ok = got.late == 0 && same_events(got.events, ref.events, why) &&
              got.stats.orphans == ref.orphans && got.stats.contested == ref.contested &&
              got.stats.hits_in_events + got.stats.orphans == hits.size() &&
              got.stats.ac_flagged == ref.ac_flagged && got.stats.l2_accepted == ref.l2_accepted;
    for (std::size_t f = 0; ok && f < ref.flag_true.size(); ++f)
      ok = got.stats.flag_true[f] == ref.flag_true[f];
    for (std::size_t a = 0; ok && a < ref.accept_true.size(); ++a)
      ok = got.stats.accept_true[a] == ref.accept_true[a];
    if (!ok) {
      if (mismatches == 0) {
        char buf[256];
        std::snprintf(buf, sizeof buf, "seed %d (ac %g/%g, %zu chunks): %s | ac_flagged %llu/%llu",
                      seed, acpre, acpost, got.chunks, why.c_str(),
                      static_cast<unsigned long long>(got.stats.ac_flagged),
                      static_cast<unsigned long long>(ref.ac_flagged));
        first_why = buf;
      }
      ++mismatches;
    }
  }
  if (mismatches) std::printf("  AC/L2 randomized: %d/200 mismatch, first: %s\n", mismatches,
                              first_why.c_str());
  CHECK(mismatches == 0);
}

static void test_generic_pool_determinism() {
  // Plans are made in Sorter order; the builder is pure, so 1 vs 4 workers
  // must give the identical event sequence.
  auto def = thgem_def();
  Sorter::Config scfg = small_cfg();
  scfg.lookback_ns = def->lookback_ns();
  Sorter s(scfg);
  TriggerGrouper g;
  g.reset(def);
  std::mt19937 rng(77);
  std::uniform_real_distribution<double> jit(0.0, 900.0);
  std::vector<ScalarHit> all;
  for (int i = 0; i < 400; ++i) {
    double t = 1000.0 + i * 1300.0 + jit(rng);
    all.push_back(hit(i % 3 ? 1 : 6, t, static_cast<uint16_t>(i)));
    all.push_back(hit(2, t + jit(rng), 1));
    all.push_back(hit(0, t - jit(rng), 2));
  }
  jitter_arrival(all, 2000.0, rng);
  std::vector<WorkMsg> msgs;
  uint64_t seq = 0;
  auto on_chunk = [&](SortedChunk&& c) {
    WorkMsg m;
    m.plan = g.plan_chunk(c);
    m.chunk = std::move(c);
    m.seq = seq++;
    msgs.push_back(std::move(m));
  };
  std::size_t fed = 0;
  while (fed < all.size()) {
    std::vector<ScalarHit> b;
    for (int i = 0; i < 23 && fed < all.size(); ++i, ++fed) b.push_back(all[fed]);
    s.push_batch(std::move(b), on_chunk);
  }
  s.flush(on_chunk);
  CHECK(msgs.size() >= 3);
  auto run = [&](int nworkers) {
    Channel<WorkMsg> work, done;
    std::vector<std::thread> ws;
    for (int i = 0; i < nworkers; ++i)
      ws.emplace_back([&] {
        WorkMsg m;
        while (work.pop(m)) {
          bool pill = m.ctrl == Ctrl::Shutdown;
          if (!pill) m.events = build_generic_events(m.chunk, m.plan, m.gstats);
          done.push(std::move(m));
          if (pill) break;
        }
      });
    uint64_t sq = 0;
    for (const auto& m0 : msgs) {
      WorkMsg m;
      m.chunk = m0.chunk;
      m.plan = m0.plan;
      m.seq = sq++;
      work.push(std::move(m));
    }
    for (int i = 0; i < nworkers; ++i) {
      WorkMsg m;
      m.ctrl = Ctrl::Shutdown;
      m.seq = sq++;
      work.push(std::move(m));
    }
    SeqReorder<WorkMsg> reorder;
    std::vector<std::pair<double, std::size_t>> out;
    int pills = 0;
    WorkMsg m;
    while (pills < nworkers && done.pop(m)) {
      reorder.push(m.seq, std::move(m));
      WorkMsg r;
      while (reorder.pop_ready(r)) {
        if (r.ctrl == Ctrl::Shutdown) ++pills;
        for (const auto& e : r.events) out.push_back({e.t, e.hits.size()});
      }
    }
    for (auto& t : ws) t.join();
    return out;
  };
  auto s1 = run(1), s4 = run(4);
  CHECK(!s1.empty());
  CHECK(s1 == s4);
}

int main() {
  test_channel_fifo();
  test_channel_blocking_pop();
  test_channel_close_drains_then_false();
  test_channel_bounded_try_push();
  test_channel_pop_for_timeout();
  test_channel_pop_for_three_state();
  test_channel_pop_for_close_race();
  test_channel_move_only();
  test_channel_multi_producer();
  test_seq_reorder_in_order();
  test_seq_reorder_out_of_order();
  test_seq_reorder_gap_holds();
  test_split_empty();
  test_split_insufficient_span();
  test_split_retained_from_lookback();
  test_split_chain_invariant();
  test_flush();
  test_split_shuffled_sorts();
  test_core_range_conservation();
  test_core_range_bounds();
  test_sorter_data_time_only();
  test_sorter_first_chunk_floor();
  test_sorter_chain();
  test_sorter_reset_between_runs();
  test_sorter_flush_remainder();
  test_sorter_late_hit_lands_sorted();
  test_sorter_counts_hits_behind_core();
  test_sorter_orders_on_aligned_time();
  test_sorter_offset_spread_beyond_horizon_counts_late();
  test_generic_disabled();
  test_generic_single_event();
  test_generic_group_does_not_extend();
  test_generic_abc_chain_no_loss();
  test_generic_priority_picks_reference();
  test_generic_priority_ties();
  test_generic_no_priorities_first_trigger();
  test_generic_nearest_reference_wins();
  test_generic_trigger_follows_its_group();
  test_generic_window_edges_inclusive();
  test_generic_asymmetric_nearest_out_of_window();
  test_generic_contested_counts_any_two_windows();
  test_generic_group_zero();
  test_generic_ntrig_histogram();
  test_generic_reset_between_runs();
  test_sorter_set_lookback();
  test_generic_boundary_reference_after_core_end();
  test_generic_matches_reference_randomized();
  test_generic_group_span_ends_at_core_end();
  test_generic_ac_window_edges();
  test_generic_ac_partner_in_other_event_or_none();
  test_generic_l2_counts_flags_accepts();
  test_generic_ac_late_shield_after_core_end();
  test_generic_ac_l2_matches_reference_randomized();
  test_generic_pool_determinism();
  std::printf("test_eb_core: %d passed, %d failed\n", g_pass, g_fail);
  return g_fail == 0 ? 0 : 1;
}
