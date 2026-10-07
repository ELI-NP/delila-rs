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

struct EventDef {
  double pre_ns = 0.0;    // window before the reference
  double post_ns = 0.0;   // window after the reference
  double group_ns = 0.0;  // G: non-extending trigger grouping span
  std::vector<TriggerDef> triggers;
  bool priorities_given = false;  // false: every priority is 0 (rule 2 -> first trigger)

  // mc_key -> index into `triggers`, -1 = not a trigger channel. 65536 entries
  // once index_channels() ran; empty = no trigger at all.
  std::vector<int16_t> trig_of;

  // Fill trig_of from `triggers`. Call after `triggers` is final (the parser
  // does; tests building an EventDef by hand must too). Duplicate channels are
  // a parse error, so here the last writer simply wins.
  void index_channels() {
    trig_of.assign(65536, -1);
    for (std::size_t i = 0; i < triggers.size(); ++i)
      for (uint16_t k : triggers[i].channels) trig_of[k] = static_cast<int16_t>(i);
  }

  int trigger_index(uint8_t module, uint8_t channel) const {
    return trig_of.empty() ? -1 : trig_of[mc_key(module, channel)];
  }

  // Context the Sorter must carry back across a chunk boundary so that every
  // emitted event sees all of its hits and all competing references:
  // G + 2*pre + post (derivation: eb_core.hpp section 7, "chunk contract").
  double lookback_ns() const { return group_ns + 2.0 * pre_ns + post_ns; }
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

inline void parse_window(const JV& root, EventDef& d, bool& ok,
                         std::vector<std::string>& errors) {
  ok = false;
  const JV* w = root.find("window_ns");
  if (!w || w->t != JV::T::Obj) {
    errors.push_back("'window_ns' must be an object {\"pre\": ns, \"post\": ns}");
    return;
  }
  for (const auto& kv : w->obj) {
    if (kv.first == "pre" || kv.first == "post") continue;
    if (kv.first == "_comment") {
      if (!toff_detail::comment_ok(kv.second))
        errors.push_back("'window_ns._comment' must be a string or an array of strings");
      continue;
    }
    errors.push_back("window_ns: unknown key '" + kv.first + "' (expected 'pre' and 'post')");
  }
  bool pre = get_span(w->find("pre"), "window_ns.pre", d.pre_ns, errors);
  bool post = get_span(w->find("post"), "window_ns.post", d.post_ns, errors);
  ok = pre && post;
}

// One trigger entry; its errors go to `errors`. `label` is what later messages
// call it ("trigger 'Si'" or, without a valid name, "triggers[3]").
inline void parse_trigger(const JV& e, const std::string& where, TriggerDef& t,
                          bool& has_priority, std::string& label,
                          std::vector<std::string>& errors) {
  has_priority = false;
  label = where;
  for (const auto& kv : e.obj) {
    const std::string& k = kv.first;
    if (k == "name" || k == "priority" || k == "channels") continue;
    if (k == "_comment") {
      if (!toff_detail::comment_ok(kv.second))
        errors.push_back(where + ": '_comment' must be a string or an array of strings");
      continue;
    }
    if (k == "tags") {
      errors.push_back(where +
                       ": 'tags' needs the channel table (TODO 71 Phase C); use 'channels'");
      continue;
    }
    errors.push_back(where + ": unknown key '" + k + "' (expected 'name', 'priority', 'channels')");
  }
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
  if (!chs || chs->t != JV::T::Arr) {
    errors.push_back(where + ": 'channels' must be a non-empty array of [module, channel]");
    return;
  }
  if (chs->arr.empty()) {
    errors.push_back(where + ": 'channels' is empty (a trigger needs at least one channel)");
    return;
  }
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

inline void parse_triggers(const JV& root, EventDef& d, std::vector<std::string>& errors) {
  const JV* trg = root.find("triggers");
  if (!trg || trg->t != JV::T::Arr) {
    errors.push_back("'triggers' must be a non-empty array of "
                     "{\"name\", \"priority\" (optional), \"channels\"}");
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
    parse_trigger(e, where, t, has_priority, label, errors);
    if (!t.name.empty()) {
      auto it = names.find(t.name);
      if (it != names.end())
        errors.push_back("triggers[" + std::to_string(it->second) + "] and " + where +
                         ": duplicate name '" + t.name + "'");
      else
        names[t.name] = i;
    }
    (has_priority ? with_prio : without_prio).push_back(label);
    // One channel = one trigger: a channel listed twice would make "which
    // trigger is this hit" (TriggerIdx, the priority) depend on file order.
    std::set<uint16_t> mine;
    for (uint16_t k : t.channels) {
      auto it = owner.find(k);
      if (it == owner.end())
        owner[k] = label;
      else if (mine.count(k))
        errors.push_back(mc_name(k) + " listed twice in " + label);
      else
        errors.push_back(mc_name(k) + " listed in both " + it->second + " and " + label);
      mine.insert(k);
    }
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

}  // namespace edef_detail

// Parse + validate. Collects EVERY error (an operator fixing a file should see
// all problems at once). `max_lookback_ns` is the Sorter's safe horizon: a
// definition whose lookback_ns() >= it is rejected (sanity bound — a typo'd
// window of seconds would make the Sorter retain seconds of hits per chunk).
// On success def.index_channels() has been called. `_comment` (string or array
// of strings) is accepted at the top level, in window_ns and in each trigger.
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
  toff_detail::check_top_keys(root, {"version", "window_ns", "group_ns", "triggers"}, errors);
  const JV* ver = root.find("version");
  if (!ver || ver->t != JV::T::Str)
    errors.push_back("missing or non-string 'version' (expected \"1.0\")");
  else if (ver->str != "1.0")
    r.warnings.push_back("version \"" + ver->str + "\" (this reader knows \"1.0\")");

  bool window_ok = false;
  edef_detail::parse_window(root, d, window_ok, errors);
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
  edef_detail::parse_triggers(root, d, errors);

  if (errors.empty() && d.lookback_ns() >= max_lookback_ns)
    errors.push_back("lookback group_ns + 2*pre + post = " + fmt_g(d.lookback_ns()) +
                     " ns >= Sorter safe horizon " + fmt_g(max_lookback_ns) +
                     " ns: the Sorter could not keep enough context across chunk "
                     "boundaries (shrink window_ns / group_ns, or raise the safe horizon)");
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
inline std::string format_event_def_line(const EventDef& d) {
  char buf[160];
  std::snprintf(buf, sizeof buf, "window [-%g, +%g] ns, group %g ns, %zu trigger(s)",
                d.pre_ns, d.post_ns, d.group_ns, d.triggers.size());
  std::string s = buf;
  s += d.priorities_given ? ":" : ", reference = first of group:";
  for (const TriggerDef& t : d.triggers) {
    s += ' ' + t.name + '(';
    if (d.priorities_given) s += 'p' + std::to_string(t.priority) + ": ";
    for (std::size_t i = 0; i < t.channels.size(); ++i)
      s += (i ? "," : "") + mc_name(t.channels[i]);
    s += ')';
  }
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
         "\",\"priority\":" + std::to_string(t.priority) + ",\"channels\":[";
    for (std::size_t j = 0; j < t.channels.size(); ++j) {
      std::snprintf(buf, sizeof buf, "%s[%u,%u]", j ? "," : "",
                    static_cast<unsigned>(t.channels[j] >> 8),
                    static_cast<unsigned>(t.channels[j] & 0xFF));
      s += buf;
    }
    s += "]}";
  }
  s += "],\"warnings\":[";
  for (std::size_t i = 0; i < a.warnings.size(); ++i)
    s += (i ? ",\"" : "\"") + json_escape(a.warnings[i]) + "\"";
  s += "],\"source_text\":\"" + json_escape(a.source_text) + "\"}";
  return s;
}

}  // namespace rootsink

#endif  // ROOTSINK_EVENT_DEF_HPP
