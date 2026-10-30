// event_def.hpp — the generic event definition for root_sink (TODO 71 Phase A).
//
// What it is: the JSON file (--event-def / TOML event_def_file) that tells the
// generic grouping layer which channels are triggers, how triggers are bundled
// into one event, which trigger becomes the event's reference, and the window
// around that reference:
//
//   {"version": "1.0",
//    "window_ns": {"pre": 1000, "post": 1000},
//    "group_ns": 1000,
//    "triggers": [
//      {"name": "ThGEM1", "priority": 1, "channels": [[0, 1]]},
//      {"name": "ThGEM2", "priority": 2, "channels": [[0, 6]]}]}
//
// Rules it parameterizes (TODO/71_generic_event_building.md §4; the grouping
// and assignment code lives in eb_core.hpp, section 7):
//   1. Triggers in time order; an ungrouped trigger t opens the group [t, t+G]
//      (G = group_ns, NOT extended by later members). Every trigger belongs to
//      exactly one group.
//   2. The group's reference = its member with the best (smallest) priority;
//      ties -> earliest, then smallest (module, channel). No priorities in the
//      file -> all equal -> the first trigger of the group.
//   3. Window = [t_ref - pre, t_ref + post]. G <= min(pre, post), so every
//      member of a group lies inside its own event's window wherever the
//      reference sits in the group.
//   4. A non-trigger hit inside two windows goes to the NEAREST reference
//      (equal distance -> the earlier one); a hit in no window is in no event.
//
// Phase C ("version": "1.1", TODO/71_phase_c_proposal.md §2.1; every section
// optional, so a 1.0 file reads unchanged):
//   "tags": {"core": [[0, 0], [1, 0]], "seg": [[0, 2, 9]], ...}
//       tag -> channels; [m, first, last] is an inclusive range.
//   triggers[i].tags: ["core"]      union with `channels` (channels first).
//   "anti_compton": {"window_ns": {"pre", "post"},
//                    "pairs": [{"name", "detector": [tags], "shield": [tags]}]}
//       window relative to the flagged detector hit, independent of events.
//   "l2": {"counters": [{"name", "tags", "ac": "any"|"clean"|"vetoed"}],
//          "flags":    [{"name", "counter", "op": "==" ... ">=", "value"}],
//          "accept":   [{"name", "op": "AND"|"OR", "flags": [...]}]}
//       names are events-tree branches: C identifiers, unique, not fixed ones.
// Every reference (tag, counter, flag) is resolved at load time: an unknown
// name is a load error, never a silent "false" at run time.
//
// Times are on the root_sink ALIGNED axis (time_offsets.hpp is applied before
// the Sorter), so the windows here are in aligned ns.
//
// Pure C++17 + TDelila.hpp's JSON parser: no ROOT, no ZMQ, unit-tested in
// test_sink_core.cpp.
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef ROOTSINK_EVENT_DEF_HPP
#define ROOTSINK_EVENT_DEF_HPP

#include <climits>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <vector>

#include "time_offsets.hpp"  // mc_key, mc_name, json_escape, toff_detail helpers

namespace rootsink {

struct TriggerDef {
  std::string name;
  int priority = 0;               // smaller = preferred as the reference
  std::vector<uint16_t> channels; // mc_key(module, channel), file order
};

// Anti-Compton pair (Phase C, TODO/71_phase_c_proposal.md §2.4): a hit on a
// `detector` channel gets HitWithAC when a `shield` hit lies within
// [t - ac_pre_ns, t + ac_post_ns] of the HIT ITSELF — independent of events, so
// a shield hit that rule 4 gave to a neighbouring event (or to none) still
// counts. Nothing is discarded.
struct AcPair {
  std::string name;
  std::vector<uint16_t> detector;  // mc_key, resolved from tags
  std::vector<uint16_t> shield;
};

// L2 (Phase C): counters -> flags -> accepts, evaluated per event, flag-only
// (no event is ever dropped). Names become events-tree branches.
enum class L2Ac : uint8_t { Any, Clean, Vetoed };  // count all / !with_ac / with_ac
enum class L2Op : uint8_t { Eq, Ne, Lt, Le, Gt, Ge };
struct L2Counter {
  std::string name;
  std::vector<uint16_t> channels;  // mc_key, resolved from tags; counts HITS
  L2Ac ac = L2Ac::Any;
};
struct L2Flag {
  std::string name;
  int counter = 0;  // index into EventDef::counters
  L2Op op = L2Op::Gt;
  long long value = 0;
};
struct L2Accept {
  std::string name;
  bool is_and = true;      // AND of its flags, else OR
  std::vector<int> flags;  // indices into EventDef::flags
};

struct EventDef {
  double pre_ns = 0.0;    // window before the reference
  double post_ns = 0.0;   // window after the reference
  double group_ns = 0.0;  // G: non-extending trigger grouping span
  std::vector<TriggerDef> triggers;
  bool priorities_given = false;  // false: every priority is 0 (rule 2 -> first trigger)

  // Phase C (all optional; empty = Phase A behaviour).
  std::map<std::string, std::vector<uint16_t>> tags;  // resolved tag -> channels (file order)
  double ac_pre_ns = 0.0, ac_post_ns = 0.0;  // AC window relative to the flagged hit
  std::vector<AcPair> ac_pairs;               // <= 64
  std::vector<L2Counter> counters;            // <= 64
  std::vector<L2Flag> flags;                  // <= 64
  std::vector<L2Accept> accepts;              // <= 64

  // mc_key -> index into `triggers`, -1 = not a trigger channel. 65536 entries
  // once index_channels() ran; empty = no trigger at all.
  std::vector<int16_t> trig_of;
  // mc_key -> bit i set = detector / shield of ac_pairs[i]; counted by
  // counters[i]. 65536 entries each, built by index_channels() only when used.
  std::vector<uint64_t> ac_det_mask, ac_shield_mask, counter_mask;

  // Fill the lookup tables from `triggers`, `ac_pairs`, `counters`. Call after
  // they are final (the parser does; tests building an EventDef by hand must
  // too). Duplicate trigger channels are a parse error, so here the last
  // writer simply wins.
  void index_channels() {
    trig_of.assign(65536, -1);
    for (std::size_t i = 0; i < triggers.size(); ++i)
      for (uint16_t k : triggers[i].channels) trig_of[k] = static_cast<int16_t>(i);
    ac_det_mask.clear();
    ac_shield_mask.clear();
    if (!ac_pairs.empty()) {
      ac_det_mask.assign(65536, 0);
      ac_shield_mask.assign(65536, 0);
      for (std::size_t i = 0; i < ac_pairs.size(); ++i) {
        for (uint16_t k : ac_pairs[i].detector) ac_det_mask[k] |= uint64_t(1) << i;
        for (uint16_t k : ac_pairs[i].shield) ac_shield_mask[k] |= uint64_t(1) << i;
      }
    }
    counter_mask.clear();
    if (!counters.empty()) {
      counter_mask.assign(65536, 0);
      for (std::size_t i = 0; i < counters.size(); ++i)
        for (uint16_t k : counters[i].channels) counter_mask[k] |= uint64_t(1) << i;
    }
  }

  int trigger_index(uint8_t module, uint8_t channel) const {
    return trig_of.empty() ? -1 : trig_of[mc_key(module, channel)];
  }

  bool has_ac() const { return !ac_pairs.empty(); }
  bool has_l2() const { return !counters.empty(); }

  // Context the Sorter must carry back across a chunk boundary so that every
  // emitted event sees all of its hits, all competing references and, with
  // AC, every shield hit within the AC window of those hits:
  // G + 2*pre + post (+ ac_pre + ac_post) (derivation: eb_core.hpp section 7,
  // "chunk contract").
  double lookback_ns() const {
    return group_ns + 2.0 * pre_ns + post_ns + (has_ac() ? ac_pre_ns + ac_post_ns : 0.0);
  }
  // How much earlier than the Phase A bound a chunk must stop emitting, so
  // the AC look-ahead of its last events only reads complete hits.
  double ac_lookahead_ns() const { return has_ac() ? ac_post_ns : 0.0; }
};

struct EventDefParseResult {
  EventDef def;
  std::vector<std::string> errors;    // non-empty => def must not be used
  std::vector<std::string> warnings;  // usable, but tell the operator
};

namespace edef_detail {

using JV = tdelila::json::Value;

inline std::string fmt_g(double v) {
  char buf[32];
  std::snprintf(buf, sizeof buf, "%g", v);
  return buf;
}

// "m0c1" / "m0c1, m0c2" / "m0c0, ..., m0c7 (+32 more)": a range overlap stays
// one readable error line instead of one error per channel.
inline std::string chan_list(const std::vector<uint16_t>& ks, std::size_t cap = 8) {
  std::string s;
  for (std::size_t i = 0; i < ks.size() && i < cap; ++i) s += (i ? ", " : "") + mc_name(ks[i]);
  if (ks.size() > cap) s += " (+" + std::to_string(ks.size() - cap) + " more)";
  return s;
}

// [[m,c],...] for the provenance JSON.
inline void append_chans(std::string& s, const std::vector<uint16_t>& ks) {
  s += '[';
  for (std::size_t i = 0; i < ks.size(); ++i) {
    char buf[24];
    std::snprintf(buf, sizeof buf, "%s[%u,%u]", i ? "," : "", static_cast<unsigned>(ks[i] >> 8),
                  static_cast<unsigned>(ks[i] & 0xFF));
    s += buf;
  }
  s += ']';
}

// Allow-list for a nested object at `where`; `_comment` is accepted everywhere
// (repo convention).
inline void check_keys(const JV& o, const std::string& where,
                       std::initializer_list<const char*> allowed,
                       std::vector<std::string>& errors) {
  for (const auto& kv : o.obj) {
    const std::string& k = kv.first;
    if (k == "_comment") {
      if (!toff_detail::comment_ok(kv.second))
        errors.push_back(where + ": '_comment' must be a string or an array of strings");
      continue;
    }
    bool ok = false;
    std::string expected;
    for (const char* a : allowed) {
      if (k == a) ok = true;
      expected += std::string(expected.empty() ? "'" : ", '") + a + "'";
    }
    if (!ok) errors.push_back(where + ": unknown key '" + k + "' (expected " + expected + ")");
  }
}

// A span in ns: present, a finite number, >= 0. `what` names it in the error.
inline bool get_span(const JV* v, const std::string& what, double& out,
                     std::vector<std::string>& errors) {
  if (!v) {
    errors.push_back("missing '" + what + "' (ns, a number >= 0)");
    return false;
  }
  if (v->t != JV::T::Num || !std::isfinite(v->num) || v->num < 0) {
    errors.push_back("'" + what + "' must be a finite number >= 0 (ns)");
    return false;
  }
  out = v->num;
  return true;
}

// A finite integral number that fits in an int.
inline bool get_int(const JV* v, int& out) {
  if (!v || v->t != JV::T::Num || !std::isfinite(v->num)) return false;
  if (v->num != std::floor(v->num) || v->num < INT_MIN || v->num > INT_MAX) return false;
  out = static_cast<int>(v->num);
  return true;
}

// A finite integral number a double holds exactly (|v| <= 2^53).
inline bool get_ll(const JV* v, long long& out) {
  if (!v || v->t != JV::T::Num || !std::isfinite(v->num)) return false;
  if (v->num != std::floor(v->num) || std::fabs(v->num) > 9007199254740992.0) return false;
  out = static_cast<long long>(v->num);
  return true;
}

// {"pre": ns, "post": ns} at `where` (window_ns, anti_compton.window_ns).
inline bool parse_pre_post(const JV* w, const std::string& where, double& pre, double& post,
                           std::vector<std::string>& errors) {
  if (!w || w->t != JV::T::Obj) {
    errors.push_back("'" + where + "' must be an object {\"pre\": ns, \"post\": ns}");
    return false;
  }
  check_keys(*w, where, {"pre", "post"}, errors);
  bool a = get_span(w->find("pre"), where + ".pre", pre, errors);
  bool b = get_span(w->find("post"), where + ".post", post, errors);
  return a && b;
}

// L2 names become events-tree branches: C identifiers that do not collide with
// the fixed branches (keep in sync with the Writer, root_sink.cxx).
inline bool is_c_ident(const std::string& s) {
  auto head = [](char c) { return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_'; };
  if (s.empty() || !head(s[0])) return false;
  for (char c : s)
    if (!head(c) && !(c >= '0' && c <= '9')) return false;
  return true;
}

inline bool is_fixed_branch(const std::string& s) {
  static const char* const fixed[] = {"EventID",   "TriggerTime", "TriggerMod",     "TriggerCh",
                                      "TriggerIdx", "NTriggers",  "NHits",          "HitMod",
                                      "HitCh",      "HitEnergy",  "HitEnergyShort", "HitTime",
                                      "HitIsTrigger", "HitWithAC", "L2Accept"};
  for (const char* f : fixed)
    if (s == f) return true;
  return false;
}

inline const char* l2_op_name(L2Op op) {
  switch (op) {
    case L2Op::Eq: return "==";
    case L2Op::Ne: return "!=";
    case L2Op::Lt: return "<";
    case L2Op::Le: return "<=";
    case L2Op::Gt: return ">";
    case L2Op::Ge: return ">=";
  }
  return "?";
}

inline bool parse_l2_op(const std::string& s, L2Op& op) {
  for (L2Op o : {L2Op::Eq, L2Op::Ne, L2Op::Lt, L2Op::Le, L2Op::Gt, L2Op::Ge})
    if (s == l2_op_name(o)) {
      op = o;
      return true;
    }
  return false;
}

inline const char* l2_ac_name(L2Ac ac) {
  switch (ac) {
    case L2Ac::Any: return "any";
    case L2Ac::Clean: return "clean";
    case L2Ac::Vetoed: return "vetoed";
  }
  return "?";
}

// Top-level "tags": {"name": [[m, c] | [m, first, last], ...]} -> d.tags, each
// tag's channels in file order. A tag is registered even when broken, so a
// reference to it does not add an "unknown tag" error on top.
inline void parse_tags(const JV& root, EventDef& d, std::vector<std::string>& errors) {
  const JV* tg = root.find("tags");
  if (!tg) return;
  if (tg->t != JV::T::Obj) {
    errors.push_back("'tags' must be an object {\"tag\": [[module, channel] or "
                     "[module, first, last], ...]}");
    return;
  }
  for (const auto& kv : tg->obj) {
    const std::string& name = kv.first;
    if (name == "_comment") {
      if (!toff_detail::comment_ok(kv.second))
        errors.push_back("tags: '_comment' must be a string or an array of strings");
      continue;
    }
    const std::string where = "tags['" + name + "']";
    std::vector<uint16_t>& chs = d.tags[name];
    if (name.empty()) errors.push_back("tags: empty tag name");
    if (kv.second.t != JV::T::Arr || kv.second.arr.empty()) {
      errors.push_back(where + " must be a non-empty array of [module, channel] or "
                               "[module, first, last]");
      continue;
    }
    std::set<uint16_t> seen;
    std::vector<uint16_t> twice;
    for (std::size_t j = 0; j < kv.second.arr.size(); ++j) {
      const JV& p = kv.second.arr[j];
      const std::string at = where + "[" + std::to_string(j) + "]";
      int m = 0, a = 0, b = 0;
      if (p.t != JV::T::Arr || (p.arr.size() != 2 && p.arr.size() != 3) ||
          !toff_detail::get_u8(&p.arr[0], m) || !toff_detail::get_u8(&p.arr[1], a) ||
          (p.arr.size() == 3 && !toff_detail::get_u8(&p.arr[2], b))) {
        errors.push_back(at + " must be [module, channel] or [module, first, last] with "
                              "integers 0..255");
        continue;
      }
      if (p.arr.size() == 2) b = a;
      if (a > b) {
        errors.push_back(at + ": range first " + std::to_string(a) + " > last " +
                         std::to_string(b));
        continue;
      }
      for (int c = a; c <= b; ++c) {
        uint16_t k = mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c));
        if (seen.insert(k).second)
          chs.push_back(k);
        else
          twice.push_back(k);
      }
    }
    if (!twice.empty()) errors.push_back(where + ": " + chan_list(twice) + " listed twice");
  }
}

// `key` of object `o` (at `where`): a non-empty array of tag names. Appends the
// union of their channels to `out` (file order, skipping channels already in
// `out`). Returns false when the key is absent (the caller decides whether
// that is an error); any other problem is reported here.
inline bool resolve_tags(const JV& o, const char* key, const std::string& where,
                         const EventDef& d, std::vector<uint16_t>& out,
                         std::vector<std::string>& errors) {
  const JV* v = o.find(key);
  if (!v) return false;
  const std::string bad = where + ": '" + key + "' must be a non-empty array of tag names";
  if (v->t != JV::T::Arr || v->arr.empty()) {
    errors.push_back(bad);
    return true;
  }
  std::set<uint16_t> have(out.begin(), out.end());
  bool bad_reported = false;
  for (const JV& n : v->arr) {
    if (n.t != JV::T::Str) {
      if (!bad_reported) errors.push_back(bad);
      bad_reported = true;
      continue;
    }
    auto it = d.tags.find(n.str);
    if (it == d.tags.end()) {
      errors.push_back(where + ": unknown tag '" + n.str + "' in '" + key + "'" +
                       (d.tags.empty() ? " (the file has no top-level 'tags' table)" : ""));
      continue;
    }
    for (uint16_t k : it->second)
      if (have.insert(k).second) out.push_back(k);
  }
  return true;
}

// One trigger entry; its errors go to `errors`. `label` is what later messages
// call it ("trigger 'Si'" or, without a valid name, "triggers[3]"). Channels =
// `channels` (file order) then the channels of `tags` not already listed.
inline void parse_trigger(const JV& e, const std::string& where, const EventDef& d,
                          TriggerDef& t, bool& has_priority, std::string& label,
                          std::vector<std::string>& errors) {
  has_priority = false;
  label = where;
  check_keys(e, where, {"name", "priority", "channels", "tags"}, errors);
  const JV* name = e.find("name");
  if (!name || name->t != JV::T::Str || name->str.empty()) {
    errors.push_back(where + ": 'name' must be a non-empty string");
  } else {
    t.name = name->str;
    label = "trigger '" + t.name + "'";
  }
  const JV* prio = e.find("priority");
  if (prio) {
    has_priority = true;
    if (!get_int(prio, t.priority))
      errors.push_back(where + ": 'priority' must be an integer (smaller = preferred reference)");
  }
  const JV* chs = e.find("channels");
  const bool has_tags = e.find("tags") != nullptr;
  if (!chs && !has_tags) {
    errors.push_back(where + ": 'channels' must be a non-empty array of [module, channel], "
                             "or give 'tags'");
  } else if (chs && chs->t != JV::T::Arr) {
    errors.push_back(where + ": 'channels' must be a non-empty array of [module, channel]");
  } else if (chs && chs->arr.empty()) {
    errors.push_back(where + ": 'channels' is empty (omit it, or list at least one channel)");
  } else if (chs) {
    for (std::size_t j = 0; j < chs->arr.size(); ++j) {
      const JV& p = chs->arr[j];
      int m = 0, c = 0;
      if (p.t != JV::T::Arr || p.arr.size() != 2 || !toff_detail::get_u8(&p.arr[0], m) ||
          !toff_detail::get_u8(&p.arr[1], c)) {
        errors.push_back(where + ": channels[" + std::to_string(j) +
                         "] must be [module, channel] with integers 0..255");
        continue;
      }
      t.channels.push_back(mc_key(static_cast<uint8_t>(m), static_cast<uint8_t>(c)));
    }
  }
  resolve_tags(e, "tags", where, d, t.channels, errors);
}

inline void parse_triggers(const JV& root, EventDef& d, std::vector<std::string>& errors) {
  const JV* trg = root.find("triggers");
  if (!trg || trg->t != JV::T::Arr) {
    errors.push_back("'triggers' must be a non-empty array of "
                     "{\"name\", \"priority\" (optional), \"channels\" and/or \"tags\"}");
    return;
  }
  if (trg->arr.empty()) {
    errors.push_back("'triggers' is empty: at least one trigger is needed");
    return;
  }
  if (trg->arr.size() > 256)
    errors.push_back("'triggers' has " + std::to_string(trg->arr.size()) +
                     " entries; at most 256 (TriggerIdx is 8-bit)");

  std::map<std::string, std::size_t> names;      // name -> entry index
  std::map<uint16_t, std::string> owner;         // mc_key -> label of its trigger
  std::vector<std::string> with_prio, without_prio;  // labels, for all-or-none
  for (std::size_t i = 0; i < trg->arr.size(); ++i) {
    const JV& e = trg->arr[i];
    const std::string where = "triggers[" + std::to_string(i) + "]";
    if (e.t != JV::T::Obj) {
      errors.push_back(where + ": not an object");
      continue;
    }
    TriggerDef t;
    bool has_priority = false;
    std::string label;
    parse_trigger(e, where, d, t, has_priority, label, errors);
    if (!t.name.empty()) {
      auto it = names.find(t.name);
      if (it != names.end())
        errors.push_back("triggers[" + std::to_string(it->second) + "] and " + where +
                         ": duplicate name '" + t.name + "'");
      else
        names[t.name] = i;
    }
    (has_priority ? with_prio : without_prio).push_back(label);
    // One channel = one trigger (over channels + tags): a channel in two
    // triggers would make "which trigger is this hit" (TriggerIdx, the
    // priority) depend on file order. One error per trigger pair.
    std::set<uint16_t> mine;
    std::vector<uint16_t> twice;
    std::vector<std::pair<std::string, std::vector<uint16_t>>> clash;  // other label, channels
    for (uint16_t k : t.channels) {
      auto it = owner.find(k);
      if (it == owner.end()) {
        owner[k] = label;
      } else if (mine.count(k)) {
        twice.push_back(k);
      } else {
        std::size_t c = 0;
        while (c < clash.size() && clash[c].first != it->second) ++c;
        if (c == clash.size()) clash.push_back({it->second, {}});
        clash[c].second.push_back(k);
      }
      mine.insert(k);
    }
    if (!twice.empty()) errors.push_back(chan_list(twice) + " listed twice in " + label);
    for (const auto& c : clash)
      errors.push_back(chan_list(c.second) + " listed in both " + c.first + " and " + label);
    d.triggers.push_back(std::move(t));
  }
  if (!with_prio.empty() && !without_prio.empty()) {
    std::string s = "'priority' given for " + std::to_string(with_prio.size()) + " of " +
                    std::to_string(with_prio.size() + without_prio.size()) +
                    " triggers (missing in:";
    for (const std::string& l : without_prio) s += " " + l;
    errors.push_back(s + "): give it for all triggers or for none "
                         "(none = the first trigger of a group is the reference)");
  }
  d.priorities_given = without_prio.empty() && !with_prio.empty();
  if (!d.priorities_given)
    for (TriggerDef& t : d.triggers) t.priority = 0;
}

// "anti_compton": {"window_ns": {"pre", "post"}, "pairs": [{"name", "detector":
// [tags], "shield": [tags]}, ...]}. The window is relative to the flagged hit.
inline void parse_anti_compton(const JV& root, EventDef& d, std::vector<std::string>& errors) {
  const JV* ac = root.find("anti_compton");
  if (!ac) return;
  if (ac->t != JV::T::Obj) {
    errors.push_back("'anti_compton' must be an object {\"window_ns\": {\"pre\", \"post\"}, "
                     "\"pairs\": [...]}");
    return;
  }
  check_keys(*ac, "anti_compton", {"window_ns", "pairs"}, errors);
  parse_pre_post(ac->find("window_ns"), "anti_compton.window_ns", d.ac_pre_ns, d.ac_post_ns,
                 errors);
  const JV* pairs = ac->find("pairs");
  if (!pairs || pairs->t != JV::T::Arr || pairs->arr.empty()) {
    errors.push_back("'anti_compton.pairs' must be a non-empty array of "
                     "{\"name\", \"detector\": [tags], \"shield\": [tags]}");
    return;
  }
  if (pairs->arr.size() > 64)
    errors.push_back("'anti_compton.pairs' has " + std::to_string(pairs->arr.size()) +
                     " entries; at most 64 (one bit each)");
  std::map<std::string, std::size_t> names;
  for (std::size_t i = 0; i < pairs->arr.size(); ++i) {
    const JV& e = pairs->arr[i];
    const std::string where = "anti_compton.pairs[" + std::to_string(i) + "]";
    if (e.t != JV::T::Obj) {
      errors.push_back(where + ": not an object");
      continue;
    }
    check_keys(e, where, {"name", "detector", "shield"}, errors);
    AcPair p;
    const JV* name = e.find("name");
    if (!name || name->t != JV::T::Str || name->str.empty()) {
      errors.push_back(where + ": 'name' must be a non-empty string");
    } else {
      p.name = name->str;
      auto it = names.find(p.name);
      if (it != names.end())
        errors.push_back("anti_compton.pairs[" + std::to_string(it->second) + "] and " + where +
                         ": duplicate name '" + p.name + "'");
      else
        names[p.name] = i;
    }
    if (!resolve_tags(e, "detector", where, d, p.detector, errors))
      errors.push_back(where + ": 'detector' must be a non-empty array of tag names");
    if (!resolve_tags(e, "shield", where, d, p.shield, errors))
      errors.push_back(where + ": 'shield' must be a non-empty array of tag names");
    // A channel vetoing itself would flag every one of its hits.
    std::set<uint16_t> det(p.detector.begin(), p.detector.end());
    std::vector<uint16_t> both;
    for (uint16_t k : p.shield)
      if (det.count(k)) both.push_back(k);
    if (!both.empty())
      errors.push_back(where + (p.name.empty() ? "" : " ('" + p.name + "')") + ": " +
                       chan_list(both) + " in both 'detector' and 'shield'");
    d.ac_pairs.push_back(std::move(p));
  }
}

// "l2": {"counters": [...], "flags": [...], "accept": [...]}. Every reference
// is resolved here: an unknown counter / flag is a load error, never a silent
// false at run time.
inline void parse_l2(const JV& root, EventDef& d, std::vector<std::string>& errors,
                     std::vector<std::string>& warnings) {
  const JV* l2 = root.find("l2");
  if (!l2) return;
  if (l2->t != JV::T::Obj) {
    errors.push_back("'l2' must be an object {\"counters\": [...], \"flags\": [...], "
                     "\"accept\": [...]}");
    return;
  }
  check_keys(*l2, "l2", {"counters", "flags", "accept"}, errors);
  const bool have_ac = root.find("anti_compton") != nullptr;

  // A list that is present must be a non-empty array of <= 64 entries.
  auto list = [&](const char* key, bool required) -> const JV* {
    const std::string what = std::string("'l2.") + key + "'";
    const JV* v = l2->find(key);
    if (!v) {
      if (required) errors.push_back(what + " is required (a non-empty array)");
      return nullptr;
    }
    if (v->t != JV::T::Arr || v->arr.empty()) {
      errors.push_back(what + " must be a non-empty array" +
                       (required ? "" : " (omit it if unused)"));
      return nullptr;
    }
    if (v->arr.size() > 64)
      errors.push_back(what + " has " + std::to_string(v->arr.size()) +
                       " entries; at most 64 (one bit each)");
    return v;
  };
  // Names become branches: C identifiers, not a fixed branch, unique over all three lists.
  std::map<std::string, std::string> names;  // name -> where
  auto take_name = [&](const JV& e, const std::string& where, std::string& out) {
    const JV* n = e.find("name");
    if (!n || n->t != JV::T::Str || n->str.empty()) {
      errors.push_back(where + ": 'name' must be a non-empty string");
      return;
    }
    out = n->str;
    if (!is_c_ident(out))
      errors.push_back(where + ": name '" + out + "' is not a C identifier "
                       "([A-Za-z_][A-Za-z0-9_]*); it becomes an events-tree branch");
    else if (is_fixed_branch(out))
      errors.push_back(where + ": name '" + out + "' is a fixed events-tree branch");
    auto it = names.find(out);
    if (it != names.end())
      errors.push_back(it->second + " and " + where + ": duplicate name '" + out +
                       "' (L2 names become branches)");
    else
      names[out] = where;
  };

  std::set<uint16_t> detectors;  // every AC detector channel, for the "clean" sanity warning
  for (const AcPair& p : d.ac_pairs) detectors.insert(p.detector.begin(), p.detector.end());
  std::map<std::string, int> counter_idx, flag_idx;
  if (const JV* cs = list("counters", true)) {
    for (std::size_t i = 0; i < cs->arr.size(); ++i) {
      const JV& e = cs->arr[i];
      const std::string where = "l2.counters[" + std::to_string(i) + "]";
      if (e.t != JV::T::Obj) {
        errors.push_back(where + ": not an object");
        continue;
      }
      check_keys(e, where, {"name", "tags", "ac"}, errors);
      L2Counter c;
      take_name(e, where, c.name);
      if (!resolve_tags(e, "tags", where, d, c.channels, errors))
        errors.push_back(where + ": 'tags' must be a non-empty array of tag names");
      if (const JV* ac = e.find("ac")) {
        const std::string s = ac->t == JV::T::Str ? ac->str : "";
        if (s == "any") {
          c.ac = L2Ac::Any;
        } else if (s == "clean" || s == "vetoed") {
          c.ac = s == "clean" ? L2Ac::Clean : L2Ac::Vetoed;
          if (!have_ac) {
            errors.push_back(where + ": ac \"" + s + "\" needs an 'anti_compton' section");
          } else if (!d.ac_pairs.empty()) {
            bool any_det = false;
            for (uint16_t k : c.channels) any_det = any_det || detectors.count(k) != 0;
            if (!any_det)
              warnings.push_back(where + " '" + c.name + "': ac \"" + s +
                                 "\" but none of its channels is an anti_compton detector "
                                 "(HitWithAC is never set there)");
          }
        } else {
          errors.push_back(where + ": 'ac' must be \"any\", \"clean\" or \"vetoed\"");
        }
      }
      if (!c.name.empty() && !counter_idx.count(c.name))
        counter_idx[c.name] = static_cast<int>(d.counters.size());
      d.counters.push_back(std::move(c));
    }
  }
  if (const JV* fs = list("flags", false)) {
    for (std::size_t i = 0; i < fs->arr.size(); ++i) {
      const JV& e = fs->arr[i];
      const std::string where = "l2.flags[" + std::to_string(i) + "]";
      if (e.t != JV::T::Obj) {
        errors.push_back(where + ": not an object");
        continue;
      }
      check_keys(e, where, {"name", "counter", "op", "value"}, errors);
      L2Flag f;
      take_name(e, where, f.name);
      const JV* cn = e.find("counter");
      if (!cn || cn->t != JV::T::Str) {
        errors.push_back(where + ": 'counter' must be the name of an l2 counter");
      } else {
        auto it = counter_idx.find(cn->str);
        if (it == counter_idx.end())
          errors.push_back(where + ": unknown counter '" + cn->str + "'");
        else
          f.counter = it->second;
      }
      const JV* op = e.find("op");
      if (!op || op->t != JV::T::Str || !parse_l2_op(op->str, f.op))
        errors.push_back(where + ": 'op' must be one of == != < <= > >=");
      if (!get_ll(e.find("value"), f.value))
        errors.push_back(where + ": 'value' must be an integer");
      if (!f.name.empty() && !flag_idx.count(f.name))
        flag_idx[f.name] = static_cast<int>(d.flags.size());
      d.flags.push_back(std::move(f));
    }
  }
  if (const JV* as = list("accept", false)) {
    for (std::size_t i = 0; i < as->arr.size(); ++i) {
      const JV& e = as->arr[i];
      const std::string where = "l2.accept[" + std::to_string(i) + "]";
      if (e.t != JV::T::Obj) {
        errors.push_back(where + ": not an object");
        continue;
      }
      check_keys(e, where, {"name", "op", "flags"}, errors);
      L2Accept a;
      take_name(e, where, a.name);
      const JV* op = e.find("op");
      if (op && op->t == JV::T::Str && (op->str == "AND" || op->str == "OR"))
        a.is_and = op->str == "AND";
      else
        errors.push_back(where + ": 'op' must be \"AND\" or \"OR\"");
      const JV* fl = e.find("flags");
      if (!fl || fl->t != JV::T::Arr || fl->arr.empty()) {
        errors.push_back(where + ": 'flags' must be a non-empty array of l2 flag names");
      } else {
        for (const JV& n : fl->arr) {
          if (n.t != JV::T::Str) {
            errors.push_back(where + ": 'flags' must be a non-empty array of l2 flag names");
            continue;
          }
          auto it = flag_idx.find(n.str);
          if (it == flag_idx.end())
            errors.push_back(where + ": unknown flag '" + n.str + "'");
          else
            a.flags.push_back(it->second);
        }
      }
      d.accepts.push_back(std::move(a));
    }
  }
}

}  // namespace edef_detail

// Parse + validate. Collects EVERY error (an operator fixing a file should see
// all problems at once). `max_lookback_ns` is the Sorter's safe horizon: a
// definition whose lookback_ns() >= it is rejected (sanity bound — a typo'd
// window of seconds would make the Sorter retain seconds of hits per chunk).
// On success def.index_channels() has been called. `_comment` (string or array
// of strings) is accepted in every object. Phase C sections (`tags`,
// `anti_compton`, `l2`) are optional, so a "1.0" file reads unchanged.
inline EventDefParseResult parse_event_def(const std::string& json_text,
                                           double max_lookback_ns) {
  using edef_detail::JV;
  using edef_detail::fmt_g;
  EventDefParseResult r;
  if (json_text.find_first_not_of(" \t\r\n") == std::string::npos) {
    r.errors.push_back("empty file (expected an event definition object)");
    return r;
  }
  JV root;
  try {
    root = tdelila::json::Parser(json_text).parse();
  } catch (const std::exception& e) {
    r.errors.push_back(std::string("malformed JSON (") + e.what() + ")");
    return r;
  }
  if (root.t != JV::T::Obj) {
    r.errors.push_back("top level must be a JSON object");
    return r;
  }
  std::vector<std::string>& errors = r.errors;
  EventDef& d = r.def;
  toff_detail::check_top_keys(root, {"version", "window_ns", "group_ns", "triggers", "tags",
                                     "anti_compton", "l2"},
                              errors);
  const JV* ver = root.find("version");
  if (!ver || ver->t != JV::T::Str)
    errors.push_back("missing or non-string 'version' (expected \"1.0\" or \"1.1\")");
  else if (ver->str != "1.0" && ver->str != "1.1")
    r.warnings.push_back("version \"" + ver->str + "\" (this reader knows \"1.0\" and \"1.1\")");

  bool window_ok = edef_detail::parse_pre_post(root.find("window_ns"), "window_ns", d.pre_ns,
                                               d.post_ns, errors);
  bool group_ok = edef_detail::get_span(root.find("group_ns"), "group_ns", d.group_ns, errors);
  // Rule 3: the reference can be the group's first trigger (the others are up
  // to G after it -> need post >= G) or its last (up to G before -> pre >= G).
  if (window_ok && group_ok && d.group_ns > std::fmin(d.pre_ns, d.post_ns))
    errors.push_back("group_ns " + fmt_g(d.group_ns) + " > min(window_ns.pre " +
                     fmt_g(d.pre_ns) + ", window_ns.post " + fmt_g(d.post_ns) +
                     "): every trigger of a group must lie inside its own event's window "
                     "wherever the reference sits in the group (first trigger -> needs "
                     "post >= group_ns, last -> needs pre >= group_ns; TODO 71 section 4 "
                     "rule 3)");
  edef_detail::parse_tags(root, d, errors);  // before everything that references tags
  edef_detail::parse_triggers(root, d, errors);
  edef_detail::parse_anti_compton(root, d, errors);
  edef_detail::parse_l2(root, d, errors, r.warnings);  // after AC: "clean" needs the pairs

  if (errors.empty() && d.lookback_ns() >= max_lookback_ns)
    errors.push_back(std::string("lookback group_ns + 2*pre + post") +
                     (d.has_ac() ? " + anti_compton pre + post" : "") + " = " +
                     fmt_g(d.lookback_ns()) + " ns >= Sorter safe horizon " +
                     fmt_g(max_lookback_ns) +
                     " ns: the Sorter could not keep enough context across chunk "
                     "boundaries (shrink the windows / group_ns, or raise the safe horizon)");
  if (errors.empty()) d.index_channels();
  return r;
}

// The definition in force, owned by the Receiver; the parsed def travels to the
// Sorter/Workers as a shared_ptr<const EventDef> frozen at RunOpen.
struct ActiveEventDef {
  std::shared_ptr<const EventDef> def;  // null until the first successful load
  std::string source_text;  // verbatim file actually applied
  std::string source_path;
  std::string loaded_at;    // UTC ISO time of the successful load
  bool loaded = false;      // a valid def has been adopted at least once
  bool stale = false;       // the latest reload failed; previous def kept
  std::string stale_error;
  std::vector<std::string> warnings;  // of the def in force (e.g. unknown version)
};

// Run-boundary reload policy — identical to reload_offsets (time_offsets.hpp).
// `text == nullptr` means the file could not be read (`read_error` says why).
// On success the new def is adopted (fresh shared_ptr), stale cleared, returns
// true. On ANY failure the previous def and text are KEPT, `stale` is set, the
// errors are returned, returns false. Never falls back to "no events" silently.
inline bool reload_event_def(ActiveEventDef& a, const std::string* text,
                             const std::string& read_error, const std::string& path,
                             double max_lookback_ns, const std::string& now_utc,
                             std::vector<std::string>& errors) {
  errors.clear();
  EventDefParseResult r;
  if (!text) {
    errors.push_back("cannot read " + path + ": " + read_error);
  } else {
    r = parse_event_def(*text, max_lookback_ns);
    errors = r.errors;
  }
  if (!errors.empty()) {
    a.stale = true;
    a.stale_error.clear();
    for (std::size_t i = 0; i < errors.size(); ++i)
      a.stale_error += (i ? "; " : "") + errors[i];
    return false;
  }
  // A NEW object every time: the def frozen into a running Sorter/Worker (its
  // own shared_ptr copy) is never mutated under it.
  a.def = std::make_shared<const EventDef>(std::move(r.def));
  a.warnings = std::move(r.warnings);
  a.source_text = *text;
  a.source_path = path;
  a.loaded_at = now_utc;
  a.loaded = true;
  a.stale = false;
  a.stale_error.clear();
  return true;
}

// One line for the run log, e.g.
//   "window [-1000, +1000] ns, group 1000 ns, 2 trigger(s): ThGEM1(p1: m0c1) ThGEM2(p2: m0c6)"
// Without priorities:
//   "window [-1000, +1000] ns, group 1000 ns, 2 trigger(s), reference = first of group:
//    ThGEM1(m0c1) ThGEM2(m0c6)"
// Phase C sections are appended when defined, e.g.
//   "...: Ge(m0c0,m1c0,...); tags 6; AC [-300, +300] ns, 2 pair(s); L2 4 counter(s),
//    1 flag(s), 1 accept(s)"
inline std::string format_event_def_line(const EventDef& d) {
  char buf[160];
  std::snprintf(buf, sizeof buf, "window [-%g, +%g] ns, group %g ns, %zu trigger(s)",
                d.pre_ns, d.post_ns, d.group_ns, d.triggers.size());
  std::string s = buf;
  s += d.priorities_given ? ":" : ", reference = first of group:";
  for (const TriggerDef& t : d.triggers) {
    s += ' ' + t.name + '(';
    if (d.priorities_given) s += 'p' + std::to_string(t.priority) + ": ";
    // Runs of >= 3 consecutive channels on one module (file order) collapse
    // to "m1c0-14": a 45-ring Si trigger stays one readable log line.
    const std::vector<uint16_t>& c = t.channels;
    for (std::size_t i = 0; i < c.size();) {
      std::size_t j = i;
      while (j + 1 < c.size() && c[j + 1] == c[j] + 1 && (c[j + 1] >> 8) == (c[i] >> 8)) ++j;
      if (i) s += ',';
      if (j - i >= 2) {
        s += mc_name(c[i]) + '-' + std::to_string(c[j] & 0xFF);
        i = j + 1;
      } else {
        s += mc_name(c[i]);
        ++i;
      }
    }
    s += ')';
  }
  // Phase C, only what is defined.
  if (!d.tags.empty()) s += "; tags " + std::to_string(d.tags.size());
  if (d.has_ac()) {
    std::snprintf(buf, sizeof buf, "; AC [-%g, +%g] ns, %zu pair(s)", d.ac_pre_ns, d.ac_post_ns,
                  d.ac_pairs.size());
    s += buf;
  }
  if (d.has_l2())
    s += "; L2 " + std::to_string(d.counters.size()) + " counter(s), " +
         std::to_string(d.flags.size()) + " flag(s), " + std::to_string(d.accepts.size()) +
         " accept(s)";
  return s;
}

// Provenance stored with every run (events-tree UserInfo TNamed "eventDef"):
// what was applied, from where, status (ok / stale + error), the resolved
// parameters and the verbatim source text. Mirrors offsets_provenance_json;
// numbers use %.17g so they round-trip exactly. A trigger's "index" is what the
// events tree's TriggerIdx refers to.
inline std::string event_def_provenance_json(const ActiveEventDef& a) {
  static const EventDef none;  // defensive: never loaded -> empty parameters
  const EventDef& d = a.def ? *a.def : none;
  std::string s = std::string("{\"applied\":") + (a.def ? "true" : "false") + ",";
  s += "\"convention\":\"events are built on the ALIGNED time axis (time offsets applied "
       "first); TriggerTime = the reference trigger's absolute aligned time (ns), HitTime = "
       "hit - reference (ns); grouping/reference/window/assignment rules: "
       "TODO/71_generic_event_building.md section 4\",";
  s += "\"source_path\":\"" + json_escape(a.source_path) + "\",";
  s += "\"loaded_at\":\"" + json_escape(a.loaded_at) + "\",";
  s += std::string("\"status\":\"") +
       (a.stale ? "stale: run-start reload failed, previous definition kept" : "ok") + "\",";
  if (a.stale) s += "\"reload_error\":\"" + json_escape(a.stale_error) + "\",";
  char buf[256];
  std::snprintf(buf, sizeof buf,
                "\"window_ns\":{\"pre\":%.17g,\"post\":%.17g},\"group_ns\":%.17g,", d.pre_ns,
                d.post_ns, d.group_ns);
  s += buf;
  s += std::string("\"priorities_given\":") + (d.priorities_given ? "true" : "false") + ",";
  s += "\"triggers\":[";
  for (std::size_t i = 0; i < d.triggers.size(); ++i) {
    const TriggerDef& t = d.triggers[i];
    if (i) s += ',';
    s += "{\"index\":" + std::to_string(i) + ",\"name\":\"" + json_escape(t.name) +
         "\",\"priority\":" + std::to_string(t.priority) + ",\"channels\":";
    edef_detail::append_chans(s, t.channels);
    s += '}';
  }
  s += "],";
  // Phase C, only what is defined: resolved tag table, AC pairs, L2. An index
  // is the bit / slot the events tree and the run summary use.
  if (!d.tags.empty()) {
    s += "\"tags\":{";
    bool first = true;
    for (const auto& kv : d.tags) {
      s += std::string(first ? "" : ",") + "\"" + json_escape(kv.first) + "\":";
      edef_detail::append_chans(s, kv.second);
      first = false;
    }
    s += "},";
  }
  if (d.has_ac()) {
    std::snprintf(buf, sizeof buf,
                  "\"anti_compton\":{\"window_ns\":{\"pre\":%.17g,\"post\":%.17g},\"pairs\":[",
                  d.ac_pre_ns, d.ac_post_ns);
    s += buf;
    for (std::size_t i = 0; i < d.ac_pairs.size(); ++i) {
      const AcPair& p = d.ac_pairs[i];
      s += std::string(i ? "," : "") + "{\"index\":" + std::to_string(i) + ",\"name\":\"" +
           json_escape(p.name) + "\",\"detector\":";
      edef_detail::append_chans(s, p.detector);
      s += ",\"shield\":";
      edef_detail::append_chans(s, p.shield);
      s += '}';
    }
    s += "]},";
  }
  if (d.has_l2()) {
    s += "\"l2\":{\"counters\":[";
    for (std::size_t i = 0; i < d.counters.size(); ++i) {
      const L2Counter& c = d.counters[i];
      s += std::string(i ? "," : "") + "{\"index\":" + std::to_string(i) + ",\"name\":\"" +
           json_escape(c.name) + "\",\"ac\":\"" + edef_detail::l2_ac_name(c.ac) +
           "\",\"channels\":";
      edef_detail::append_chans(s, c.channels);
      s += '}';
    }
    s += "],\"flags\":[";
    for (std::size_t i = 0; i < d.flags.size(); ++i) {
      const L2Flag& f = d.flags[i];
      const bool ok = f.counter >= 0 && static_cast<std::size_t>(f.counter) < d.counters.size();
      s += std::string(i ? "," : "") + "{\"index\":" + std::to_string(i) + ",\"name\":\"" +
           json_escape(f.name) + "\",\"counter\":\"" +
           json_escape(ok ? d.counters[f.counter].name : "?") + "\",\"op\":\"" +
           edef_detail::l2_op_name(f.op) + "\",\"value\":" + std::to_string(f.value) + '}';
    }
    s += "],\"accept\":[";
    for (std::size_t i = 0; i < d.accepts.size(); ++i) {
      const L2Accept& x = d.accepts[i];
      s += std::string(i ? "," : "") + "{\"index\":" + std::to_string(i) + ",\"name\":\"" +
           json_escape(x.name) + "\",\"op\":\"" + (x.is_and ? "AND" : "OR") +
           "\",\"flags\":[";
      for (std::size_t j = 0; j < x.flags.size(); ++j) {
        const int fi = x.flags[j];
        const bool ok = fi >= 0 && static_cast<std::size_t>(fi) < d.flags.size();
        s += std::string(j ? "," : "") + "\"" + json_escape(ok ? d.flags[fi].name : "?") + "\"";
      }
      s += "]}";
    }
    s += "]},";
  }
  s += "\"warnings\":[";
  for (std::size_t i = 0; i < a.warnings.size(); ++i)
    s += (i ? ",\"" : "\"") + json_escape(a.warnings[i]) + "\"";
  s += "],\"source_text\":\"" + json_escape(a.source_text) + "\"}";
  return s;
}

}  // namespace rootsink

#endif  // ROOTSINK_EVENT_DEF_HPP
