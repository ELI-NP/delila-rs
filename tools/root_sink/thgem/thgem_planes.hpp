// thgem_planes.hpp — the ThGEMs of one built event, for thgem_sink's display
// (TODO 88 §3.3). Any number of ThGEMs, each found through the channel map
// (thgem_map.hpp), so module:channel never has to be given on the command line.
//
// Matching is the rule of thgem_reco.hpp (TODO 71 Phase B), reused as is: per
// ThGEM, the trigger is the EARLIEST trigger hit of that ThGEM in the event,
// and each arm is the hit of that electrode NEAREST to that trigger within
// ±arm_window_ns (inclusive; ties go to the later hit). Candidates are the
// event's own hits only, so a hit is never used by two events. x = t_XR - t_XL,
// y = t_YU - t_YD in ns on the aligned time axis (time offsets, TODO 70, are
// applied before the Sorter); no mm calibration here.
//
// Pure C++17 on top of eb_core.hpp, no ROOT: unit-tested in test_thgem_sink.cpp.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef THGEM_PLANES_HPP
#define THGEM_PLANES_HPP

#include <array>
#include <cmath>
#include <cstdint>
#include <limits>
#include <map>
#include <vector>

#include "../common/eb_core.hpp"  // GenericEvent, EventHit
#include "thgem_map.hpp"
#include "thgem_reco.hpp"  // detail::ChView, detail::nearest_in: the same matching rule

namespace thgem {

constexpr double kNaN = std::numeric_limits<double>::quiet_NaN();

// One ThGEM in one event. Arms index 0..3 = XL, XR, YU, YD (Kind - 1).
struct Plane {
  int number = -1;
  double t_trig = kNaN;  // absolute, aligned ns
  uint16_t e_trig = 0;
  double dt[4] = {kNaN, kNaN, kNaN, kNaN};  // t(arm) - t_trig; NaN = no arm hit
  uint16_t e[4] = {0, 0, 0, 0};
  int n_arms = 0;  // 0..4
  double x = kNaN;  // t_XR - t_XL
  double y = kNaN;  // t_YU - t_YD

  bool has_xy() const { return !std::isnan(x) && !std::isnan(y); }
};

// The ThGEMs whose trigger is in `ev`, ascending by number.
inline std::vector<Plane> planes_from_event(const rootsink::eb::GenericEvent& ev,
                                            const ChannelMap& m, double arm_window_ns) {
  using rootsink::eb::detail::ChView;
  std::map<int, std::array<ChView, kElectrodes>> views;
  for (const rootsink::eb::EventHit& h : ev.hits) {
    const Role* r = m.role(h.mod, h.ch);
    if (!r || r->det != Det::ThGEM) continue;
    ChView& v = views[r->index][static_cast<int>(r->kind)];
    v.t.push_back(ev.t + h.dt);  // the event's hits are in time order, so v stays sorted
    v.e.push_back(h.energy);
  }
  std::vector<Plane> out;
  for (const auto& kv : views) {
    const std::array<ChView, kElectrodes>& v = kv.second;
    const ChView& trig = v[static_cast<int>(Kind::T)];
    if (trig.t.empty()) continue;  // arms without their trigger: not a ThGEM hit
    Plane p;
    p.number = kv.first;
    p.t_trig = trig.t[0];
    p.e_trig = trig.e[0];
    for (int a = 0; a < 4; ++a) {
      double d = 0.0;
      uint16_t en = 0;
      if (rootsink::eb::detail::nearest_in(v[a + 1], p.t_trig, arm_window_ns, d, en)) {
        p.dt[a] = d;
        p.e[a] = en;
        ++p.n_arms;
      }
    }
    if (!std::isnan(p.dt[0]) && !std::isnan(p.dt[1])) p.x = p.dt[1] - p.dt[0];
    if (!std::isnan(p.dt[2]) && !std::isnan(p.dt[3])) p.y = p.dt[2] - p.dt[3];
    out.push_back(p);
  }
  return out;
}

// The dt pair values of one event: for every two ThGEMs i < j of `planes`,
// dt_i = t_trig(i) - tb and dt_j = t_trig(j) - tb against ONE bunch time tb
// (the bunch the event's trigger picked), so both sit on a common reference.
struct PairDt {
  int i = -1, j = -1;
  double dt_i = kNaN, dt_j = kNaN;
};

inline std::vector<PairDt> pair_dts(const std::vector<Plane>& planes, double tb) {
  std::vector<PairDt> out;
  for (std::size_t a = 0; a < planes.size(); ++a)
    for (std::size_t b = a + 1; b < planes.size(); ++b) {
      const Plane& p = planes[a].number < planes[b].number ? planes[a] : planes[b];
      const Plane& q = planes[a].number < planes[b].number ? planes[b] : planes[a];
      out.push_back({p.number, q.number, p.t_trig - tb, q.t_trig - tb});
    }
  return out;
}

}  // namespace thgem

#endif  // THGEM_PLANES_HPP
