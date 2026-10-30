// bunch_clock.hpp — beam-bunch times rebuilt from a downscaled beam pulse
// (TODO 73 §3.3, TODO 79 §5.2).
//
// The beam is bunched at the accelerator RF (5.0005 MHz, T = 199.980 ns on
// pf-qaq run 217). Only every N-th pulse is recorded (1/100 → one beam hit per
// ~20 µs), so every other bunch is inferred: from the beam hit b nearest to a
// time t, the bunches are b + n·T. On run 217 the bunch phase stayed within
// ~2.5 ns over 4 µs and ~2.7 ns over 40 µs, so the nearest beam hit (at most
// half a downscale period away) pins every bunch to a few ns.
//
// T is measured per run: every gap between consecutive beam hits is k·T for an
// integer k (k = round(gap / T_current)); T = Σgap / Σk once `min_gaps` gaps
// agree. A gap that is not close to a multiple of T (|gap − k·T| > T/4) is
// counted as off-grid and ignored; a gap longer than `max_gap_ns` (beam off)
// is ignored too, and no bunch is inferred that far from a beam hit.
//
// Input must arrive in time order (nwall_sink feeds it the Sorter's sorted
// chunks); a late beam hit is inserted in place. Beam hits are kept until the
// caller says they are no longer needed (forget_before): nwall_sink adds a
// whole chunk's beam hits before it looks any bunch up, and a chunk can span
// more than a second. A count cap (max_beam_hits) only guards memory against
// a runaway beam channel and is counted when it bites. Pure C++17, no ROOT:
// unit-tested in test_nwall.cpp.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef NWALL_BUNCH_CLOCK_HPP
#define NWALL_BUNCH_CLOCK_HPP

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <deque>

namespace nwall {

class BunchClock {
 public:
  struct Config {
    double period_ns = 199.980;  // nominal RF period (start value of the estimate)
    double max_gap_ns = 1e6;     // no bunch inferred farther than this from a beam hit
    std::size_t max_beam_hits = 2000000;  // memory guard (1/100 at 5 MHz: 40 s of beam)
    std::size_t min_gaps = 10;   // gaps needed before the measured T replaces the nominal
  };

  // (Two constructors, not a `Config c = Config()` default argument: a nested
  // struct's member initializers are not usable in a default argument of the
  // enclosing class.)
  BunchClock() : BunchClock(Config{}) {}
  explicit BunchClock(const Config& c) : cfg_(c), period_(c.period_ns) {}

  // Run start: the digitizer clock restarts, so nothing carries over.
  void reset() {
    beams_.clear();
    period_ = cfg_.period_ns;
    sum_gap_ = 0.0;
    sum_k_ = 0;
    gaps_ = 0;
    off_grid_ = 0;
    capped_ = 0;
  }

  void add_beam(double t) {
    if (!beams_.empty() && t < beams_.back()) {
      beams_.insert(std::upper_bound(beams_.begin(), beams_.end(), t), t);
    } else {
      if (!beams_.empty()) measure_gap(t - beams_.back());
      beams_.push_back(t);
    }
    while (beams_.size() > cfg_.max_beam_hits) {
      beams_.pop_front();
      ++capped_;
    }
  }

  // Drop the beam hits before t (they are no longer needed by the caller).
  void forget_before(double t) {
    while (!beams_.empty() && beams_.front() < t) beams_.pop_front();
  }

  double period_ns() const { return period_; }
  bool period_measured() const { return gaps_ >= cfg_.min_gaps; }
  std::size_t gaps() const { return gaps_; }
  std::size_t off_grid() const { return off_grid_; }
  std::size_t beam_hits() const { return beams_.size(); }
  std::size_t capped() const { return capped_; }  // dropped by the memory guard

  // The bunch at or before t (the one a particle seen at t came from, if it
  // took less than one period): b + floor((t − b) / T)·T.
  bool bunch_at_or_before(double t, double& tb) const {
    double b;
    if (!nearest_beam(t, b)) return false;
    tb = b + std::floor((t - b) / period_) * period_;
    return true;
  }

  // The bunch nearest to t: b + round((t − b) / T)·T.
  bool nearest_bunch(double t, double& tb) const {
    double b;
    if (!nearest_beam(t, b)) return false;
    tb = b + std::round((t - b) / period_) * period_;
    return true;
  }

 private:
  void measure_gap(double gap) {
    if (gap <= 0.0 || gap > cfg_.max_gap_ns) return;
    const double k = std::round(gap / period_);
    if (k < 1.0 || std::fabs(gap - k * period_) > period_ / 4.0) {
      ++off_grid_;
      return;
    }
    sum_gap_ += gap;
    sum_k_ += static_cast<uint64_t>(k);
    ++gaps_;
    if (gaps_ >= cfg_.min_gaps) period_ = sum_gap_ / static_cast<double>(sum_k_);
  }

  bool nearest_beam(double t, double& b) const {
    if (beams_.empty()) return false;
    auto it = std::lower_bound(beams_.begin(), beams_.end(), t);
    double best = 0.0;
    bool have = false;
    if (it != beams_.end()) {
      best = *it;
      have = true;
    }
    if (it != beams_.begin()) {
      const double before = *(it - 1);
      if (!have || t - before < best - t) best = before;
      have = true;
    }
    if (!have || std::fabs(t - best) > cfg_.max_gap_ns) return false;
    b = best;
    return true;
  }

  Config cfg_;
  std::deque<double> beams_;
  double period_;
  double sum_gap_ = 0.0;
  uint64_t sum_k_ = 0;
  std::size_t gaps_ = 0;
  std::size_t off_grid_ = 0;
  std::size_t capped_ = 0;
};

}  // namespace nwall

#endif  // NWALL_BUNCH_CLOCK_HPP
