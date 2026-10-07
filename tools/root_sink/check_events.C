// check_events.C — recompute a run's generic events (TODO 71) from its hits
// tree in ONE chunk and compare them with the `events` tree root_sink wrote.
//
// root_sink builds events chunk by chunk (Sorter -> TriggerGrouper -> Workers);
// recomputing over the whole run as a single chunk has no chunk boundary at
// all, so any boundary, ordering or wiring bug (branches, EventID, Writer
// order) shows up as a mismatch. The hits tree holds exactly what root_sink
// fed the Sorter (after --module and time alignment), so the run needs
// --hits-tree. Same eb_core.hpp code — the rules themselves are checked against
// an independent brute-force implementation in test_eb_core.cpp.
//
//   root -l -b -q 'check_events.C+("run0056_0000_exp.root")'
//   root -l -b -q 'check_events.C+("run.root","event_def.json","delila")'
//
// The definition defaults to the run's own sidecar <stem>_eventDef.json (the
// verbatim file root_sink applied). Hits inside an event are compared as a
// canonical multiset: equal timestamps may leave std::sort in either order.
// Prints IDENTICAL or MISMATCH plus the run's counters (orphans, contested,
// NTriggers distribution). A ROOT MaxTreeSize-split run is not supported.
#include <algorithm>
#include <cstdio>
#include <fstream>
#include <limits>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

#include "TFile.h"
#include "TTree.h"

#include "eb_core.hpp"

void check_events(const char* path, const char* def_path = "", const char* hits_tree = "delila") {
  using namespace rootsink;
  std::string root_path = path;
  std::string dpath = def_path ? def_path : "";
  if (dpath.empty()) {
    dpath = root_path;
    if (dpath.size() > 5 && dpath.compare(dpath.size() - 5, 5, ".root") == 0)
      dpath.erase(dpath.size() - 5);
    dpath += "_eventDef.json";
  }
  std::ifstream in(dpath, std::ios::binary);
  if (!in) {
    printf("check_events: cannot read event definition %s\n", dpath.c_str());
    return;
  }
  std::stringstream ss;
  ss << in.rdbuf();
  EventDefParseResult pr = parse_event_def(ss.str(), std::numeric_limits<double>::infinity());
  if (!pr.errors.empty()) {
    printf("check_events: %zu error(s) in %s:\n", pr.errors.size(), dpath.c_str());
    for (const auto& e : pr.errors) printf("  - %s\n", e.c_str());
    return;
  }
  auto def = std::make_shared<const EventDef>(pr.def);
  printf("check_events: definition %s: %s\n", dpath.c_str(), format_event_def_line(*def).c_str());

  TFile f(path, "READ");
  if (f.IsZombie()) return;
  TTree* ht = f.Get<TTree>(hits_tree);
  TTree* et = f.Get<TTree>("events");
  if (!ht || !et) {
    printf("check_events: need trees '%s' (run with --hits-tree) and 'events' in %s\n", hits_tree,
           path);
    return;
  }

  // --- hits -> one chunk covering the whole run ---
  UChar_t mod = 0, ch = 0;
  UShort_t e = 0, es = 0;
  Double_t ts = 0;
  ht->SetBranchAddress("module", &mod);
  ht->SetBranchAddress("channel", &ch);
  ht->SetBranchAddress("energy", &e);
  ht->SetBranchAddress("energy_short", &es);
  ht->SetBranchAddress("timestamp_ns", &ts);
  eb::SortedChunk c;
  const Long64_t nh = ht->GetEntries();
  c.hits.reserve(static_cast<std::size_t>(nh));
  for (Long64_t i = 0; i < nh; ++i) {
    ht->GetEntry(i);
    ScalarHit h;
    h.module = mod;
    h.channel = ch;
    h.energy = e;
    h.energy_short = es;
    h.timestamp_ns = ts;
    c.hits.push_back(h);
  }
  auto by_t = [](const ScalarHit& a, const ScalarHit& b) { return a.timestamp_ns < b.timestamp_ns; };
  if (!std::is_sorted(c.hits.begin(), c.hits.end(), by_t)) {
    printf("check_events: NOTE hits tree not time-sorted (pre-TODO 66 file?) — sorting\n");
    std::stable_sort(c.hits.begin(), c.hits.end(), by_t);
  }
  eb::TriggerGrouper g;
  g.reset(def);
  eb::EventPlan plan = g.plan_chunk(c);  // core (-inf, +inf): everything decided
  eb::GenericStats st;
  std::vector<eb::GenericEvent> ref = eb::build_generic_events(c, plan, st);

  // --- the events tree ---
  ULong64_t id = 0;
  Double_t tt = 0;
  UChar_t tmod = 0, tch = 0, tidx = 0;
  UInt_t ntrig = 0, nhits = 0;
  const Long64_t ne = et->GetEntries();
  const std::size_t cap = static_cast<std::size_t>(std::max(1.0, et->GetMaximum("NHits")));
  std::vector<UChar_t> hmod(cap), hch(cap);
  std::vector<UShort_t> he(cap), hes(cap);
  std::vector<Double_t> htime(cap);
  std::unique_ptr<Bool_t[]> htrig(new Bool_t[cap]);
  et->SetBranchAddress("EventID", &id);
  et->SetBranchAddress("TriggerTime", &tt);
  et->SetBranchAddress("TriggerMod", &tmod);
  et->SetBranchAddress("TriggerCh", &tch);
  et->SetBranchAddress("TriggerIdx", &tidx);
  et->SetBranchAddress("NTriggers", &ntrig);
  et->SetBranchAddress("NHits", &nhits);
  et->SetBranchAddress("HitMod", hmod.data());
  et->SetBranchAddress("HitCh", hch.data());
  et->SetBranchAddress("HitEnergy", he.data());
  et->SetBranchAddress("HitEnergyShort", hes.data());
  et->SetBranchAddress("HitTime", htime.data());
  et->SetBranchAddress("HitIsTrigger", htrig.get());

  auto canon = [](std::vector<eb::EventHit> v) {
    std::sort(v.begin(), v.end(), [](const eb::EventHit& a, const eb::EventHit& b) {
      if (a.dt != b.dt) return a.dt < b.dt;
      if (a.mod != b.mod) return a.mod < b.mod;
      if (a.ch != b.ch) return a.ch < b.ch;
      if (a.energy != b.energy) return a.energy < b.energy;
      return a.energy_short < b.energy_short;
    });
    return v;
  };

  long long mismatches = 0;
  long long tree_hits = 0;
  const Long64_t n = std::min<Long64_t>(ne, static_cast<Long64_t>(ref.size()));
  for (Long64_t i = 0; i < n; ++i) {
    et->GetEntry(i);
    tree_hits += nhits;
    const eb::GenericEvent& r = ref[static_cast<std::size_t>(i)];
    std::vector<eb::EventHit> got(nhits);
    for (UInt_t j = 0; j < nhits; ++j) {
      got[j].mod = hmod[j];
      got[j].ch = hch[j];
      got[j].energy = he[j];
      got[j].energy_short = hes[j];
      got[j].dt = htime[j];
      got[j].is_trigger = htrig[j];
    }
    // Time order inside the event (as written), then content as a multiset.
    bool ordered = true;
    for (UInt_t j = 1; j < nhits; ++j)
      if (htime[j] < htime[j - 1]) ordered = false;
    bool same = id == static_cast<ULong64_t>(i) && tt == r.t && tmod == r.mod && tch == r.ch &&
                tidx == r.trig_idx && ntrig == r.n_triggers && nhits == r.hits.size() && ordered;
    if (same) {
      auto a = canon(got), b = canon(r.hits);
      for (std::size_t j = 0; same && j < a.size(); ++j)
        same = a[j].dt == b[j].dt && a[j].mod == b[j].mod && a[j].ch == b[j].ch &&
               a[j].energy == b[j].energy && a[j].energy_short == b[j].energy_short &&
               a[j].is_trigger == b[j].is_trigger;
    }
    if (!same) {
      if (mismatches < 5)
        printf("  MISMATCH entry %lld: id %llu t %.3f/%.3f m%uc%u/m%uc%u idx %u/%u ntrig %u/%u "
               "nhits %u/%zu%s\n",
               i, static_cast<unsigned long long>(id), tt, r.t, tmod, tch, r.mod, r.ch, tidx,
               r.trig_idx, ntrig, r.n_triggers, nhits, r.hits.size(),
               ordered ? "" : " (hits not time-ordered)");
      ++mismatches;
    }
  }
  const bool count_ok = ne == static_cast<Long64_t>(ref.size());
  printf("check_events: %s hits %lld -> events tree %lld, recomputed %zu%s\n", path,
         static_cast<long long>(nh), static_cast<long long>(ne), ref.size(),
         count_ok ? "" : "  <-- COUNT MISMATCH");
  printf("  hits in events %llu, orphans %llu (sum %llu %s hits), contested %llu\n",
         static_cast<unsigned long long>(st.hits_in_events),
         static_cast<unsigned long long>(st.orphans),
         static_cast<unsigned long long>(st.hits_in_events + st.orphans),
         st.hits_in_events + st.orphans == static_cast<unsigned long long>(nh) ? "==" : "!=",
         static_cast<unsigned long long>(st.contested));
  printf("  NTriggers:");
  for (std::size_t i = 0; i < st.ntrig.size(); ++i)
    if (st.ntrig[i])
      printf(" %s%zu:%llu", i == 7 ? "" : "", i + 1, static_cast<unsigned long long>(st.ntrig[i]));
  printf("%s\n", st.ntrig[7] ? "  (8 = 8+)" : "");
  const bool ok = count_ok && mismatches == 0 &&
                  st.hits_in_events + st.orphans == static_cast<unsigned long long>(nh) &&
                  tree_hits == static_cast<long long>(st.hits_in_events);
  printf("check_events: %s (%lld mismatching entries)\n", ok ? "IDENTICAL" : "MISMATCH",
         mismatches);
}
