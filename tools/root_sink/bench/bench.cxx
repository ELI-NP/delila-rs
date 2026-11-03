// bench.cxx — ROOT parallel-write benchmark for a DAQ-like hit stream.
//
// Build:  g++ -O2 -std=c++17 bench.cxx $(root-config --cflags --libs) -lROOTNTuple -o bench
// Usage:  ./bench MODE [options]      (see print_usage)
//
// Modes:
//   ttree        one thread, TTree hits (5 scalar branches), optional IMT
//   tbm          ROOT::TBufferMerger, N filler threads each with its own TTree
//   rntuple      one RNTupleWriter, optional IMT (parallel page compression)
//   rntuple-par  RNTupleParallelWriter, N fill contexts (optionally ordered)
//   events-ttree / events-rntuple   event layout (variable-length hit arrays)
//   twofile      two threads: hits TTree -> file A, events TTree -> file B
//   ttree2       one thread, hits + events TTrees in the SAME file (interleaved)
//
// Every mode prints one CSV line:
//   mode,params,order,n_hits,wall_s,cpu_s,cores,hits_per_s,file_MB,file_MB_s,ratio,inversions,maxrss_MB
#include <sys/resource.h>
#include <sys/stat.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "ROOT/RNTupleFillContext.hxx"
#include "ROOT/RNTupleModel.hxx"
#include "ROOT/RNTupleParallelWriter.hxx"
#include "ROOT/RNTupleReader.hxx"
#include "ROOT/RNTupleWriteOptions.hxx"
#include "ROOT/RNTupleWriter.hxx"
#include "ROOT/TBufferMerger.hxx"
#include "TFile.h"
#include "TROOT.h"
#include "TTree.h"

#include "hits_gen.hpp"

using Clock = std::chrono::steady_clock;

// ----------------------------------------------------------------------------
struct Opt {
  std::string mode;
  std::string out = "bench_out.root";
  std::string cache;        // binary cache of generated hits
  std::size_t n = 30'000'000;
  std::string order = "sorted";  // sorted | board
  int comp = 505;
  int imt = 0;              // 0 = off, else EnableImplicitMT(imt)
  int threads = 4;          // tbm / rntuple-par fillers
  std::size_t block = 1'000'000;  // hits per block handed to a filler
  int basket = 32000;       // TTree branch buffer size
  long long autoflush = 0;  // 0 = ROOT default
  std::size_t cluster_mb = 0;  // RNTuple approx zipped cluster size (0 = default)
  bool ordered = false;     // tbm / rntuple-par: serialize commits in block order
  bool fast_merge = false;  // tbm: SetMergeOptions("fast")
  bool check = false;       // read back and count inversions
  bool keep = false;
  bool no_buffered = false; // rntuple: SetUseBufferedWrite(false)
  int max_ev_hits = 64;
};

static void print_usage() {
  std::fprintf(stderr,
               "bench MODE [--n N] [--order sorted|board] [--comp C] [--imt T] [--threads T]\n"
               "      [--block N] [--basket B] [--autoflush N] [--cluster-mb M] [--ordered]\n"
               "      [--fast-merge] [--check] [--keep] [--out F] [--cache F] [--no-buffered]\n");
}

static Opt parse(int argc, char** argv) {
  Opt o;
  if (argc < 2) { print_usage(); std::exit(2); }
  o.mode = argv[1];
  for (int i = 2; i < argc; ++i) {
    std::string a = argv[i];
    auto next = [&]() -> std::string { if (i + 1 >= argc) { print_usage(); std::exit(2); } return argv[++i]; };
    if (a == "--n") o.n = std::stoull(next());
    else if (a == "--order") o.order = next();
    else if (a == "--comp") o.comp = std::stoi(next());
    else if (a == "--imt") o.imt = std::stoi(next());
    else if (a == "--threads") o.threads = std::stoi(next());
    else if (a == "--block") o.block = std::stoull(next());
    else if (a == "--basket") o.basket = std::stoi(next());
    else if (a == "--autoflush") o.autoflush = std::stoll(next());
    else if (a == "--cluster-mb") o.cluster_mb = std::stoull(next());
    else if (a == "--ordered") o.ordered = true;
    else if (a == "--fast-merge") o.fast_merge = true;
    else if (a == "--check") o.check = true;
    else if (a == "--keep") o.keep = true;
    else if (a == "--no-buffered") o.no_buffered = true;
    else if (a == "--out") o.out = next();
    else if (a == "--cache") o.cache = next();
    else { std::fprintf(stderr, "unknown option %s\n", a.c_str()); print_usage(); std::exit(2); }
  }
  return o;
}

// ----------------------------------------------------------------------------
static double cpu_seconds() {
  rusage ru{};
  getrusage(RUSAGE_SELF, &ru);
  return ru.ru_utime.tv_sec + ru.ru_utime.tv_usec * 1e-6 + ru.ru_stime.tv_sec + ru.ru_stime.tv_usec * 1e-6;
}
static double maxrss_mb() {
  rusage ru{};
  getrusage(RUSAGE_SELF, &ru);
#ifdef __APPLE__
  return ru.ru_maxrss / 1048576.0;
#else
  return ru.ru_maxrss / 1024.0;
#endif
}
static long long file_size(const std::string& p) {
  struct stat st{};
  if (::stat(p.c_str(), &st) != 0) return -1;
  return st.st_size;
}

// Load (or generate + cache) the hit stream.
static std::vector<Hit> load_hits(const Opt& o) {
  GenConfig cfg;
  cfg.n = o.n;
  cfg.board_order = (o.order == "board");
  std::string cache = o.cache;
  if (!cache.empty()) {
    cache += "." + o.order + "." + std::to_string(o.n) + ".bin";
    std::ifstream in(cache, std::ios::binary);
    if (in) {
      std::vector<Hit> hits(o.n);
      in.read(reinterpret_cast<char*>(hits.data()), sizeof(Hit) * o.n);
      if (in.gcount() == static_cast<std::streamsize>(sizeof(Hit) * o.n)) return hits;
    }
  }
  auto t0 = Clock::now();
  auto hits = generate_hits(cfg);
  double dt = std::chrono::duration<double>(Clock::now() - t0).count();
  std::fprintf(stderr, "# generated %zu hits in %.1f s (%s)\n", hits.size(), dt, o.order.c_str());
  if (!cache.empty()) {
    std::ofstream out(cache, std::ios::binary);
    out.write(reinterpret_cast<const char*>(hits.data()), sizeof(Hit) * hits.size());
  }
  return hits;
}

// Event boundaries: deterministic variable-length grouping of consecutive hits
// (1 + geometric, mean ~5, capped). Returns start index of each event (+ end).
static std::vector<std::size_t> event_bounds(std::size_t n, int cap) {
  std::mt19937_64 rng(777);
  std::geometric_distribution<int> g(0.2);
  std::vector<std::size_t> b;
  b.reserve(n / 4);
  std::size_t i = 0;
  while (i < n) {
    b.push_back(i);
    int k = 1 + g(rng);
    if (k > cap) k = cap;
    i += k;
  }
  b.push_back(n);
  return b;
}

// Ordered-commit ticket: block b may commit only after block b-1 committed.
struct Ticket {
  std::mutex m;
  std::condition_variable cv;
  std::size_t next = 0;
  void wait_for(std::size_t b) {
    std::unique_lock<std::mutex> lk(m);
    cv.wait(lk, [&] { return next == b; });
  }
  void advance() {
    { std::lock_guard<std::mutex> lk(m); ++next; }
    cv.notify_all();
  }
};

// ----------------------------------------------------------------------------
// TTree hits branches bound to a Hit-like scalar set.
struct HitScalars {
  UChar_t module{}, channel{};
  UShort_t energy{}, energy_short{};
  Double_t ts{};
  void bind(TTree* t, int basket) {
    t->Branch("module", &module, "module/b", basket);
    t->Branch("channel", &channel, "channel/b", basket);
    t->Branch("energy", &energy, "energy/s", basket);
    t->Branch("energy_short", &energy_short, "energy_short/s", basket);
    t->Branch("timestamp_ns", &ts, "timestamp_ns/D", basket);
  }
  void set(const Hit& h) {
    module = h.module; channel = h.channel; energy = h.energy; energy_short = h.energy_short; ts = h.timestamp_ns;
  }
};

// TTree events branches (variable-length arrays counted by NHits).
struct EventArrays {
  ULong64_t id{};
  Double_t trig_t{};
  UInt_t nhits{};
  std::vector<UChar_t> mod, ch;
  std::vector<UShort_t> e, es;
  std::vector<Double_t> dt;
  explicit EventArrays(int cap) : mod(cap), ch(cap), e(cap), es(cap), dt(cap) {}
  void bind(TTree* t, int basket) {
    t->Branch("EventID", &id, "EventID/l", basket);
    t->Branch("TriggerTime", &trig_t, "TriggerTime/D", basket);
    t->Branch("NHits", &nhits, "NHits/i", basket);
    t->Branch("HitMod", mod.data(), "HitMod[NHits]/b", basket);
    t->Branch("HitCh", ch.data(), "HitCh[NHits]/b", basket);
    t->Branch("HitEnergy", e.data(), "HitEnergy[NHits]/s", basket);
    t->Branch("HitEnergyShort", es.data(), "HitEnergyShort[NHits]/s", basket);
    t->Branch("HitTime", dt.data(), "HitTime[NHits]/D", basket);
  }
  void set(ULong64_t eid, const Hit* h, std::size_t k) {
    id = eid; trig_t = h[0].timestamp_ns; nhits = static_cast<UInt_t>(k);
    for (std::size_t i = 0; i < k; ++i) {
      mod[i] = h[i].module; ch[i] = h[i].channel; e[i] = h[i].energy; es[i] = h[i].energy_short;
      dt[i] = h[i].timestamp_ns - trig_t;
    }
  }
};

// ----------------------------------------------------------------------------
static void write_ttree_hits(const Opt& o, const std::vector<Hit>& hits, const std::string& out) {
  TFile* f = TFile::Open(out.c_str(), "RECREATE", "", o.comp);
  TTree* t = new TTree("delila", "hits");
  if (o.autoflush) t->SetAutoFlush(o.autoflush);
  HitScalars s;
  s.bind(t, o.basket);
  for (const Hit& h : hits) { s.set(h); t->Fill(); }
  f->Write();
  f->Close();
  delete f;
}

static void write_ttree_events(const Opt& o, const std::vector<Hit>& hits, const std::vector<std::size_t>& eb,
                               const std::string& out) {
  TFile* f = TFile::Open(out.c_str(), "RECREATE", "", o.comp);
  TTree* t = new TTree("events", "events");
  if (o.autoflush) t->SetAutoFlush(o.autoflush);
  EventArrays ev(o.max_ev_hits);
  ev.bind(t, o.basket);
  for (std::size_t i = 0; i + 1 < eb.size(); ++i) {
    ev.set(i, &hits[eb[i]], eb[i + 1] - eb[i]);
    t->Fill();
  }
  f->Write();
  f->Close();
  delete f;
}

// One thread, both trees in one file, interleaved (event filled after its hits).
static void write_ttree_both(const Opt& o, const std::vector<Hit>& hits, const std::vector<std::size_t>& eb,
                             const std::string& out) {
  TFile* f = TFile::Open(out.c_str(), "RECREATE", "", o.comp);
  TTree* th = new TTree("delila", "hits");
  TTree* te = new TTree("events", "events");
  if (o.autoflush) { th->SetAutoFlush(o.autoflush); te->SetAutoFlush(o.autoflush); }
  HitScalars s;
  s.bind(th, o.basket);
  EventArrays ev(o.max_ev_hits);
  ev.bind(te, o.basket);
  for (std::size_t i = 0; i + 1 < eb.size(); ++i) {
    for (std::size_t k = eb[i]; k < eb[i + 1]; ++k) { s.set(hits[k]); th->Fill(); }
    ev.set(i, &hits[eb[i]], eb[i + 1] - eb[i]);
    te->Fill();
  }
  f->Write();
  f->Close();
  delete f;
}

// ----------------------------------------------------------------------------
static void write_tbm(const Opt& o, const std::vector<Hit>& hits, const std::string& out) {
  ROOT::TBufferMerger merger(out.c_str(), "RECREATE", o.comp);
  if (o.fast_merge) merger.SetMergeOptions("fast");
  const std::size_t nblocks = (hits.size() + o.block - 1) / o.block;
  Ticket ticket;
  auto worker = [&](int tid) {
    auto f = merger.GetFile();
    f->cd();
    TTree* t = new TTree("delila", "hits");
    t->ResetBit(kMustCleanup);
    if (o.autoflush) t->SetAutoFlush(o.autoflush);
    HitScalars s;
    s.bind(t, o.basket);
    for (std::size_t b = tid; b < nblocks; b += o.threads) {
      const std::size_t lo = b * o.block, hi = std::min(hits.size(), lo + o.block);
      for (std::size_t k = lo; k < hi; ++k) { s.set(hits[k]); t->Fill(); }
      if (o.ordered) ticket.wait_for(b);
      f->Write();  // compress pending baskets here, then merge under the merger mutex
      if (o.ordered) ticket.advance();
    }
    delete t;
    f.reset();
  };
  std::vector<std::thread> th;
  for (int i = 0; i < o.threads; ++i) th.emplace_back(worker, i);
  for (auto& x : th) x.join();
  // merger destructor writes the output tree keys and closes the file
}

// ----------------------------------------------------------------------------
static ROOT::RNTupleWriteOptions ntuple_opts(const Opt& o, bool parallel) {
  ROOT::RNTupleWriteOptions w;
  w.SetCompression(o.comp);
  if (o.cluster_mb) w.SetApproxZippedClusterSize(o.cluster_mb * 1024 * 1024);
  if (o.no_buffered) w.SetUseBufferedWrite(false);
  if (parallel && o.imt > 0) w.SetUseImplicitMT(ROOT::RNTupleWriteOptions::EImplicitMT::kOn);
  if (!parallel && o.imt == 0) w.SetUseImplicitMT(ROOT::RNTupleWriteOptions::EImplicitMT::kOff);
  return w;
}

static std::unique_ptr<ROOT::RNTupleModel> hits_model(bool bare) {
  auto m = bare ? ROOT::RNTupleModel::CreateBare() : ROOT::RNTupleModel::Create();
  m->MakeField<std::uint8_t>("module");
  m->MakeField<std::uint8_t>("channel");
  m->MakeField<std::uint16_t>("energy");
  m->MakeField<std::uint16_t>("energy_short");
  m->MakeField<double>("timestamp_ns");
  return m;
}

static void write_rntuple_hits(const Opt& o, const std::vector<Hit>& hits, const std::string& out) {
  auto model = hits_model(false);
  auto pm = model->GetDefaultEntry().GetPtr<std::uint8_t>("module");
  auto pc = model->GetDefaultEntry().GetPtr<std::uint8_t>("channel");
  auto pe = model->GetDefaultEntry().GetPtr<std::uint16_t>("energy");
  auto pes = model->GetDefaultEntry().GetPtr<std::uint16_t>("energy_short");
  auto pt = model->GetDefaultEntry().GetPtr<double>("timestamp_ns");
  auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "delila", out, ntuple_opts(o, false));
  for (const Hit& h : hits) {
    *pm = h.module; *pc = h.channel; *pe = h.energy; *pes = h.energy_short; *pt = h.timestamp_ns;
    writer->Fill();
  }
  writer.reset();
}

static void write_rntuple_par(const Opt& o, const std::vector<Hit>& hits, const std::string& out) {
  auto writer = ROOT::RNTupleParallelWriter::Recreate(hits_model(true), "delila", out, ntuple_opts(o, true));
  const std::size_t nblocks = (hits.size() + o.block - 1) / o.block;
  Ticket ticket;
  auto worker = [&](int tid) {
    auto ctx = writer->CreateFillContext();
    if (o.ordered) ctx->EnableStagedClusterCommitting();
    auto entry = ctx->CreateEntry();
    auto pm = entry->GetPtr<std::uint8_t>("module");
    auto pc = entry->GetPtr<std::uint8_t>("channel");
    auto pe = entry->GetPtr<std::uint16_t>("energy");
    auto pes = entry->GetPtr<std::uint16_t>("energy_short");
    auto pt = entry->GetPtr<double>("timestamp_ns");
    for (std::size_t b = tid; b < nblocks; b += o.threads) {
      const std::size_t lo = b * o.block, hi = std::min(hits.size(), lo + o.block);
      for (std::size_t k = lo; k < hi; ++k) {
        const Hit& h = hits[k];
        *pm = h.module; *pc = h.channel; *pe = h.energy; *pes = h.energy_short; *pt = h.timestamp_ns;
        ctx->Fill(*entry);
      }
      ctx->FlushCluster();  // compress + write this block's pages (staged if ordered)
      if (o.ordered) {
        ticket.wait_for(b);
        ctx->CommitStagedClusters();  // logical commit only, in block order
        ticket.advance();
      }
    }
    entry.reset();
    ctx.reset();
  };
  std::vector<std::thread> th;
  for (int i = 0; i < o.threads; ++i) th.emplace_back(worker, i);
  for (auto& x : th) x.join();
  writer.reset();
}

static void write_rntuple_events(const Opt& o, const std::vector<Hit>& hits, const std::vector<std::size_t>& eb,
                                 const std::string& out) {
  auto model = ROOT::RNTupleModel::Create();
  auto id = model->MakeField<std::uint64_t>("EventID");
  auto tt = model->MakeField<double>("TriggerTime");
  auto mod = model->MakeField<std::vector<std::uint8_t>>("HitMod");
  auto ch = model->MakeField<std::vector<std::uint8_t>>("HitCh");
  auto e = model->MakeField<std::vector<std::uint16_t>>("HitEnergy");
  auto es = model->MakeField<std::vector<std::uint16_t>>("HitEnergyShort");
  auto dt = model->MakeField<std::vector<double>>("HitTime");
  auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "events", out, ntuple_opts(o, false));
  for (std::size_t i = 0; i + 1 < eb.size(); ++i) {
    const std::size_t k = eb[i + 1] - eb[i];
    const Hit* h = &hits[eb[i]];
    *id = i; *tt = h[0].timestamp_ns;
    mod->resize(k); ch->resize(k); e->resize(k); es->resize(k); dt->resize(k);
    for (std::size_t j = 0; j < k; ++j) {
      (*mod)[j] = h[j].module; (*ch)[j] = h[j].channel; (*e)[j] = h[j].energy; (*es)[j] = h[j].energy_short;
      (*dt)[j] = h[j].timestamp_ns - *tt;
    }
    writer->Fill();
  }
  writer.reset();
}

// ----------------------------------------------------------------------------
static std::size_t check_ttree(const std::string& out, const char* tree, const char* br, std::size_t& nread) {
  std::unique_ptr<TFile> f(TFile::Open(out.c_str()));
  TTree* t = f ? f->Get<TTree>(tree) : nullptr;
  if (!t) { nread = 0; return 0; }
  t->SetBranchStatus("*", false);
  t->SetBranchStatus(br, true);
  Double_t ts = 0;
  t->SetBranchAddress(br, &ts);
  const Long64_t n = t->GetEntries();
  std::vector<double> v;
  v.reserve(n);
  for (Long64_t i = 0; i < n; ++i) { t->GetEntry(i); v.push_back(ts); }
  nread = v.size();
  return count_inversions(v);
}

static std::size_t check_rntuple(const std::string& out, const char* name, const char* field, std::size_t& nread) {
  auto reader = ROOT::RNTupleReader::Open(name, out);
  auto view = reader->GetView<double>(field);
  std::vector<double> v;
  v.reserve(reader->GetNEntries());
  for (auto i : reader->GetEntryRange()) v.push_back(view(i));
  nread = v.size();
  return count_inversions(v);
}

// ----------------------------------------------------------------------------
int main(int argc, char** argv) {
  Opt o = parse(argc, argv);
  ROOT::EnableThreadSafety();
  if (o.imt > 0) ROOT::EnableImplicitMT(o.imt);

  std::vector<Hit> hits = load_hits(o);
  std::vector<std::size_t> eb;
  const bool needs_events = o.mode.rfind("events", 0) == 0 || o.mode == "twofile" || o.mode == "ttree2";
  if (needs_events) eb = event_bounds(hits.size(), o.max_ev_hits);
  const std::string out2 = o.out + ".events.root";

  const double cpu0 = cpu_seconds();
  const auto t0 = Clock::now();
  if (o.mode == "ttree") write_ttree_hits(o, hits, o.out);
  else if (o.mode == "tbm") write_tbm(o, hits, o.out);
  else if (o.mode == "rntuple") write_rntuple_hits(o, hits, o.out);
  else if (o.mode == "rntuple-par") write_rntuple_par(o, hits, o.out);
  else if (o.mode == "events-ttree") write_ttree_events(o, hits, eb, o.out);
  else if (o.mode == "events-rntuple") write_rntuple_events(o, hits, eb, o.out);
  else if (o.mode == "ttree2") write_ttree_both(o, hits, eb, o.out);
  else if (o.mode == "twofile") {
    std::thread a([&] { write_ttree_hits(o, hits, o.out); });
    std::thread b([&] { write_ttree_events(o, hits, eb, out2); });
    a.join(); b.join();
  } else { std::fprintf(stderr, "unknown mode %s\n", o.mode.c_str()); return 2; }
  const double wall = std::chrono::duration<double>(Clock::now() - t0).count();
  const double cpu = cpu_seconds() - cpu0;

  long long bytes = file_size(o.out);
  if (o.mode == "twofile") bytes += file_size(out2);
  const double raw = 14.0 * hits.size() + (needs_events ? 20.0 * (eb.size() - 1) : 0.0);

  long long inv = -1;
  std::size_t nread = 0;
  if (o.check) {
    if (o.mode == "rntuple" || o.mode == "rntuple-par") inv = check_rntuple(o.out, "delila", "timestamp_ns", nread);
    else if (o.mode == "events-rntuple") inv = check_rntuple(o.out, "events", "TriggerTime", nread);
    else if (o.mode == "events-ttree") inv = check_ttree(o.out, "events", "TriggerTime", nread);
    else inv = check_ttree(o.out, "delila", "timestamp_ns", nread);
    if (nread != (o.mode.rfind("events", 0) == 0 ? eb.size() - 1 : hits.size()))
      std::fprintf(stderr, "# WARNING entry count mismatch: read %zu\n", nread);
  }

  char params[256];
  std::snprintf(params, sizeof params, "comp=%d imt=%d thr=%d block=%zu basket=%d af=%lld clu=%zu%s%s%s",
                o.comp, o.imt, o.threads, o.block, o.basket, o.autoflush, o.cluster_mb,
                o.ordered ? " ordered" : "", o.fast_merge ? " fast" : "", o.no_buffered ? " nobuf" : "");
  std::printf("%s,\"%s\",%s,%zu,%.2f,%.2f,%.2f,%.0f,%.1f,%.1f,%.2f,%lld,%.0f\n", o.mode.c_str(), params,
              o.order.c_str(), hits.size(), wall, cpu, cpu / wall, hits.size() / wall, bytes / 1e6,
              bytes / 1e6 / wall, raw / bytes, inv, maxrss_mb());
  if (!o.keep) { std::remove(o.out.c_str()); std::remove(out2.c_str()); }
  return 0;
}
