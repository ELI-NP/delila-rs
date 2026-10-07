// thgem_reco.hpp — the ThGEM reconstruction layer on top of the generic
// events (TODO 71 Phase B; the in-sink `built` tree was retired in favour of it).
//
// root_sink records the experiment-independent `events` tree (eb_core.hpp
// section 7); turning an event into ThGEM quantities — per-plane x/y from the
// delay-line arms, plane-to-plane dt, TOF against LaBr3 — is this layer's job,
// run OFFLINE by thgem_events.C, which writes a tree named `built` with the
// same branches the sink used to write, so existing analysis macros keep
// working on its output. Pure C++17 on top of eb_core.hpp, no ROOT; unit-tested
// in test_eb_core.cpp.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef ROOTSINK_THGEM_RECO_HPP
#define ROOTSINK_THGEM_RECO_HPP

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>
#include <vector>

#include "eb_core.hpp"  // GenericEvent, EventHit

namespace rootsink {
namespace eb {

// ---------------------------------------------------------------------------
// 1. The ThGEM row: BuiltEvent + BuilderConfig + matching helpers
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
// Plane pairing: the two planes of one row are the ThGEM1 / ThGEM2 triggers
// of ONE generic event (one trigger group, TODO 71 rule 1) — the earliest of
// each plane. The row's anchor is the EARLIER of the two; dt_trig is ALWAYS
// t(det2) - t(det1) regardless of which side anchored.

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

// The ThGEM channel columns, filled hit by hit in time order (so each column
// stays sorted). Shared by build_events_from_chunk and thgem_from_event.
struct ThgemViews {
  ChView t1, xl1, xr1, yu1, yd1;
  ChView t2, xl2, xr2, yu2, yd2;
  ChView labr;
  void add(int ch, double t, uint16_t e, const BuilderConfig& cfg) {
    ChView* v = nullptr;
    if (ch == cfg.trig1) v = &t1;
    else if (ch == cfg.xl1) v = &xl1;
    else if (ch == cfg.xr1) v = &xr1;
    else if (ch == cfg.yu1) v = &yu1;
    else if (ch == cfg.yd1) v = &yd1;
    else if (ch == cfg.trig2) v = &t2;
    else if (ch == cfg.xl2) v = &xl2;
    else if (ch == cfg.xr2) v = &xr2;
    else if (ch == cfg.yu2) v = &yu2;
    else if (ch == cfg.yd2) v = &yd2;
    else if (ch == cfg.labr_ch) v = &labr;
    if (!v) return;
    v->t.push_back(t);
    v->e.push_back(e);
  }
};

// Match one detector side of an event: arms nearest to that plane's own
// trigger, x/y from complete axes, energy sum over present arms.
inline void fill_plane(BuiltEvent& ev, int plane, double trig_t, uint16_t trig_e,
                       const ThgemViews& v, double window) {
  const bool p1 = plane == 1;
  const ChView& xl = p1 ? v.xl1 : v.xl2;
  const ChView& xr = p1 ? v.xr1 : v.xr2;
  const ChView& yu = p1 ? v.yu1 : v.yu2;
  const ChView& yd = p1 ? v.yd1 : v.yd2;
  float& dxl = p1 ? ev.dt_xl1 : ev.dt_xl2;
  float& dxr = p1 ? ev.dt_xr1 : ev.dt_xr2;
  float& dyu = p1 ? ev.dt_yu1 : ev.dt_yu2;
  float& dyd = p1 ? ev.dt_yd1 : ev.dt_yd2;
  uint16_t& exl = p1 ? ev.e_xl1 : ev.e_xl2;
  uint16_t& exr = p1 ? ev.e_xr1 : ev.e_xr2;
  uint16_t& eyu = p1 ? ev.e_yu1 : ev.e_yu2;
  uint16_t& eyd = p1 ? ev.e_yd1 : ev.e_yd2;
  uint32_t& e_sum = p1 ? ev.e_sum1 : ev.e_sum2;
  uint8_t& n_arms = p1 ? ev.n_arms1 : ev.n_arms2;
  (p1 ? ev.trig1_t : ev.trig2_t) = trig_t;
  (p1 ? ev.e_trig1 : ev.e_trig2) = trig_e;
  double d = 0.0;
  uint16_t en = 0;
  if (nearest_in(xl, trig_t, window, d, en)) {
    dxl = static_cast<float>(d); exl = en; e_sum += en; ++n_arms;
  }
  if (nearest_in(xr, trig_t, window, d, en)) {
    dxr = static_cast<float>(d); exr = en; e_sum += en; ++n_arms;
  }
  if (nearest_in(yu, trig_t, window, d, en)) {
    dyu = static_cast<float>(d); eyu = en; e_sum += en; ++n_arms;
  }
  if (nearest_in(yd, trig_t, window, d, en)) {
    dyd = static_cast<float>(d); eyd = en; e_sum += en; ++n_arms;
  }
  if (!std::isnan(dxl) && !std::isnan(dxr)) (p1 ? ev.x1 : ev.x2) = dxr - dxl;
  if (!std::isnan(dyu) && !std::isnan(dyd)) (p1 ? ev.y1 : ev.y2) = dyu - dyd;
}

// Both planes filled: plane-to-plane dt, then ONE gamma per event — the
// nearest LaBr3 hit to the anchor (the earlier trigger); both TOFs come from
// that same hit, so tof2 - tof1 == dt_trig by construction.
inline void fill_cross(BuiltEvent& ev, bool has1, bool has2, double anchor, const ThgemViews& v,
                       double window) {
  if (has1 && has2) ev.dt_trig = static_cast<float>(ev.trig2_t - ev.trig1_t);
  double dl = 0.0;
  uint16_t el = 0;
  if (nearest_in(v.labr, anchor, window, dl, el)) {
    ev.labr_t = anchor + dl;
    ev.e_labr = el;
    if (has1) ev.tof1 = static_cast<float>(ev.trig1_t - ev.labr_t);
    if (has2) ev.tof2 = static_cast<float>(ev.trig2_t - ev.labr_t);
  }
}

}  // namespace detail

// ---------------------------------------------------------------------------
// 2. One ThGEM row from one generic event
// ---------------------------------------------------------------------------
//
// One BuiltEvent from one GenericEvent (eb_core.hpp section 7), with exactly
// the retired chunk builder's matching rules (detail::fill_plane / fill_cross: per plane the arm hit
// nearest to that plane's own trigger within ±window_ns, ties to the
// successor; one LaBr3 hit nearest to the anchor = the earlier plane trigger).
// Only the CANDIDATES change: the event's own hits instead of the whole chunk,
// so a hit is never used by two rows (rule 4 gave it to its nearest reference).
//
// It reproduced the retired chunk builder row by row (run0023: 15,644 four-
// fold, all 40,045 rows; run 56: all 33,463 rows — TODO 71 §9) when no hit is
// contested AND the event window covers group_ns + window_ns on both sides (a
// plane-2 trigger can sit up to G from the reference, its arms up to
// window_ns from that trigger). Two triggers of the SAME plane in one group
// make ONE row here (the earliest); the chunk builder made two.
// Returns false when the event holds no ThGEM trigger (e.g. a definition that
// also triggers on LaBr3).
inline bool thgem_from_event(const GenericEvent& ev, const BuilderConfig& cfg, BuiltEvent& out) {
  detail::ThgemViews v;
  for (const EventHit& h : ev.hits) v.add(static_cast<int>(h.ch), ev.t + h.dt, h.energy, cfg);
  const bool has1 = !v.t1.t.empty();
  const bool has2 = cfg.trig2 >= 0 && !v.t2.t.empty();
  if (!has1 && !has2) return false;
  out = BuiltEvent();
  if (has1) detail::fill_plane(out, 1, v.t1.t[0], v.t1.e[0], v, cfg.window_ns);
  if (has2) detail::fill_plane(out, 2, v.t2.t[0], v.t2.e[0], v, cfg.window_ns);
  double anchor = !has2 ? v.t1.t[0] : !has1 ? v.t2.t[0] : std::min(v.t1.t[0], v.t2.t[0]);
  detail::fill_cross(out, has1, has2, anchor, v, cfg.window_ns);
  return true;
}

}  // namespace eb
}  // namespace rootsink

#endif  // ROOTSINK_THGEM_RECO_HPP
