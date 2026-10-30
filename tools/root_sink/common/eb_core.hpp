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
//     context (the generic event layer, section 7) scan the full chunk. Do
//     not "fix" the overlap.
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
// 7. Generic event layer (TODO 71 Phase A) — "まとめる層"
// ---------------------------------------------------------------------------
//
// Bundle hits around triggers for EVERY experiment the same way; what an event
// means (XY, addback, E-dE) is the per-experiment reconstruction layer on top
// (thgem_reco.hpp, run offline, is the first such layer). Rules, parameterized by
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
// Phase C (TODO/71_phase_c_proposal.md) rides on the same chunks, still pure
// per (chunk, plan): the AC flag of a detector hit looks for a shield hit in
// [t - ac_pre, t + ac_post] of THAT HIT over the whole chunk (events play no
// part — a shield hit that rule 4 gave to a neighbour, or to no event, still
// counts), and L2 (counters -> flags -> accepts) is evaluated per event and
// only flags it. AC widens the contract on both sides: ev_hi loses another
// ac_post (shield hits up to ev_hi + post + ac_post must be complete) and the
// lookback gains ac_pre + ac_post (EventDef::lookback_ns()).
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
  bool with_ac = false;     // Phase C: an AC shield fired within the AC window of this hit
};

struct GenericEvent {
  double t = 0.0;  // reference time (aligned, absolute)
  uint8_t mod = 0, ch = 0, trig_idx = 0;
  uint32_t n_triggers = 0;
  std::vector<EventHit> hits;  // time order; includes the reference itself
  // Phase C L2 (empty / 0 when the definition has no l2 section):
  std::vector<uint32_t> l2_counts;  // per EventDef::counters
  uint64_t l2_flags = 0;            // bit i = flags[i] true
  uint64_t l2_accepts = 0;          // bit i = accepts[i] true
  bool l2_accept = false;           // OR of all accepts
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
  // Phase C: hits in emitted events with HitWithAC; events per true flag /
  // accept (index = EventDef::flags / accepts); events with L2Accept.
  uint64_t ac_flagged = 0;
  std::array<uint64_t, 64> flag_true{};
  std::array<uint64_t, 64> accept_true{};
  uint64_t l2_accepted = 0;
  void add(const GenericStats& o) {
    events += o.events;
    hits_in_events += o.hits_in_events;
    contested += o.contested;
    orphans += o.orphans;
    for (std::size_t i = 0; i < ntrig.size(); ++i) ntrig[i] += o.ntrig[i];
    ac_flagged += o.ac_flagged;
    for (std::size_t i = 0; i < flag_true.size(); ++i) flag_true[i] += o.flag_true[i];
    for (std::size_t i = 0; i < accept_true.size(); ++i) accept_true[i] += o.accept_true[i];
    l2_accepted += o.l2_accepted;
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
    // With AC, an event's last hits look ac_post further ahead for shield
    // hits, which must be complete too (below core_end).
    double ev_hi = frontier - margin - d.ac_lookahead_ns();  // +inf at the flush
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

// L2 (Phase C): counters -> flags -> accepts over ONE event's hits, after AC.
// Flag-only: the event is always kept; L2Accept says whether it passed.
inline void apply_l2(const EventDef& d, GenericEvent& e, GenericStats& stats) {
  if (!d.has_l2()) return;
  e.l2_counts.assign(d.counters.size(), 0);
  for (const EventHit& h : e.hits) {
    uint64_t m = d.counter_mask[mc_key(h.mod, h.ch)];
    for (std::size_t i = 0; m; ++i, m >>= 1) {
      if (!(m & 1)) continue;
      L2Ac ac = d.counters[i].ac;
      if (ac == L2Ac::Any || (ac == L2Ac::Clean && !h.with_ac) ||
          (ac == L2Ac::Vetoed && h.with_ac))
        ++e.l2_counts[i];
    }
  }
  e.l2_flags = 0;
  for (std::size_t i = 0; i < d.flags.size(); ++i) {
    const L2Flag& f = d.flags[i];
    const long long v = e.l2_counts[static_cast<std::size_t>(f.counter)];
    bool b = false;
    switch (f.op) {
      case L2Op::Eq: b = v == f.value; break;
      case L2Op::Ne: b = v != f.value; break;
      case L2Op::Lt: b = v < f.value; break;
      case L2Op::Le: b = v <= f.value; break;
      case L2Op::Gt: b = v > f.value; break;
      case L2Op::Ge: b = v >= f.value; break;
    }
    if (b) {
      e.l2_flags |= uint64_t(1) << i;
      ++stats.flag_true[i];
    }
  }
  e.l2_accepts = 0;
  for (std::size_t i = 0; i < d.accepts.size(); ++i) {
    const L2Accept& a = d.accepts[i];
    bool b = a.is_and;
    for (int f : a.flags) {
      bool fb = (e.l2_flags >> f) & 1;
      b = a.is_and ? (b && fb) : (b || fb);
    }
    if (b) {
      e.l2_accepts |= uint64_t(1) << i;
      ++stats.accept_true[i];
    }
  }
  e.l2_accept = e.l2_accepts != 0;
  if (e.l2_accept) ++stats.l2_accepted;
}

// Rules 3-4, run by a Worker: a pure function of the chunk and its plan, so
// the output does not depend on the worker count. Adds this chunk's counters
// to `stats`. Phase C rides along: the AC flag of every hit placed in an
// emitted event, then L2 per event.
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
  // AC: per pair, the sorted times of its shield hits in this chunk (the
  // chunk is time-ordered, so they come out sorted). A detector hit is
  // flagged when any of its pairs has a shield hit in
  // [t - ac_pre, t + ac_post] — measured from the hit, events play no part.
  std::vector<std::vector<double>> shield_t;
  if (d.has_ac()) {
    shield_t.resize(d.ac_pairs.size());
    for (const ScalarHit& h : c.hits) {
      uint64_t m = d.ac_shield_mask[mc_key(h.module, h.channel)];
      for (std::size_t i = 0; m; ++i, m >>= 1)
        if (m & 1) shield_t[i].push_back(h.timestamp_ns);
    }
  }
  auto ac_flag = [&](const ScalarHit& h) {
    uint64_t m = d.ac_det_mask[mc_key(h.module, h.channel)];
    for (std::size_t i = 0; m; ++i, m >>= 1) {
      if (!(m & 1)) continue;
      const std::vector<double>& v = shield_t[i];
      auto it = std::lower_bound(v.begin(), v.end(), h.timestamp_ns - d.ac_pre_ns);
      if (it != v.end() && *it <= h.timestamp_ns + d.ac_post_ns) return true;
    }
    return false;
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
    eh.with_ac = d.has_ac() && ac_flag(h);
    e.hits.push_back(eh);
    ++stats.hits_in_events;
    if (contested) ++stats.contested;
    if (eh.with_ac) ++stats.ac_flagged;
  }

  for (GenericEvent& e : out) apply_l2(d, e, stats);
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
  EventPlan plan;                // None: Sorter's grouping result (plan.def null = off)
  std::vector<GenericEvent> events;  // filled by a worker when plan.def is set
  GenericStats gstats;           // the worker's counters for `events`
  uint32_t run_number = 0;       // RunClose
  RunProvenance prov;            // RunOpen only
  // RunOpen only: the run's frozen event definition, so the Writer can create
  // the definition-named L2 branches (null = no events tree this run).
  std::shared_ptr<const EventDef> event_def;
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
