// compare_hits.C — order-INDEPENDENT equality check between two root_sink
// hits trees (V2/V3 gates of the TODO 66 thread restructure).
//
// The multithreaded sink writes the same hit SET as the old one but globally
// time-sorted, so entry-by-entry diffing is useless. Instead compare, per
// channel: entry count, Σenergy, Σenergy_short (exact integer sums), and the
// XOR of every timestamp's 64-bit pattern (exact, order-independent — unlike
// a floating sum). Also verifies the second file is monotonically time-sorted.
//
//   root -l -b -q 'compare_hits.C("baseline.root","candidate.root")'
//
// Time offsets (TODO 70): a root_sink run with --time-calib writes ALIGNED
// timestamps (raw - offset_ns). Comparing it against a RAW tree (delila2root /
// replay input) without the 4th argument reports tsxor MISMATCH on exactly the
// offset channels while n/esum/essum stay identical. Pass the same
// timeSettings.json as the 4th argument to align A first — the subtraction uses
// the same doubles as root_sink, so a correct run is bit-IDENTICAL:
//   root -l -b -q 'compare_hits.C("raw.root","aligned.root","delila","ts.json")'
// (Channels are still keyed on channel alone here; module-aware compare is TODO 70 Phase 2.)
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>

#include "TFile.h"
#include "TTree.h"

#include "time_offsets.hpp"

void compare_hits(const char* a_path, const char* b_path,
                  const char* tree_name = "delila", const char* time_calib = "") {
  rootsink::OffsetTable toff;
  const bool align_a = time_calib && time_calib[0];
  if (align_a) {
    std::ifstream in(time_calib, std::ios::binary);
    if (!in) {
      printf("compare_hits: cannot read %s\n", time_calib);
      return;
    }
    std::stringstream ss;
    ss << in.rdbuf();
    rootsink::OffsetParseResult pr = rootsink::parse_time_offsets(ss.str());
    if (!pr.errors.empty()) {
      for (const auto& e : pr.errors) printf("compare_hits: %s: %s\n", time_calib, e.c_str());
      return;
    }
    toff = pr.table;
    printf("compare_hits: aligning A with %zu offsets from %s (aligned = raw - offset_ns)\n",
           toff.abs_ns.size(), time_calib);
  }
  struct Stat {
    Long64_t n[256] = {};
    ULong64_t esum[256] = {};
    ULong64_t essum[256] = {};
    ULong64_t tsxor[256] = {};
    Long64_t total = 0;
    bool monotone = true;
  };
  auto scan = [&](const char* path, Stat& s, bool align) -> bool {
    TFile* f = TFile::Open(path, "READ");
    if (!f || f->IsZombie()) {
      printf("compare_hits: cannot open %s\n", path);
      return false;
    }
    TTree* t = dynamic_cast<TTree*>(f->Get(tree_name));
    if (!t) {
      printf("compare_hits: no tree '%s' in %s\n", tree_name, path);
      return false;
    }
    UChar_t mod = 0, ch = 0;
    UShort_t e = 0, es = 0;
    Double_t ts = 0;
    t->SetBranchAddress("module", &mod);
    t->SetBranchAddress("channel", &ch);
    t->SetBranchAddress("energy", &e);
    t->SetBranchAddress("energy_short", &es);
    t->SetBranchAddress("timestamp_ns", &ts);
    Double_t prev = -1e300;
    const Long64_t n = t->GetEntries();
    for (Long64_t i = 0; i < n; ++i) {
      t->GetEntry(i);
      if (align) {
        auto it = toff.abs_ns.find(rootsink::mc_key(mod, ch));
        if (it != toff.abs_ns.end()) ts -= it->second;
      }
      s.n[ch]++;
      s.esum[ch] += e;
      s.essum[ch] += es;
      ULong64_t bits;
      std::memcpy(&bits, &ts, 8);
      s.tsxor[ch] ^= bits;
      if (ts < prev) s.monotone = false;
      prev = ts;
    }
    s.total = n;
    f->Close();
    return true;
  };

  Stat A, B;
  if (!scan(a_path, A, align_a) || !scan(b_path, B, false)) return;

  bool same = A.total == B.total;
  for (int c = 0; c < 256; ++c) {
    if (A.n[c] != B.n[c] || A.esum[c] != B.esum[c] || A.essum[c] != B.essum[c] ||
        A.tsxor[c] != B.tsxor[c]) {
      printf("compare_hits: MISMATCH ch%d  n %lld/%lld  esum %llu/%llu  tsxor %llx/%llx\n",
             c, A.n[c], B.n[c], A.esum[c], B.esum[c], A.tsxor[c], B.tsxor[c]);
      same = false;
    }
  }
  printf("compare_hits: A=%lld entries, B=%lld entries\n", A.total, B.total);
  printf("compare_hits: B monotone in timestamp_ns: %s\n", B.monotone ? "YES" : "NO");
  printf("compare_hits: content %s\n", same ? "IDENTICAL" : "DIFFERS");
}
