// delila2root.C — convert a DELILA `.delila` file to a compressed ROOT TTree.
//
// Replaces the old Rust/oxyroot `delila2root` (which could not compress and
// emitted a fixed 49-branch tree). This one reads via TDelila, writes one branch
// per event field, and uses ROOT's native ZSTD compression — so the branches
// that are empty for a given firmware cost almost nothing.
//
// Run as a ROOT macro:
//   root -l -b -q 'delila2root.C("run0003_0000.delila")'
//   root -l -b -q 'delila2root.C("in.delila","out.root")'
// Or compile a standalone tool:
//   g++ -O2 -std=c++17 delila2root.C $(root-config --cflags --libs) -o delila2root
//   ./delila2root in.delila [out.root] [in2.delila ...]
//
// License: BSD-3-Clause (same as delila-rs).

#include <cstdio>
#include <set>
#include <string>
#include <vector>

#include "TDelila.hpp"
#include "TFile.h"
#include "TTree.h"

// ROOT compression setting = algorithm*100 + level. ZSTD is algorithm 5, so
// 505 = ZSTD level 5. Using the integer avoids the ROOT::kZSTD enum, whose
// namespace moved across ROOT versions. (Fallback if ZSTD is unavailable on an
// old ROOT: 404 = LZ4 level 4.)
static const int kDelilaCompression = 505;

// Convert one or more `.delila` files (same run) into a single ROOT tree.
// Returns the number of events written, or -1 on error.
long delila2root(const char* in_path, const char* out_path = nullptr,
                 const std::vector<std::string>& extra = {}) {
  std::vector<std::string> inputs;
  inputs.push_back(in_path);
  for (const auto& e : extra) inputs.push_back(e);

  // Default output: replace/append .root next to the first input.
  std::string out;
  if (out_path && out_path[0]) {
    out = out_path;
  } else {
    out = inputs[0];
    auto dot = out.rfind(".delila");
    if (dot != std::string::npos) out = out.substr(0, dot);
    out += ".root";
  }

  // ZSTD level 5 (native compression; the old oxyroot tool wrote raw).
  TFile* f = TFile::Open(out.c_str(), "RECREATE", "", kDelilaCompression);
  if (!f || f->IsZombie()) {
    std::fprintf(stderr, "delila2root: cannot create %s\n", out.c_str());
    return -1;
  }
  TTree* tree = new TTree("delila", "DELILA events");

  // --- Branch buffers (one per event field; all firmwares) ---
  UChar_t  module = 0, channel = 0, time_resolution = 0;
  UShort_t energy = 0, energy_short = 0, trigger_threshold = 0;
  Double_t timestamp_ns = 0.0, ns_per_sample = 0.0;
  ULong64_t flags = 0;
  ULong64_t user_info[4] = {0, 0, 0, 0};
  Bool_t has_waveform = false;
  Bool_t analog_probe_is_signed[3] = {false, false, false};
  UChar_t analog_probe_type[3] = {0, 0, 0};
  UChar_t digital_probe_type[16] = {0};
  std::vector<short> analog_probe[3];
  std::vector<short> digital_probe[16];

  tree->Branch("module", &module, "module/b");
  tree->Branch("channel", &channel, "channel/b");
  tree->Branch("energy", &energy, "energy/s");
  tree->Branch("energy_short", &energy_short, "energy_short/s");
  tree->Branch("timestamp_ns", &timestamp_ns, "timestamp_ns/D");
  tree->Branch("flags", &flags, "flags/l");
  tree->Branch("user_info", user_info, "user_info[4]/l");
  tree->Branch("has_waveform", &has_waveform, "has_waveform/O");
  tree->Branch("time_resolution", &time_resolution, "time_resolution/b");
  tree->Branch("trigger_threshold", &trigger_threshold, "trigger_threshold/s");
  tree->Branch("ns_per_sample", &ns_per_sample, "ns_per_sample/D");
  for (int k = 0; k < 3; ++k)
    tree->Branch(Form("analog_probe%d", k + 1), &analog_probe[k]);
  for (int k = 0; k < 16; ++k)
    tree->Branch(Form("digital_probe%d", k + 1), &digital_probe[k]);
  tree->Branch("analog_probe1_is_signed", &analog_probe_is_signed[0], "analog_probe1_is_signed/O");
  tree->Branch("analog_probe2_is_signed", &analog_probe_is_signed[1], "analog_probe2_is_signed/O");
  tree->Branch("analog_probe3_is_signed", &analog_probe_is_signed[2], "analog_probe3_is_signed/O");
  tree->Branch("analog_probe_type", analog_probe_type, "analog_probe_type[3]/b");
  tree->Branch("digital_probe_type", digital_probe_type, "digital_probe_type[16]/b");

  // Set of field names we materialize into branches, for a coverage check.
  const std::set<std::string> handled_event = {
      "module", "channel", "energy", "energy_short", "timestamp_ns", "flags",
      "user_info", "waveform"};
  const std::set<std::string> handled_wave = {
      "analog_probe1", "analog_probe2", "analog_probe3", "time_resolution",
      "trigger_threshold", "ns_per_sample", "analog_probe1_is_signed",
      "analog_probe2_is_signed", "analog_probe3_is_signed", "analog_probe_type",
      "digital_probe_type"};

  long total = 0;
  bool schema_checked = false;

  for (const auto& path : inputs) {
    tdelila::TDelila d(path);
    if (!d.good()) {
      std::fprintf(stderr, "delila2root: %s: %s\n", path.c_str(), d.error().c_str());
      continue;
    }

    // One-time schema coverage warning: never silently drop a field the Rust
    // side added but this converter doesn't know about.
    if (!schema_checked) {
      schema_checked = true;
      for (const auto& fd : d.schema().fields("EventData"))
        if (fd.name.rfind("digital_probe", 0) != 0 && !handled_event.count(fd.name))
          std::fprintf(stderr, "delila2root: NOTE unhandled EventData field '%s' (%s) — add a branch\n",
                       fd.name.c_str(), fd.tag.c_str());
      for (const auto& fd : d.schema().fields("Waveform"))
        if (fd.name.rfind("digital_probe", 0) != 0 && !handled_wave.count(fd.name))
          std::fprintf(stderr, "delila2root: NOTE unhandled Waveform field '%s' (%s) — add a branch\n",
                       fd.name.c_str(), fd.tag.c_str());
      std::printf("delila2root: %s v%u, run %u '%s'\n", path.c_str(),
                  d.header().version, d.header().run_number, d.header().exp_name.c_str());
    }

    tdelila::Event ev;
    while (d.next(ev)) {
      module = (UChar_t)ev.module();
      channel = (UChar_t)ev.channel();
      energy = (UShort_t)ev.energy();
      energy_short = (UShort_t)ev.energy_short();
      timestamp_ns = ev.timestamp_ns();
      flags = ev.flags();
      for (int k = 0; k < 4; ++k) user_info[k] = ev.user_info(k);

      has_waveform = ev.has_waveform();
      // reset waveform branches
      time_resolution = 0;
      trigger_threshold = 0;
      ns_per_sample = 0.0;
      for (int k = 0; k < 3; ++k) { analog_probe[k].clear(); analog_probe_is_signed[k] = false; analog_probe_type[k] = 0; }
      for (int k = 0; k < 16; ++k) { digital_probe[k].clear(); digital_probe_type[k] = 0; }

      if (has_waveform) {
        const auto& wf = ev.waveform();
        for (int k = 0; k < 3; ++k) {
          analog_probe[k] = wf.analog_probe(k + 1);
          analog_probe_is_signed[k] = wf.analog_probe_is_signed(k + 1);
        }
        for (int k = 0; k < 16; ++k) {
          auto d8 = wf.digital_probe(k + 1);
          digital_probe[k].assign(d8.begin(), d8.end());
        }
        ns_per_sample = wf.ns_per_sample();
        trigger_threshold = (UShort_t)wf.trigger_threshold();
        if (const tdelila::mp::Value* tr = wf.field("time_resolution")) time_resolution = (UChar_t)tr->as_u64();
        if (const tdelila::mp::Value* at = wf.field("analog_probe_type"))
          for (size_t k = 0; k < at->arr.size() && k < 3; ++k) analog_probe_type[k] = (UChar_t)at->arr[k].as_u64();
        if (const tdelila::mp::Value* dt = wf.field("digital_probe_type"))
          for (size_t k = 0; k < dt->arr.size() && k < 16; ++k) digital_probe_type[k] = (UChar_t)dt->arr[k].as_u64();
      }

      tree->Fill();
      total++;
    }
    if (d.footer().present && d.events_returned() != d.footer().total_events)
      std::fprintf(stderr, "delila2root: WARNING %s: read %llu events but footer says %llu\n",
                   path.c_str(), (unsigned long long)d.events_returned(),
                   (unsigned long long)d.footer().total_events);
  }

  tree->Write();
  f->Close();
  delete f;
  std::printf("delila2root: wrote %ld events -> %s\n", total, out.c_str());
  return total;
}

#ifndef __CLING__
int main(int argc, char** argv) {
  if (argc < 2) {
    std::fprintf(stderr, "usage: %s in.delila [out.root] [in2.delila ...]\n", argv[0]);
    return 1;
  }
  const char* in = argv[1];
  const char* out = nullptr;
  std::vector<std::string> extra;
  // argv[2] is out.root if it ends in .root, otherwise an additional input.
  int i = 2;
  if (argc >= 3) {
    std::string a2 = argv[2];
    if (a2.size() >= 5 && a2.substr(a2.size() - 5) == ".root") { out = argv[2]; i = 3; }
  }
  for (; i < argc; ++i) extra.push_back(argv[i]);
  return delila2root(in, out, extra) < 0 ? 1 : 0;
}
#endif
