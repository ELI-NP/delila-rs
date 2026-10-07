// eb_core.hpp — multithreaded event-builder core for root_sink (TODO 66 Phase 1).
//
// This header holds every piece of the Receiver → Sorter → Workers → Writer
// pipeline that can exist without ROOT or ZMQ: the inter-thread channel, the
// sequence reorderer, the sorted-chunk splitter (a faithful port of the frozen
// Rust reference in src/event_builder/chunk_builder.rs, including the TODO 58
// H10 chunk-boundary rules), and the data-time-driven Sorter. Everything here
// is unit-testable with a plain `g++ -std=c++17 -pthread` build — see
// test_eb_core.cpp. The thread spawning and the ROOT/ZMQ wiring live in
// root_sink.cxx.
//
// Design constraints inherited from the project rules (CLAUDE.md / TODO 66 §5):
//   * Record-path channels are UNBOUNDED — data is never dropped between the
//     ZMQ socket and the ROOT file. Only the display tee may be bounded, and
//     its drops must be counted by the caller (Monitor-exception policy).
//   * Chunks deliberately carry duplicated context hits (the lookback overlap
//     and the post-core tail). Consumers that must see each hit exactly once
//     (the hits tree) use core_range(); consumers that need coincidence
//     context (the builder) scan the full chunk. Do not "fix" the overlap.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef ROOTSINK_EB_CORE_HPP
#define ROOTSINK_EB_CORE_HPP

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <limits>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

#include "event_def.hpp"  // rootsink::EventDef (generic grouping layer)
#include "sink_core.hpp"  // rootsink::ScalarHit

namespace rootsink {
namespace eb {

// ---------------------------------------------------------------------------
// 1. Channel<T> — the one inter-thread queue (mutex + condvar + deque)
// ---------------------------------------------------------------------------
//
// capacity == 0 means unbounded (the record path: push never blocks, nothing
// is ever dropped). A bounded channel (the display tee) offers two flavors:
//   * try_push — returns false when full WITHOUT consuming the value, so the
//     caller can count the drop and keep the original (Monitor-exception
//     bookkeeping).
//   * push     — blocks until space; reserved for control markers (RunOpen /
//     RunClose / Shutdown), which must never be dropped even on the display
//     path (a lost RunOpen would leave stale matcher clocks across runs).
// Message granularity is a whole batch/chunk, so lock traffic is a few dozen
// operations per second — a lock-free structure would buy nothing here (KISS).

// Outcome of Channel::pop_for. Value/Timeout/Closed are distinct so that a
// consumer can exit on closed-and-drained ONLY — see the comment on pop_for.
enum class PopResult { Value, Timeout, Closed };

template <class T>
class Channel {
 public:
  explicit Channel(std::size_t capacity = 0) : cap_(capacity) {}

  // Blocking push. Unbounded: appends immediately. Bounded: waits for space.
  // After close() the value is silently discarded — producers may still be
  // running during shutdown and have nowhere else to put it.
  void push(T&& v) {
    std::unique_lock<std::mutex> lk(m_);
    if (cap_ > 0) cv_push_.wait(lk, [&] { return q_.size() < cap_ || closed_; });
    if (closed_) return;
    q_.push_back(std::move(v));
    cv_pop_.notify_one();
  }

  // Non-blocking push for the bounded display data path. Returns false when
  // full and leaves `v` UNTOUCHED (no move happened) so the caller can count
  // the drop. On a closed channel the value is discarded and true is returned:
  // shutdown-time discards are not display drops and must not inflate the
  // counter.
  bool try_push(T&& v) {
    std::lock_guard<std::mutex> lk(m_);
    if (closed_) return true;
    if (cap_ > 0 && q_.size() >= cap_) return false;
    q_.push_back(std::move(v));
    cv_pop_.notify_one();
    return true;
  }

  // Blocking pop. Returns false only when the channel is closed AND drained —
  // the normal thread-exit signal for abnormal termination. (Orderly shutdown
  // flows through in-band Ctrl::Shutdown markers instead.)
  bool pop(T& out) {
    std::unique_lock<std::mutex> lk(m_);
    cv_pop_.wait(lk, [&] { return !q_.empty() || closed_; });
    if (q_.empty()) return false;  // closed and drained
    out = std::move(q_.front());
    q_.pop_front();
    if (cap_ > 0) cv_push_.notify_one();
    return true;
  }

  // Pop with a timeout, for loops that must wake periodically (the Display
  // loop's ProcessEvents tick, the Writer's autosave timer).
  //
  // Three-state result, decided under the channel lock (issue #26): a bool
  // here would conflate Timeout with Closed, forcing callers onto an external
  // exit flag — and "queue empty" + "flag set" are observed at different
  // times, so a producer's final push(msg); close() can slip between them and
  // the last message is silently lost. (Hit for real in tpcdaq-rs, whose
  // Channel is a port of this one.) With Closed only ever returned when the
  // queue is empty AND closed under one lock, drain-then-Closed is guaranteed:
  // exit on Closed, never on Timeout.
  PopResult pop_for(T& out, int timeout_ms) {
    std::unique_lock<std::mutex> lk(m_);
    cv_pop_.wait_for(lk, std::chrono::milliseconds(timeout_ms),
                     [&] { return !q_.empty() || closed_; });
    if (q_.empty()) return closed_ ? PopResult::Closed : PopResult::Timeout;
    out = std::move(q_.front());
    q_.pop_front();
    if (cap_ > 0) cv_push_.notify_one();
    return PopResult::Value;
  }

  // Wake every blocked producer and consumer. Pending items remain poppable
  // (drain-then-false semantics in pop()).
  void close() {
    std::lock_guard<std::mutex> lk(m_);
    closed_ = true;
    cv_pop_.notify_all();
    cv_push_.notify_all();
  }

  // Instantaneous depth — status-line gauge only (racy by nature, fine).
  std::size_t size() const {
    std::lock_guard<std::mutex> lk(m_);
    return q_.size();
  }

 private:
  mutable std::mutex m_;
  std::condition_variable cv_pop_, cv_push_;
  std::deque<T> q_;
  std::size_t cap_;  // 0 = unbounded
  bool closed_ = false;
};

// ---------------------------------------------------------------------------
// 2. SeqReorder<T> — restore Sorter emission order after the worker pool
// ---------------------------------------------------------------------------
//
// Workers finish out of order; the Writer pushes each result in with its
// Sorter-assigned sequence number and pops back the contiguous prefix. This
// makes the output deterministic in the worker count (same input → same tree,
// --workers 1 or 8) and turns in-band control markers into barriers for free:
// a RunClose with seq k cannot pop before every chunk with seq < k.
//
// NOT thread-safe — it lives entirely inside the Writer thread.
template <class T>
class SeqReorder {
 public:
  void push(uint64_t seq, T&& v) { buf_.emplace(seq, std::move(v)); }

  // True while the next contiguous sequence number (starting at 0) is
  // buffered; `out` receives that item.
  bool pop_ready(T& out) {
    auto it = buf_.find(next_);
    if (it == buf_.end()) return false;
    out = std::move(it->second);
    buf_.erase(it);
    ++next_;
    return true;
  }

  std::size_t pending() const { return buf_.size(); }

 private:
  std::map<uint64_t, T> buf_;
  uint64_t next_ = 0;
};

// ---------------------------------------------------------------------------
// 3. SortedChunk + split / flush / core_range
// ---------------------------------------------------------------------------
//
// Port of src/event_builder/chunk_builder.rs (SortedChunk, sort_and_split,
// sort_and_flush) — the frozen Rust reference that carries the TODO 58 H10
// chunk-boundary fixes. Semantics preserved exactly; see the Rust file for
// the original derivation.

struct SortedChunk {
  std::vector<ScalarHit> hits;  // ascending timestamp_ns; includes context
  // Emit floor [ns] (H10): triggers before this were already emitted by the
  // previous chunk — they are present here only as backward-window context
  // carried in by the lookback overlap. First chunk of a run: -infinity.
  double core_start = -std::numeric_limits<double>::infinity();
  // Emit ceiling [ns]: triggers at or after this belong to the next chunk,
  // but their hits remain visible as coincidence partners in this one.
  double core_end = std::numeric_limits<double>::infinity();
};

namespace detail {
inline bool ts_less(const ScalarHit& a, const ScalarHit& b) {
  // Plain < is safe: timestamps come off the wire as finite f64 (the digitizer
  // never produces NaN), so total_cmp-style NaN ordering is not needed here.
  return a.timestamp_ns < b.timestamp_ns;
}
}  // namespace detail

// Sort `buffer` and split off a chunk whose core ends safe_horizon_ns before
// the newest hit. On success (returns true): `chunk` takes ALL of the buffer's
// hits, and `buffer` is replaced by the retained tail starting at
// core_end - lookback_ns — the [core_end - lookback, core_end) overlap rides
// along purely as context; the next chunk's core_start (== this core_end)
// keeps those hits from emitting twice (H10). On failure (empty buffer, or
// core_end would not clear the earliest hit) returns false with `buffer`
// sorted but otherwise untouched.
inline bool sort_and_split(std::vector<ScalarHit>& buffer, double safe_horizon_ns,
                           double lookback_ns, double core_start,
                           SortedChunk& chunk) {
  if (buffer.empty()) return false;
  std::sort(buffer.begin(), buffer.end(), detail::ts_less);
  double earliest = buffer.front().timestamp_ns;
  double latest = buffer.back().timestamp_ns;
  double core_end = latest - safe_horizon_ns;
  if (core_end <= earliest) return false;  // not enough spread yet
  // No core progress since the previous split (no newer data arrived): refuse.
  // Without this guard, a caller looping "split until false" would carve
  // zero-width chunks out of the retained lookback tail forever. (The Rust
  // original relies on its caller's span check for this; here the invariant
  // "a chunk always advances the core" lives in the primitive itself.)
  if (core_end <= core_start) return false;

  auto split = std::lower_bound(
      buffer.begin(), buffer.end(), core_end - lookback_ns,
      [](const ScalarHit& h, double v) { return h.timestamp_ns < v; });
  std::vector<ScalarHit> retained(split, buffer.end());

  chunk.hits = std::move(buffer);
  chunk.core_start = core_start;
  chunk.core_end = core_end;
  buffer = std::move(retained);
  return true;
}

// EOS / shutdown flush: everything left becomes one final chunk with
// core_end = +infinity. The core_start floor still applies — lookback hits
// carried into this buffer were already emitted by the previous chunk.
// Returns false when there is nothing to flush.
inline bool sort_and_flush(std::vector<ScalarHit>& buffer, double core_start,
                           SortedChunk& chunk) {
  if (buffer.empty()) return false;
  std::sort(buffer.begin(), buffer.end(), detail::ts_less);
  chunk.hits = std::move(buffer);
  buffer.clear();
  chunk.core_start = core_start;
  chunk.core_end = std::numeric_limits<double>::infinity();
  return true;
}

// [first, last) index range of the hits owned by this chunk's core region
// [core_start, core_end). THE dedup rule for exactly-once consumers: over a
// run's chunk sequence (chained core_start == previous core_end, final flush
// at +inf), every hit falls in exactly one chunk's core range — the Writer
// fills the hits tree from this range only, never from the full chunk.
inline std::pair<std::size_t, std::size_t> core_range(const SortedChunk& c) {
  auto lo = std::lower_bound(
      c.hits.begin(), c.hits.end(), c.core_start,
      [](const ScalarHit& h, double v) { return h.timestamp_ns < v; });
  auto hi = std::lower_bound(
      lo, c.hits.end(), c.core_end,
      [](const ScalarHit& h, double v) { return h.timestamp_ns < v; });
  return {static_cast<std::size_t>(lo - c.hits.begin()),
          static_cast<std::size_t>(hi - c.hits.begin())};
}

// ---------------------------------------------------------------------------
// 4. Sorter — accumulate, sort, emit chunks on DATA-TIME advance
// ---------------------------------------------------------------------------
//
// The single owner of the disorder buffer. Emission is driven purely by data
// time (the watermark advancing), never by hit count or wall clock: the rate
// can rise or fall and chunks keep a constant time span, and a silent beam
// pause simply pauses emission (EOS flushes the remainder). One split per
// push_batch is sufficient — sort_and_split advances core_end all the way to
// max_ts - safe_horizon, so the emit condition cannot re-fire until new data
// arrives.
class Sorter {
 public:
  struct Config {
    double chunk_span_ns = 100e6;    // emit when the core grows this much
    double safe_horizon_ns = 50e6;   // disorder absorption (>= max disorder)
    double lookback_ns = 1000.0;     // context carry (>= coincidence window)
  };

  explicit Sorter(Config cfg) : cfg_(cfg) {}

  // Append a decoded batch (any internal order) and emit at most one chunk.
  template <class ChunkF>
  void push_batch(std::vector<ScalarHit>&& hits, ChunkF on_chunk) {
    for (const ScalarHit& h : hits) {
      if (h.timestamp_ns > max_ts_) max_ts_ = h.timestamp_ns;
      if (h.timestamp_ns < min_ts_) min_ts_ = h.timestamp_ns;
      // Behind the emitted core: no future core range can hold it (H10 floor),
      // so it reaches neither the hits tree nor a built event. Out of the
      // safe-horizon contract — counted so the caller can say so.
      if (h.timestamp_ns < prev_core_end_) ++late_;
    }
    if (buffer_.empty()) {
      buffer_ = std::move(hits);
    } else {
      buffer_.insert(buffer_.end(), std::make_move_iterator(hits.begin()),
                     std::make_move_iterator(hits.end()));
    }
    if (buffer_.empty()) return;

    double candidate = max_ts_ - cfg_.safe_horizon_ns;
    // Floor: where the previous core ended, or (first chunk) the earliest hit
    // ever buffered this run. The emit test against the floor is what makes
    // emission data-time-driven.
    double floor = std::isinf(prev_core_end_) ? min_ts_ : prev_core_end_;
    if (candidate - floor < cfg_.chunk_span_ns) return;

    SortedChunk chunk;
    if (sort_and_split(buffer_, cfg_.safe_horizon_ns, cfg_.lookback_ns,
                       prev_core_end_, chunk)) {
      prev_core_end_ = chunk.core_end;
      on_chunk(std::move(chunk));
    }
  }

  // Emit everything left as a final chunk (RunClose / Shutdown).
  template <class ChunkF>
  void flush(ChunkF on_chunk) {
    SortedChunk chunk;
    if (sort_and_flush(buffer_, prev_core_end_, chunk)) {
      on_chunk(std::move(chunk));
    }
  }

  // Call at RunOpen. CRITICAL: prev_core_end_ MUST return to -infinity — the
  // digitizer clock restarts between runs, and a stale floor from run N's huge
  // timestamps would put every run-N+1 hit below core_start, silently emitting
  // nothing (no built events AND an empty hits tree) for the entire run.
  void reset() {
    buffer_.clear();
    max_ts_ = -std::numeric_limits<double>::infinity();
    min_ts_ = std::numeric_limits<double>::infinity();
    prev_core_end_ = -std::numeric_limits<double>::infinity();
    late_ = 0;
  }

  // Per-run context carry: the generic event layer needs G + 2*pre + post
  // (section 7), which follows the event definition reloaded at RunOpen. Call
  // after reset(), before the run's first push_batch.
  void set_lookback_ns(double ns) { cfg_.lookback_ns = ns; }
  double lookback_ns() const { return cfg_.lookback_ns; }

  std::size_t buffered() const { return buffer_.size(); }

  // Hits of this run that arrived behind an already-emitted core_end (disorder
  // beyond safe_horizon_ns) and are therefore missing from the hits tree and
  // the built events. Reset by reset().
  uint64_t late_hits() const { return late_; }

 private:
  Config cfg_;
  std::vector<ScalarHit> buffer_;
  double max_ts_ = -std::numeric_limits<double>::infinity();
  double min_ts_ = std::numeric_limits<double>::infinity();
  double prev_core_end_ = -std::numeric_limits<double>::infinity();
  uint64_t late_ = 0;
};

// ---------------------------------------------------------------------------
// 5. Integrated-event builder (pure function over a SortedChunk)
// ---------------------------------------------------------------------------
//
// One BuiltEvent row carries BOTH ThGEM detectors plus the LaBr3 gamma:
// completeness is an offline cut, never a build condition (TODO 66 §5.2).
// A missing piece is NaN (times/derived) or 0 (energies) — 4-fold analysis is
// `!isnan(x1) && !isnan(y1)`, plane correlation is `!isnan(dt_trig)`, and the
// arm-efficiency map falls out of n_arms for free. Trigger-only events are
// therefore emitted too; dropping them would destroy the efficiency
// measurement, so don't.
//
// Matching semantics are the offline ground truth (tmp/test.cpp — the code
// that produced the 15,644 four-fold reference count on run0023, and
// macros/grid_resolution.C): per arm, the NEAREST hit to that detector's own
// trigger within ±window_ns (inclusive), via lower_bound predecessor/successor
// comparison. NOT LiveEventBuilder.C's first-found-backward rule. The X/Y
// convention is x = t_XR - t_XL, y = t_YU - t_YD in raw ns — LiveEventBuilder's
// X=(t4-t5) is that macro's deliberate 90-degree rotation; do not copy it, and
// do not bake the mm calibration or the -70 ns TOF offset in here (analysis
// constants live in macros). Per-channel cable/delay offsets are a different
// thing: they correct the data time axis itself, so the Receiver applies them
// to every hit before the Sorter (time_offsets.hpp, --time-calib) and all
// times below are on that aligned axis (= raw when no offsets are configured).
//
// Cross-plane pairing (the "integrated event"): both trigger channels are
// scanned as one time-ordered sequence; a trigger pairs with the IMMEDIATELY
// NEXT trigger when that one belongs to the other detector and lies within
// window_ns (greedy, first-come — genuine coincidences sit ~ns apart while
// the window is ~us, so nearest-pair refinement would change nothing real).
// Same-detector triggers never merge (two ch1 triggers = two events). The
// event's anchor is the EARLIER trigger of the pair; dt_trig is ALWAYS
// t(det2) - t(det1) regardless of which side anchored.
//
// Chunk-boundary determinism (H10): pairing and arm matching run over the
// FULL chunk (context included), but an event is emitted only when its anchor
// falls in [core_start, core_end). A pair spanning core_end is emitted by the
// chunk that owns the anchor; in the next chunk the anchor reappears as
// lookback context (lookback >= window guarantees it), pairs again, and is
// gated out by core_start — so the partner never anchors a duplicate.
// Requirement on the caller: lookback_ns >= window_ns and
// safe_horizon_ns >= 2*window_ns (trivially true at 50 ms vs ~us windows).

struct BuiltEvent {
  // Times are digitizer-clock ns on the ALIGNED axis (raw - timeSettings offset,
  // applied by the Receiver; raw if none). NaN = that piece is absent.
  double trig1_t = std::numeric_limits<double>::quiet_NaN();
  double trig2_t = std::numeric_limits<double>::quiet_NaN();
  double labr_t = std::numeric_limits<double>::quiet_NaN();
  // Positions in raw ns: xN = dt_xrN - dt_xlN, yN = dt_yuN - dt_ydN — valid
  // only when BOTH arms of that axis matched.
  float x1 = std::numeric_limits<float>::quiet_NaN();
  float y1 = std::numeric_limits<float>::quiet_NaN();
  float x2 = std::numeric_limits<float>::quiet_NaN();
  float y2 = std::numeric_limits<float>::quiet_NaN();
  // Per-arm deltas dt = t(arm) - t(own trigger); NaN = arm not matched.
  float dt_xl1 = std::numeric_limits<float>::quiet_NaN();
  float dt_xr1 = std::numeric_limits<float>::quiet_NaN();
  float dt_yu1 = std::numeric_limits<float>::quiet_NaN();
  float dt_yd1 = std::numeric_limits<float>::quiet_NaN();
  float dt_xl2 = std::numeric_limits<float>::quiet_NaN();
  float dt_xr2 = std::numeric_limits<float>::quiet_NaN();
  float dt_yu2 = std::numeric_limits<float>::quiet_NaN();
  float dt_yd2 = std::numeric_limits<float>::quiet_NaN();
  // Plane-to-plane: t(det2 trigger) - t(det1 trigger); NaN unless both fired.
  float dt_trig = std::numeric_limits<float>::quiet_NaN();
  // Raw TOFs: t(trigger) - t(labr), no analysis offset applied.
  float tof1 = std::numeric_limits<float>::quiet_NaN();
  float tof2 = std::numeric_limits<float>::quiet_NaN();
  uint16_t e_trig1 = 0, e_trig2 = 0, e_labr = 0;
  uint16_t e_xl1 = 0, e_xr1 = 0, e_yu1 = 0, e_yd1 = 0;
  uint16_t e_xl2 = 0, e_xr2 = 0, e_yu2 = 0, e_yd2 = 0;
  uint32_t e_sum1 = 0, e_sum2 = 0;  // sum of PRESENT arms (4x65535 > u16)
  uint8_t n_arms1 = 0, n_arms2 = 0;  // 0..4 matched arms (LaBr not counted)
};

struct BuilderConfig {
  // Detector 1 (--xy1-ch T,XL,XR,YU,YD). -1 = channel unused.
  int trig1 = -1, xl1 = -1, xr1 = -1, yu1 = -1, yd1 = -1;
  // Detector 2 (--xy2-ch); trig2 = -1 => single-detector mode.
  int trig2 = -1, xl2 = -1, xr2 = -1, yu2 = -1, yd2 = -1;
  int labr_ch = -1;  // --gamma-ch; -1 => tof1/tof2/labr_t stay NaN
  double window_ns = 500.0;
};

namespace detail {

// Per-channel column view: times ascending (inherited from the sorted chunk)
// with the matching energies alongside.
struct ChView {
  std::vector<double> t;
  std::vector<uint16_t> e;
};

// Nearest hit to t0 within ±window (inclusive): lower_bound successor, then
// take the predecessor when it is STRICTLY closer (grid_resolution.C's rule —
// ties go to the successor). dt = t(hit) - t0, signed.
inline bool nearest_in(const ChView& v, double t0, double window, double& dt,
                       uint16_t& energy) {
  if (v.t.empty()) return false;
  std::size_t idx;
  auto it = std::lower_bound(v.t.begin(), v.t.end(), t0);
  if (it == v.t.end()) {
    idx = v.t.size() - 1;
  } else {
    idx = static_cast<std::size_t>(it - v.t.begin());
    if (idx > 0 && (t0 - v.t[idx - 1]) < (v.t[idx] - t0)) idx = idx - 1;
  }
  double d = v.t[idx] - t0;
  double ad = d < 0 ? -d : d;
  if (ad > window) return false;
  dt = d;
  energy = v.e[idx];
  return true;
}

}  // namespace detail

inline std::vector<BuiltEvent> build_events_from_chunk(const SortedChunk& chunk,
                                                       const BuilderConfig& cfg) {
  std::vector<BuiltEvent> out;
  if (chunk.hits.empty()) return out;

  // Distribute the sorted chunk into per-channel columns (sorted for free).
  detail::ChView vt1, vxl1, vxr1, vyu1, vyd1;
  detail::ChView vt2, vxl2, vxr2, vyu2, vyd2;
  detail::ChView vlabr;
  auto grab = [](detail::ChView& v, const ScalarHit& h) {
    v.t.push_back(h.timestamp_ns);
    v.e.push_back(h.energy);
  };
  for (const ScalarHit& h : chunk.hits) {
    int ch = static_cast<int>(h.channel);
    if (ch == cfg.trig1) grab(vt1, h);
    else if (ch == cfg.xl1) grab(vxl1, h);
    else if (ch == cfg.xr1) grab(vxr1, h);
    else if (ch == cfg.yu1) grab(vyu1, h);
    else if (ch == cfg.yd1) grab(vyd1, h);
    else if (ch == cfg.trig2) grab(vt2, h);
    else if (ch == cfg.xl2) grab(vxl2, h);
    else if (ch == cfg.xr2) grab(vxr2, h);
    else if (ch == cfg.yu2) grab(vyu2, h);
    else if (ch == cfg.yd2) grab(vyd2, h);
    else if (ch == cfg.labr_ch) grab(vlabr, h);
  }

  // Merge both trigger columns into one time-ordered sequence (ties: det1
  // first — deterministic).
  struct Trig {
    double t;
    uint16_t e;
    int det;  // 1 or 2
  };
  std::vector<Trig> trigs;
  trigs.reserve(vt1.t.size() + vt2.t.size());
  {
    std::size_t i = 0, j = 0;
    while (i < vt1.t.size() || j < vt2.t.size()) {
      bool take1 = j >= vt2.t.size() ||
                   (i < vt1.t.size() && vt1.t[i] <= vt2.t[j]);
      if (take1) {
        trigs.push_back({vt1.t[i], vt1.e[i], 1});
        ++i;
      } else {
        trigs.push_back({vt2.t[j], vt2.e[j], 2});
        ++j;
      }
    }
  }

  // Match one detector side of an event: arms nearest to that detector's own
  // trigger, x/y from complete axes, energy sum over present arms.
  auto fill_det = [&](const Trig& tr, const detail::ChView& xl,
                      const detail::ChView& xr, const detail::ChView& yu,
                      const detail::ChView& yd, double& trig_t, uint16_t& e_trig,
                      float& dxl, float& dxr, float& dyu, float& dyd, float& x,
                      float& y, uint16_t& exl, uint16_t& exr, uint16_t& eyu,
                      uint16_t& eyd, uint32_t& e_sum, uint8_t& n_arms) {
    trig_t = tr.t;
    e_trig = tr.e;
    double d = 0.0;
    uint16_t en = 0;
    if (detail::nearest_in(xl, tr.t, cfg.window_ns, d, en)) {
      dxl = static_cast<float>(d); exl = en; e_sum += en; ++n_arms;
    }
    if (detail::nearest_in(xr, tr.t, cfg.window_ns, d, en)) {
      dxr = static_cast<float>(d); exr = en; e_sum += en; ++n_arms;
    }
    if (detail::nearest_in(yu, tr.t, cfg.window_ns, d, en)) {
      dyu = static_cast<float>(d); eyu = en; e_sum += en; ++n_arms;
    }
    if (detail::nearest_in(yd, tr.t, cfg.window_ns, d, en)) {
      dyd = static_cast<float>(d); eyd = en; e_sum += en; ++n_arms;
    }
    if (!std::isnan(dxl) && !std::isnan(dxr)) x = dxr - dxl;
    if (!std::isnan(dyu) && !std::isnan(dyd)) y = dyu - dyd;
  };

  // Greedy pairing scan + emission gate on the ANCHOR time (H10 gates).
  std::size_t k = 0;
  while (k < trigs.size()) {
    const Trig& a = trigs[k];
    const Trig* b = nullptr;
    if (k + 1 < trigs.size() && trigs[k + 1].det != a.det &&
        trigs[k + 1].t - a.t <= cfg.window_ns) {
      b = &trigs[k + 1];
    }
    std::size_t consumed = b ? 2 : 1;
    if (a.t >= chunk.core_start && a.t < chunk.core_end) {
      BuiltEvent ev;
      const Trig* t1 = (a.det == 1) ? &a : b;
      const Trig* t2 = (a.det == 2) ? &a : b;
      if (t1) {
        fill_det(*t1, vxl1, vxr1, vyu1, vyd1, ev.trig1_t, ev.e_trig1, ev.dt_xl1,
                 ev.dt_xr1, ev.dt_yu1, ev.dt_yd1, ev.x1, ev.y1, ev.e_xl1,
                 ev.e_xr1, ev.e_yu1, ev.e_yd1, ev.e_sum1, ev.n_arms1);
      }
      if (t2) {
        fill_det(*t2, vxl2, vxr2, vyu2, vyd2, ev.trig2_t, ev.e_trig2, ev.dt_xl2,
                 ev.dt_xr2, ev.dt_yu2, ev.dt_yd2, ev.x2, ev.y2, ev.e_xl2,
                 ev.e_xr2, ev.e_yu2, ev.e_yd2, ev.e_sum2, ev.n_arms2);
      }
      if (t1 && t2) ev.dt_trig = static_cast<float>(t2->t - t1->t);
      // One gamma per event: nearest LaBr3 hit to the anchor; both TOFs come
      // from that same hit (so tof2 - tof1 == dt_trig by construction).
      double dl = 0.0;
      uint16_t el = 0;
      if (detail::nearest_in(vlabr, a.t, cfg.window_ns, dl, el)) {
        ev.labr_t = a.t + dl;
        ev.e_labr = el;
        if (t1) ev.tof1 = static_cast<float>(t1->t - ev.labr_t);
        if (t2) ev.tof2 = static_cast<float>(t2->t - ev.labr_t);
      }
      out.push_back(ev);
    }
    k += consumed;
  }
  return out;
}

// ---------------------------------------------------------------------------
// 7. Generic event layer (TODO 71 Phase A) — "まとめる層"
// ---------------------------------------------------------------------------
//
// Bundle hits around triggers for EVERY experiment the same way; what an event
// means (XY, addback, E-dE) is the per-experiment reconstruction layer on top
// (the ThGEM BuiltEvent above is the first such layer). Rules, parameterized by
// event_def.hpp (TODO/71_generic_event_building.md §4):
//   1. Triggers in time order; an ungrouped trigger t opens the group [t, t+G]
//      (non-extending). Every trigger belongs to exactly one group — no
//      cascade loss as with ELIFANT-Event's ID priority (A->B->C at 0.8 W).
//   2. Reference = best (priority, time, module, channel) in the group.
//   3. Window [t_ref - pre, t_ref + post], inclusive; G <= min(pre, post) puts
//      every group member inside its own window.
//   4. Trigger hits follow their group (rule 1 owns them). Any other hit goes
//      to the NEAREST reference whose window holds it (tie -> the earlier
//      reference); in no window -> in no event (it stays in the .delila).
//
// Split of work. Rules 1-2 are sequential over the whole run (where a group
// starts depends on where the previous one did), so the single-threaded
// Sorter runs TriggerGrouper over each chunk's core triggers, in order, and
// attaches the resulting EventPlan. Rules 3-4 are local, so the Workers run
// build_generic_events() in parallel, a pure function of (chunk, plan).
//
// Chunk contract (why the lookback is G + 2*pre + post). Hits below a chunk's
// core_end are complete (Sorter disorder contract), so after feeding a chunk's
// core triggers every group starting before the decided frontier
//   D = (start of the still-open group) or core_end if none is open
// is final, and D >= core_end - G (a group closes as soon as start + G <
// core_end). Assigning a hit h of an event with reference r needs every
// reference that could compete for h, i.e. up to h + pre <= r + post + pre;
// so the chunk emits exactly the events with r in [ev_lo, ev_hi), where
// ev_hi = D - pre - post and ev_lo = the previous chunk's ev_hi (chained:
// -inf first, +inf at the flush). Those events need hits down to
// ev_lo - pre >= previous core_end - G - 2*pre - post, which the Sorter's
// lookback must carry: EventDef::lookback_ns(). Upward they need hits below
// ev_hi + post = D - pre <= core_end, i.e. complete ones. Exactly-once: each
// event is emitted by the one chunk whose [ev_lo, ev_hi) holds its reference;
// each orphan is counted by the one chunk whose [ev_lo, ev_hi) holds its TIME.
// Over a run, hits_in_events + orphans == the hits in the core ranges.
//
// A hit that broke the disorder contract (Sorter late_hits(): behind an
// emitted core) is not in the hits tree and was not fed as a trigger; it may
// still ride a later chunk as context and join an event there. That case is
// already reported as data loss by the Sorter, so it is not special-cased.

// One decided group of triggers (rule 1) with its reference (rule 2).
struct GroupRef {
  double t = 0.0;        // reference trigger time (aligned ns) = the event's TriggerTime
  double first_t = 0.0;  // group span: every member trigger lies in [first_t, last_t]
  double last_t = 0.0;
  uint8_t mod = 0, ch = 0;  // reference trigger channel
  uint8_t trig_idx = 0;     // index into EventDef::triggers
  uint32_t n_triggers = 0;  // group size (>= 1)
};

// What the Sorter attaches to each chunk (TriggerGrouper::plan_chunk).
struct EventPlan {
  std::shared_ptr<const EventDef> def;  // null = generic events disabled
  std::vector<GroupRef> refs;           // ascending t; every decided ref that can matter
  // Emit exactly the events whose reference time is in [ev_lo, ev_hi).
  // Chained over a run (ev_lo == previous ev_hi; -inf first, +inf at flush).
  double ev_lo = -std::numeric_limits<double>::infinity();
  double ev_hi = -std::numeric_limits<double>::infinity();
};

struct EventHit {
  uint8_t mod = 0, ch = 0;
  uint16_t energy = 0, energy_short = 0;
  double dt = 0.0;          // hit time - reference time (aligned ns)
  bool is_trigger = false;  // a trigger-channel hit (a member of this event's group)
};

struct GenericEvent {
  double t = 0.0;  // reference time (aligned, absolute)
  uint8_t mod = 0, ch = 0, trig_idx = 0;
  uint32_t n_triggers = 0;
  std::vector<EventHit> hits;  // time order; includes the reference itself
};

// Per-chunk counters (summed per run by the Writer). Exactly-once over a run:
// hits_in_events + orphans == every hit the Sorter put in a core range.
struct GenericStats {
  uint64_t events = 0;
  uint64_t hits_in_events = 0;
  uint64_t contested = 0;  // non-trigger hits inside two windows (nearest reference won)
  uint64_t orphans = 0;    // hits inside no window (still in the .delila / hits tree)
  // NTriggers distribution: [i] = events with min(NTriggers, 8) == i + 1 ("8" = 8+).
  std::array<uint64_t, 8> ntrig{};
  void add(const GenericStats& o) {
    events += o.events;
    hits_in_events += o.hits_in_events;
    contested += o.contested;
    orphans += o.orphans;
    for (std::size_t i = 0; i < ntrig.size(); ++i) ntrig[i] += o.ntrig[i];
  }
};

// Rules 1-2, run by the (single-threaded) Sorter on every emitted chunk, in
// order (flush chunks included — the +inf core_end closes the last group).
// Stateful across the chunks of one run; reset() at RunOpen.
class TriggerGrouper {
 public:
  // CRITICAL at RunOpen, like Sorter::reset(): the digitizer clock restarts,
  // and a stale ev_lo from run N's huge timestamps would gate out every event
  // of run N+1.
  void reset(std::shared_ptr<const EventDef> def) {
    def_ = std::move(def);
    refs_.clear();
    open_ = false;
    ev_lo_ = -std::numeric_limits<double>::infinity();
  }
  bool enabled() const { return def_ != nullptr; }

  EventPlan plan_chunk(const SortedChunk& c) {
    EventPlan p;
    p.def = def_;
    if (!def_) return p;
    const EventDef& d = *def_;
    // Core range only: the lookback head was fed by the previous chunk, and
    // the post-core tail is not complete yet (it is fed by the next chunk).
    auto core = core_range(c);
    for (std::size_t i = core.first; i < core.second; ++i) {
      const ScalarHit& h = c.hits[i];
      int k = d.trigger_index(h.module, h.channel);
      if (k < 0) continue;
      if (open_ && h.timestamp_ns <= cur_.first_t + d.group_ns) {
        join(h, k);
      } else {
        if (open_) refs_.push_back(cur_);
        start(h, k);
      }
    }
    if (open_ && cur_.first_t + d.group_ns < c.core_end) {  // span fully seen
      refs_.push_back(cur_);
      open_ = false;
    }
    const double margin = d.pre_ns + d.post_ns;
    const double frontier = open_ ? cur_.first_t : c.core_end;
    double ev_hi = frontier - margin;  // +inf at the flush
    if (ev_hi < ev_lo_) ev_hi = ev_lo_;  // monotone by construction; keep the chain
    p.ev_lo = ev_lo_;
    p.ev_hi = ev_hi;
    p.refs = refs_;
    ev_lo_ = ev_hi;
    // The next chunk competes hits against references down to its
    // ev_lo - pre - post; anything older can never matter again.
    auto keep = std::lower_bound(refs_.begin(), refs_.end(), ev_lo_ - margin,
                                 [](const GroupRef& r, double v) { return r.t < v; });
    refs_.erase(refs_.begin(), keep);
    return p;
  }

 private:
  void start(const ScalarHit& h, int k) {
    open_ = true;
    cur_ = GroupRef();
    cur_.t = cur_.first_t = cur_.last_t = h.timestamp_ns;
    cur_.mod = h.module;
    cur_.ch = h.channel;
    cur_.trig_idx = static_cast<uint8_t>(k);
    cur_.n_triggers = 1;
    cur_prio_ = def_->triggers[static_cast<std::size_t>(k)].priority;
  }

  // Members arrive in time order, so "earlier" only decides exact time ties —
  // broken by (module, channel) so the reference never depends on the order
  // std::sort left equal timestamps in.
  void join(const ScalarHit& h, int k) {
    cur_.last_t = h.timestamp_ns;
    ++cur_.n_triggers;
    int prio = def_->triggers[static_cast<std::size_t>(k)].priority;
    bool better = prio < cur_prio_ ||
                  (prio == cur_prio_ && h.timestamp_ns == cur_.t &&
                   mc_key(h.module, h.channel) < mc_key(cur_.mod, cur_.ch));
    if (!better) return;
    cur_.t = h.timestamp_ns;
    cur_.mod = h.module;
    cur_.ch = h.channel;
    cur_.trig_idx = static_cast<uint8_t>(k);
    cur_prio_ = prio;
  }

  std::shared_ptr<const EventDef> def_;
  std::vector<GroupRef> refs_;  // decided, ascending t, trimmed per chunk
  GroupRef cur_;                // the open group (valid while open_)
  int cur_prio_ = 0;
  bool open_ = false;
  double ev_lo_ = -std::numeric_limits<double>::infinity();
};

// Rules 3-4, run by a Worker: a pure function of the chunk and its plan, so
// the output does not depend on the worker count. Adds this chunk's counters
// to `stats`.
inline std::vector<GenericEvent> build_generic_events(const SortedChunk& c,
                                                      const EventPlan& p,
                                                      GenericStats& stats) {
  std::vector<GenericEvent> out;
  if (!p.def) return out;
  const EventDef& d = *p.def;
  const std::vector<GroupRef>& refs = p.refs;
  const std::size_t n = refs.size();
  auto t_less = [](const GroupRef& r, double v) { return r.t < v; };
  const std::size_t e0 =
      static_cast<std::size_t>(std::lower_bound(refs.begin(), refs.end(), p.ev_lo, t_less) -
                               refs.begin());
  const std::size_t e1 =
      static_cast<std::size_t>(std::lower_bound(refs.begin(), refs.end(), p.ev_hi, t_less) -
                               refs.begin());
  out.resize(e1 - e0);
  for (std::size_t i = e0; i < e1; ++i) {
    GenericEvent& e = out[i - e0];
    e.t = refs[i].t;
    e.mod = refs[i].mod;
    e.ch = refs[i].ch;
    e.trig_idx = refs[i].trig_idx;
    e.n_triggers = refs[i].n_triggers;
  }
  auto in_window = [&](std::size_t j, double t) {
    double dt = t - refs[j].t;
    return dt >= -d.pre_ns && dt <= d.post_ns;
  };

  for (const ScalarHit& h : c.hits) {
    const double t = h.timestamp_ns;
    const int k = d.trigger_index(h.module, h.channel);
    std::size_t ev = n;  // n = no event
    bool contested = false;
    if (k >= 0) {
      // Spans are disjoint and ordered: the last group starting at or before t.
      auto it = std::upper_bound(refs.begin(), refs.end(), t,
                                 [](double v, const GroupRef& r) { return v < r.first_t; });
      if (it != refs.begin() && t <= (it - 1)->last_t)
        ev = static_cast<std::size_t>(it - 1 - refs.begin());
    } else {
      // Window validity is monotone along each direction (a farther reference
      // on the same side is out of window if the nearer one is), so the
      // nearest in-window reference is the predecessor or the successor, and
      // "inside two windows" needs at most the second one on each side.
      std::size_t s = static_cast<std::size_t>(
          std::lower_bound(refs.begin(), refs.end(), t, t_less) - refs.begin());
      bool pred = s > 0 && in_window(s - 1, t);
      bool succ = s < n && in_window(s, t);
      if (pred && succ)
        ev = (refs[s].t - t < t - refs[s - 1].t) ? s : s - 1;  // tie -> earlier
      else if (pred)
        ev = s - 1;
      else if (succ)
        ev = s;
      contested = (pred && succ) || (pred && s > 1 && in_window(s - 2, t)) ||
                  (succ && s + 1 < n && in_window(s + 1, t));
    }
    if (ev == n) {
      if (t >= p.ev_lo && t < p.ev_hi) ++stats.orphans;
      continue;
    }
    if (ev < e0 || ev >= e1) continue;  // another chunk emits that event
    GenericEvent& e = out[ev - e0];
    EventHit eh;
    eh.mod = h.module;
    eh.ch = h.channel;
    eh.energy = h.energy;
    eh.energy_short = h.energy_short;
    eh.dt = t - e.t;
    eh.is_trigger = k >= 0;
    e.hits.push_back(eh);
    ++stats.hits_in_events;
    if (contested) ++stats.contested;
  }

  stats.events += out.size();
  for (const GenericEvent& e : out) {
    uint32_t m = e.n_triggers < 8 ? e.n_triggers : 8;
    if (m > 0) ++stats.ntrig[m - 1];
  }
  return out;
}

// ---------------------------------------------------------------------------
// 6. Pipeline message types (in-band control markers)
// ---------------------------------------------------------------------------
//
// Control flows THROUGH the data path with sequence numbers, never around it:
// the Writer's SeqReorder then guarantees that a RunClose is processed only
// after every chunk of that run is written (a barrier for free) and that
// Shutdown pills drain all real work first.

enum class Ctrl : uint8_t { None, RunOpen, RunClose, Shutdown };

// Frozen per-run metadata riding the RunOpen marker to the Writer (the only
// thread that may touch the TFile). In-band, so it is seq-ordered with the
// run's chunks. Plain strings keep this core ignorant of what they describe;
// empty = nothing to record.
struct RunProvenance {
  std::string time_offsets_meta;    // provenance JSON -> TTree UserInfo TNamed "timeSettings"
  std::string time_offsets_source;  // verbatim file applied -> <stem>_timeSettings.json
  std::string event_def_meta;       // provenance JSON -> events-tree UserInfo TNamed "eventDef"
  std::string event_def_source;     // verbatim file applied -> <stem>_eventDef.json
};

// Receiver -> Sorter. None carries a decoded batch; markers carry no hits.
struct SorterMsg {
  Ctrl ctrl = Ctrl::None;
  std::vector<ScalarHit> hits;
  uint32_t run_number = 0;  // RunClose: from the EOS envelope
  RunProvenance prov;       // RunOpen only
  // RunOpen only: the event definition frozen for this run (null = no events
  // tree). The Sorter resets its TriggerGrouper + lookback from it.
  std::shared_ptr<const EventDef> event_def;
};

// Sorter -> Workers -> Writer. Every message (data AND control) gets a seq.
struct WorkMsg {
  Ctrl ctrl = Ctrl::None;
  uint64_t seq = 0;
  SortedChunk chunk;             // None: the sorted chunk
  std::vector<BuiltEvent> built; // filled by a worker when building is on
  EventPlan plan;                // None: Sorter's grouping result (plan.def null = off)
  std::vector<GenericEvent> events;  // filled by a worker when plan.def is set
  GenericStats gstats;           // the worker's counters for `events`
  uint32_t run_number = 0;       // RunClose
  RunProvenance prov;            // RunOpen only
};

// Receiver -> Display (bounded tee). Data may be dropped-and-counted; control
// markers must use the blocking push (see Channel notes).
struct DisplayMsg {
  Ctrl ctrl = Ctrl::None;
  std::vector<ScalarHit> hits;
};

}  // namespace eb
}  // namespace rootsink

#endif  // ROOTSINK_EB_CORE_HPP
