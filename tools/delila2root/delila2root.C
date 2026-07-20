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
//   ./delila2root -o out.root --tree tr in_0000.delila in_0001.delila   # Rust-CLI compatible
//
// License: BSD-3-Clause (same as delila-rs).

#include <cstdio>
#include <set>
#include <string>
#include <vector>

#include "TDelila.hpp"
#include "TFile.h"
#include "TROOT.h"
#include "TTree.h"

// ROOT compression setting = algorithm*100 + level. ZSTD is algorithm 5, so
// 505 = ZSTD level 5. Using the integer avoids the ROOT::kZSTD enum, whose
// namespace moved across ROOT versions. (Fallback if ZSTD is unavailable on an
// old ROOT: 404 = LZ4 level 4.)
static const int kDelilaCompression = 505;

// Convert one or more `.delila` files (same run) into a single ROOT tree.
// Returns the number of events written, or -1 on error.
// `tree_name` defaults to "delila"; the CLI's `--tree` maps here (the old
// Rust tool's converter scripts pass `--tree tr`).
long delila2root(const char* in_path, const char* out_path = nullptr,
                 const std::vector<std::string>& extra = {},
                 const char* tree_name = "delila") {
  // Parallelize ROOT's basket compression (ZSTD-5) across cores. Previously the
  // converter was single-threaded: 100% of one core, compression on the same
  // thread as decode. Works in both compiled and macro (Cling) mode.
  ROOT::EnableImplicitMT();

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
  TTree* tree = new TTree(tree_name, "DELILA events");

  // --- Scalar branch buffers (one per event field; all firmwares) ---
  UChar_t  module = 0, channel = 0;
  UShort_t energy = 0, energy_short = 0;
  Double_t timestamp_ns = 0.0;
  ULong64_t flags = 0;
  ULong64_t user_info[4] = {0, 0, 0, 0};
  Bool_t has_waveform = false;

  // --- Waveform branch buffer ---
  // Decoded straight into this reused struct (no per-event Value DOM): the
  // ROOT branches point at its members. Its bool/uint8_t/uint16_t/double
  // members are layout-compatible with the Bool_t(1B)/UChar_t/UShort_t/Double_t
  // leaflists below, so the tree's on-disk types are unchanged.
  tdelila::DecodedWaveform dw;

  tree->Branch("module", &module, "module/b");
  tree->Branch("channel", &channel, "channel/b");
  tree->Branch("energy", &energy, "energy/s");
  tree->Branch("energy_short", &energy_short, "energy_short/s");
  tree->Branch("timestamp_ns", &timestamp_ns, "timestamp_ns/D");
  tree->Branch("flags", &flags, "flags/l");
  tree->Branch("user_info", user_info, "user_info[4]/l");
  tree->Branch("has_waveform", &has_waveform, "has_waveform/O");
  tree->Branch("time_resolution", &dw.time_resolution, "time_resolution/b");
  tree->Branch("trigger_threshold", &dw.trigger_threshold, "trigger_threshold/s");
  tree->Branch("ns_per_sample", &dw.ns_per_sample, "ns_per_sample/D");
  for (int k = 0; k < 3; ++k)
    tree->Branch(Form("analog_probe%d", k + 1), &dw.analog[k]);
  for (int k = 0; k < 16; ++k)
    tree->Branch(Form("digital_probe%d", k + 1), &dw.digital[k]);
  tree->Branch("analog_probe1_is_signed", &dw.analog_is_signed[0], "analog_probe1_is_signed/O");
  tree->Branch("analog_probe2_is_signed", &dw.analog_is_signed[1], "analog_probe2_is_signed/O");
  tree->Branch("analog_probe3_is_signed", &dw.analog_is_signed[2], "analog_probe3_is_signed/O");
  tree->Branch("analog_probe_type", dw.analog_type, "analog_probe_type[3]/b");
  tree->Branch("digital_probe_type", dw.digital_type, "digital_probe_type[16]/b");

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

      // Typed decode straight into dw's branch buffers (both branches clear dw).
      has_waveform = ev.has_waveform();
      if (has_waveform) ev.decode_waveform(dw);
      else dw.clear();

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
// CLI accepts both the positional form and the old Rust tool's flags, so
// existing converter scripts (`delila2root -o out.root --tree tr in_00*`)
// keep working unchanged:
//   delila2root in.delila [out.root] [in2.delila ...]
//   delila2root -o out.root [--tree name] in.delila [in2.delila ...]
int main(int argc, char** argv) {
  std::string out;
  std::string tree_name = "delila";
  std::vector<std::string> inputs;
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    if ((a == "-o" || a == "--output") && i + 1 < argc) {
      out = argv[++i];
    } else if (a == "--tree" && i + 1 < argc) {
      tree_name = argv[++i];
    } else if (out.empty() && a.size() >= 5 && a.substr(a.size() - 5) == ".root") {
      out = a;  // positional out.root (legacy form)
    } else {
      inputs.push_back(a);
    }
  }
  if (inputs.empty()) {
    std::fprintf(stderr,
                 "usage: %s [-o out.root] [--tree name] in.delila [in2.delila ...]\n"
                 "       %s in.delila [out.root] [in2.delila ...]\n",
                 argv[0], argv[0]);
    return 1;
  }
  std::vector<std::string> extra(inputs.begin() + 1, inputs.end());
  return delila2root(inputs[0].c_str(), out.empty() ? nullptr : out.c_str(), extra,
                     tree_name.c_str()) < 0
             ? 1
             : 0;
}
#endif
