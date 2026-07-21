// root_sink.cxx — DELILA parallel ROOT sink + live Δt monitor.
//
// One process, two roles, subscribing to the DELILA merger's ZMQ PUB as an
// ADDITIONAL consumer (the Rust Recorder that writes `.delila` stays the
// authoritative recorder — this tool never gates or throttles the pipeline):
//
//   Role 1 (recorder): decode every event to 5 scalar fields and Fill a flat,
//     ZSTD-compressed ROOT TTree. One file per run, named on the EOS-carried
//     run number. Cheaper than the two-step `.delila` -> delila2root path when
//     only scalars are needed.
//
//   Role 2 (monitor): a coincidence Δt monitor for the ThGEM test (single
//     digitizer, gamma + ThGEM1 + ThGEM2 on configurable channels). Three
//     histograms (dt1, dt2, dt2-vs-dt1) plus channel occupancy, served live over
//     THttpServer (browse with a JSROOT-capable browser — zero frontend code).
//
// All logic (envelope parse, decode, matcher, run state) is in sink_core.hpp and
// is unit-tested without ROOT/ZMQ. This file is only the wiring.
//
// Build (see README.md):
//   g++ -O2 -std=c++17 root_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o root_sink
//
// License: BSD-3-Clause (same as delila-rs).

#include <zmq.h>

#include <sys/stat.h>

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <memory>
#include <set>
#include <string>
#include <vector>

#include "sink_core.hpp"

#include "TApplication.h"
#include "TFile.h"
#include "TH1.h"
#include "TH2.h"
#include "THttpServer.h"
#include "TInterpreter.h"
#include "TROOT.h"
#include "TString.h"
#include "TSystem.h"
#include "TTree.h"

using namespace rootsink;
using Clock = std::chrono::steady_clock;

// ROOT compression = algorithm*100 + level. 505 = ZSTD level 5, the same
// constant delila2root uses (kDelilaCompression). Kept as an int to avoid the
// ROOT::kZSTD enum, whose namespace moved across versions.
static const int kDelilaCompression = 505;

// ---------------------------------------------------------------------------
// Signals: SIGINT (Ctrl-C) + SIGTERM (pkill) flip a flag; the main loop drains
// and closes the file cleanly. (See MEMORY sigterm_handler_required: ctrl_c only
// covers SIGINT; a plain SIGTERM would otherwise leave the file unfinalized.)
// ---------------------------------------------------------------------------
static volatile sig_atomic_t g_stop = 0;
static void on_signal(int) { g_stop = 1; }

// ---------------------------------------------------------------------------
// Monitor histograms — file-scope so the /Reset HTTP command (executed by Cling
// on the main thread) can reach them via baked-in pointers.
// ---------------------------------------------------------------------------
static TH1D* g_h_dt1 = nullptr;
static TH1D* g_h_dt2 = nullptr;
static TH2D* g_h_dt2_vs_dt1 = nullptr;
static TH1I* g_h_channels = nullptr;

// ---------------------------------------------------------------------------
// CLI options
// ---------------------------------------------------------------------------
struct Options {
  std::string zmq = "tcp://localhost:5557";
  std::string out_dir = ".";
  std::string tree = "tr";
  int gamma_ch = -1;
  int thgem1_ch = -1;
  int thgem2_ch = -1;
  double window_ns = 1000.0;
  double margin_ns = 10000.0;
  int http_port = 8090;
  int dt_bins = 2000;
  double dt_min = -1000.0;
  double dt_max = 1000.0;
  int autosave_sec = 30;
};

static void print_usage(const char* argv0) {
  std::printf(
      "usage: %s [options]\n"
      "  --zmq ADDR          merger PUB endpoint (default tcp://localhost:5557)\n"
      "  --out-dir DIR       output directory for run*.root (default .)\n"
      "  --tree NAME         TTree name (default tr)\n"
      "  --gamma-ch N        gamma detector channel (enables Δt monitor)\n"
      "  --thgem1-ch N       ThGEM1 channel\n"
      "  --thgem2-ch N       ThGEM2 channel\n"
      "                      (all three required; if any omitted -> recorder only)\n"
      "  --window-ns X       coincidence half-window (default 1000)\n"
      "  --margin-ns X       out-of-order tolerance / ripen delay (default 10000)\n"
      "  --http-port N       THttpServer port, 0 disables (default 8090)\n"
      "  --dt-bins N         Δt histogram bins (default 2000)\n"
      "  --dt-min X          Δt axis min ns (default -1000)\n"
      "  --dt-max X          Δt axis max ns (default 1000)\n"
      "  --autosave-sec N    TTree AutoSave interval (default 30)\n"
      "  --help              this message\n",
      argv0);
}

// Parse argv into opt. Returns false to stop (help printed, or a bad flag).
static bool parse_args(int argc, char** argv, Options& opt, bool& help) {
  help = false;
  auto need = [&](int& i) -> const char* {
    if (i + 1 >= argc) {
      std::fprintf(stderr, "root_sink: missing value for %s\n", argv[i]);
      return nullptr;
    }
    return argv[++i];
  };
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    const char* v = nullptr;
    if (a == "--help" || a == "-h") {
      print_usage(argv[0]);
      help = true;
      return false;
    } else if (a == "--zmq") {
      if (!(v = need(i))) return false;
      opt.zmq = v;
    } else if (a == "--out-dir") {
      if (!(v = need(i))) return false;
      opt.out_dir = v;
    } else if (a == "--tree") {
      if (!(v = need(i))) return false;
      opt.tree = v;
    } else if (a == "--gamma-ch") {
      if (!(v = need(i))) return false;
      opt.gamma_ch = std::atoi(v);
    } else if (a == "--thgem1-ch") {
      if (!(v = need(i))) return false;
      opt.thgem1_ch = std::atoi(v);
    } else if (a == "--thgem2-ch") {
      if (!(v = need(i))) return false;
      opt.thgem2_ch = std::atoi(v);
    } else if (a == "--window-ns") {
      if (!(v = need(i))) return false;
      opt.window_ns = std::atof(v);
    } else if (a == "--margin-ns") {
      if (!(v = need(i))) return false;
      opt.margin_ns = std::atof(v);
    } else if (a == "--http-port") {
      if (!(v = need(i))) return false;
      opt.http_port = std::atoi(v);
    } else if (a == "--dt-bins") {
      if (!(v = need(i))) return false;
      opt.dt_bins = std::atoi(v);
    } else if (a == "--dt-min") {
      if (!(v = need(i))) return false;
      opt.dt_min = std::atof(v);
    } else if (a == "--dt-max") {
      if (!(v = need(i))) return false;
      opt.dt_max = std::atof(v);
    } else if (a == "--autosave-sec") {
      if (!(v = need(i))) return false;
      opt.autosave_sec = std::atoi(v);
    } else {
      std::fprintf(stderr, "root_sink: unknown argument '%s' (try --help)\n", a.c_str());
      return false;
    }
  }
  return true;
}

// ---------------------------------------------------------------------------
// ROOT scalar recorder
// ---------------------------------------------------------------------------
static bool path_exists(const std::string& p) {
  struct stat st;
  return ::stat(p.c_str(), &st) == 0;
}

class Recorder {
 public:
  bool is_open() const { return file_ != nullptr; }
  const std::string& provisional() const { return provisional_; }
  long long entries() const { return entries_; }

  // Open a provisional file `<dir>/run_inprogress_<unixtime>.root` + empty tree.
  bool open_run(const std::string& dir, const std::string& tree_name) {
    out_dir_ = dir;
    entries_ = 0;
    provisional_ = dir + "/run_inprogress_" + std::to_string((long long)::time(nullptr)) + ".root";
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", kDelilaCompression);
    if (!file_ || file_->IsZombie()) {
      std::fprintf(stderr, "root_sink: ERROR cannot create %s\n", provisional_.c_str());
      delete file_;
      file_ = nullptr;
      return false;
    }
    tree_ = new TTree(tree_name.c_str(), "DELILA scalar events");
    tree_->Branch("module", &module_, "module/b");
    tree_->Branch("channel", &channel_, "channel/b");
    tree_->Branch("energy", &energy_, "energy/s");
    tree_->Branch("energy_short", &energy_short_, "energy_short/s");
    tree_->Branch("timestamp_ns", &timestamp_ns_, "timestamp_ns/D");
    return true;
  }

  void fill(const ScalarHit& h) {
    if (!tree_) return;
    module_ = h.module;
    channel_ = h.channel;
    energy_ = h.energy;
    energy_short_ = h.energy_short;
    timestamp_ns_ = h.timestamp_ns;
    tree_->Fill();
    ++entries_;
  }

  // Flush the current tree so the in-progress file is openable in ROOT while the
  // run is still going (SaveSelf writes tree + keys without closing).
  void autosave() {
    if (tree_) tree_->AutoSave("SaveSelf");
  }

  // Close and rename to run%04d_scalar.root (collision -> _1, _2, ...).
  // Returns the final path (or the provisional path if the rename failed).
  std::string finalize(uint32_t run) {
    if (!file_) return "";
    file_->cd();
    tree_->Write();
    file_->Close();
    delete file_;
    file_ = nullptr;
    tree_ = nullptr;
    std::string final = pick_final_name(run);
    if (std::rename(provisional_.c_str(), final.c_str()) != 0) {
      std::fprintf(stderr, "root_sink: WARNING could not rename %s -> %s (%s); file kept\n",
                   provisional_.c_str(), final.c_str(), std::strerror(errno));
      return provisional_;
    }
    return final;
  }

  // Shutdown mid-run: write + close but keep the provisional name (the run was
  // never finalized, so it must not masquerade as a complete run%04d file).
  void close_unfinalized() {
    if (!file_) return;
    file_->cd();
    tree_->Write();
    file_->Close();
    delete file_;
    file_ = nullptr;
    tree_ = nullptr;
    std::printf("root_sink: shutdown mid-run — kept provisional %s (%lld events, NOT finalized)\n",
                provisional_.c_str(), entries_);
  }

 private:
  std::string pick_final_name(uint32_t run) const {
    char base[64];
    std::snprintf(base, sizeof(base), "run%04u_scalar", run);
    std::string cand = out_dir_ + "/" + base + ".root";
    if (!path_exists(cand)) return cand;
    for (int i = 1;; ++i) {
      std::string c = out_dir_ + "/" + base + "_" + std::to_string(i) + ".root";
      if (!path_exists(c)) return c;
    }
  }

  TFile* file_ = nullptr;
  TTree* tree_ = nullptr;
  std::string out_dir_;
  std::string provisional_;
  long long entries_ = 0;
  // Branch buffers (types match the leaflist: /b UChar_t, /s UShort_t, /D Double_t).
  UChar_t module_ = 0, channel_ = 0;
  UShort_t energy_ = 0, energy_short_ = 0;
  Double_t timestamp_ns_ = 0.0;
};

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
int main(int argc, char** argv) {
  Options opt;
  bool help = false;
  if (!parse_args(argc, argv, opt, help)) return help ? 0 : 2;

  const bool http_enabled = opt.http_port != 0;
  const bool coinc_enabled =
      opt.gamma_ch >= 0 && opt.thgem1_ch >= 0 && opt.thgem2_ch >= 0;

  std::printf("root_sink: sub=%s out-dir=%s tree=%s\n", opt.zmq.c_str(),
              opt.out_dir.c_str(), opt.tree.c_str());
  if (coinc_enabled)
    std::printf("root_sink: monitor gamma=ch%d thgem1=ch%d thgem2=ch%d window=%.0fns margin=%.0fns\n",
                opt.gamma_ch, opt.thgem1_ch, opt.thgem2_ch, opt.window_ns, opt.margin_ns);
  else
    std::printf("root_sink: Δt monitor DISABLED (need --gamma-ch/--thgem1-ch/--thgem2-ch) — recorder only\n");

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  // --- ROOT monitor objects (created before any TFile; detached from gDirectory
  //     so they persist across runs and are never owned/deleted by a run file) ---
  THttpServer* server = nullptr;
  if (http_enabled) {
    TH1::AddDirectory(kFALSE);
    g_h_dt1 = new TH1D("dt1", "t(ThGEM1) - t(gamma) [ns];#Deltat_{1} [ns];counts",
                       opt.dt_bins, opt.dt_min, opt.dt_max);
    g_h_dt2 = new TH1D("dt2", "t(ThGEM2) - t(gamma) [ns];#Deltat_{2} [ns];counts",
                       opt.dt_bins, opt.dt_min, opt.dt_max);
    // 2D reuses the Δt axes but caps each axis at 500 bins: dt-bins defaults to
    // 2000, and a 2000x2000 (=4M-bin) 2D would be needlessly heavy to draw live.
    int b2 = std::min(opt.dt_bins, 500);
    g_h_dt2_vs_dt1 = new TH2D("dt2_vs_dt1", "#Deltat_{2} vs #Deltat_{1};#Deltat_{1} [ns];#Deltat_{2} [ns]",
                              b2, opt.dt_min, opt.dt_max, b2, opt.dt_min, opt.dt_max);
    g_h_channels = new TH1I("channels", "channel occupancy;channel;counts", 64, 0, 64);

    // A TApplication must exist for gSystem->ProcessEvents() to drive the HTTP
    // server's timer. It is process-lifetime (never deleted) — don't store it in
    // an unused local. Pass an empty argv so it doesn't try to parse our flags.
    static int fake_argc = 1;
    static char a0[] = "root_sink";
    static char* fake_argv[] = {a0, nullptr};
    new TApplication("root_sink", &fake_argc, fake_argv);

    server = new THttpServer(Form("http:%d", opt.http_port));
    server->Register("/", g_h_dt1);
    server->Register("/", g_h_dt2);
    server->Register("/", g_h_dt2_vs_dt1);
    server->Register("/", g_h_channels);
    // /Reset zeroes all four histograms. The command runs through Cling on the
    // main thread; we bake the (dictionaried) TH1 pointers into an interpreter
    // function so no compiled-symbol export (-rdynamic) is needed.
    gInterpreter->Declare(
        Form("void rootsink_reset(){((TH1*)%p)->Reset();((TH1*)%p)->Reset();"
             "((TH1*)%p)->Reset();((TH1*)%p)->Reset();}",
             (void*)g_h_dt1, (void*)g_h_dt2, (void*)g_h_dt2_vs_dt1, (void*)g_h_channels));
    server->RegisterCommand("/Reset", "rootsink_reset()", "button;Reset histograms");
    std::printf("root_sink: THttpServer on http://<host>:%d/  (open in a browser; JSROOT)\n",
                opt.http_port);
  }

  std::unique_ptr<CoincidenceMatcher> matcher;
  if (http_enabled && coinc_enabled) {
    CoincidenceMatcher::Config cfg;
    cfg.gamma_ch = opt.gamma_ch;
    cfg.thgem1_ch = opt.thgem1_ch;
    cfg.thgem2_ch = opt.thgem2_ch;
    cfg.window_ns = opt.window_ns;
    cfg.margin_ns = opt.margin_ns;
    matcher.reset(new CoincidenceMatcher(cfg));
  }

  // --- ZMQ SUB (HWM=0 per the data-preservation rule) ---
  void* ctx = zmq_ctx_new();
  void* sub = zmq_socket(ctx, ZMQ_SUB);
  int zero = 0;
  zmq_setsockopt(sub, ZMQ_RCVHWM, &zero, sizeof(zero));  // unlimited buffer
  zmq_setsockopt(sub, ZMQ_SUBSCRIBE, "", 0);             // all messages
  if (zmq_connect(sub, opt.zmq.c_str()) != 0) {
    std::fprintf(stderr, "root_sink: zmq_connect(%s) failed: %s\n", opt.zmq.c_str(),
                 zmq_strerror(zmq_errno()));
    return 3;
  }

  Recorder recorder;
  RunState run_state;
  std::set<std::string> warned;
  long long events_written = 0;
  long long matcher_fills = 0;

  // Emit any coincidence results a matcher produced into the histograms.
  auto emit = [&](const CoincResult& r) {
    if (r.has_dt1) g_h_dt1->Fill(r.dt1);
    if (r.has_dt2) g_h_dt2->Fill(r.dt2);
    if (r.has_dt1 && r.has_dt2) g_h_dt2_vs_dt1->Fill(r.dt1, r.dt2);
    ++matcher_fills;
  };

  auto process = [&](const uint8_t* data, size_t size) {
    Envelope env = parse_envelope(data, size);
    switch (env.kind) {
      case MsgKind::Data: {
        long n = decode_batch(
            env.payload, env.payload_size,
            [&](uint32_t sid) {
              run_state.on_data(sid, [&] {
                if (!recorder.open_run(opt.out_dir, opt.tree)) {
                  std::fprintf(stderr, "root_sink: run start FAILED — dropping this run's events\n");
                }
                if (matcher) matcher->reset();  // clock may restart between runs
                std::printf("root_sink: run started (source %u) -> %s\n", sid,
                            recorder.provisional().c_str());
              });
            },
            [&](const ScalarHit& h) {
              recorder.fill(h);
              ++events_written;
              if (http_enabled) {
                if (h.channel < 64) g_h_channels->Fill(h.channel);
                if (matcher) matcher->push(h, emit);
              }
            });
        if (n < 0)
          std::fprintf(stderr, "root_sink: WARNING malformed Data batch (skipped)\n");
        break;
      }
      case MsgKind::EndOfStream: {
        run_state.on_eos(
            env.source_id, env.run_number,
            [&](uint32_t rn) {
              if (matcher) matcher->flush(emit);  // ripen the final partial window
              long long ev = recorder.entries();
              std::string fn = recorder.finalize(rn);
              std::printf("root_sink: run %u finalized -> %s (%lld events)\n", rn,
                          fn.c_str(), ev);
              if (http_enabled)
                std::printf("root_sink: (monitor histograms kept across the run boundary)\n");
            },
            [&](uint32_t sid, uint32_t rn) {
              std::printf("root_sink: ignoring stale EOS (source %u, run %u) while idle\n",
                          sid, rn);
            });
        break;
      }
      case MsgKind::Heartbeat:
        break;  // liveness only — nothing to record
      case MsgKind::Unknown:
        if (warned.insert(env.variant).second)
          std::fprintf(stderr, "root_sink: WARNING unknown message variant '%s' (skipping; further ones silenced)\n",
                       env.variant.c_str());
        break;
    }
  };

  std::printf("root_sink: running. Ctrl-C or SIGTERM to stop.\n");
  std::fflush(stdout);

  auto t_last_status = Clock::now();
  auto t_last_autosave = Clock::now();
  long long last_events = 0;

  while (!g_stop) {
    zmq_pollitem_t items[1];
    items[0].socket = sub;
    items[0].fd = 0;
    items[0].events = ZMQ_POLLIN;
    items[0].revents = 0;
    int rc = zmq_poll(items, 1, 100);  // 100 ms
    if (rc < 0) {
      if (zmq_errno() == EINTR) continue;  // interrupted by our signal
      std::fprintf(stderr, "root_sink: zmq_poll error: %s\n", zmq_strerror(zmq_errno()));
      break;
    }
    if (items[0].revents & ZMQ_POLLIN) {
      // Drain everything pending this wakeup (keep up with the merger).
      while (true) {
        zmq_msg_t msg;
        zmq_msg_init(&msg);
        int nb = zmq_msg_recv(&msg, sub, ZMQ_DONTWAIT);
        if (nb < 0) {
          zmq_msg_close(&msg);
          break;  // EAGAIN (drained) or error
        }
        process(static_cast<const uint8_t*>(zmq_msg_data(&msg)), zmq_msg_size(&msg));
        zmq_msg_close(&msg);
      }
    }

    // Service HTTP requests on the main thread (no lock needed vs Fill above).
    if (server) gSystem->ProcessEvents();

    auto now = Clock::now();
    using std::chrono::duration;
    using std::chrono::duration_cast;
    using std::chrono::seconds;
    if (recorder.is_open() &&
        duration_cast<seconds>(now - t_last_autosave).count() >= opt.autosave_sec) {
      recorder.autosave();
      t_last_autosave = now;
    }
    if (duration_cast<seconds>(now - t_last_status).count() >= 10) {
      double dt = duration_cast<duration<double>>(now - t_last_status).count();
      double rate = dt > 0 ? (double)(events_written - last_events) / dt : 0.0;
      std::printf("root_sink: %s | events=%lld | %.0f ev/s | matcher_fills=%lld\n",
                  run_state.is_writing() ? "WRITING" : "idle", events_written, rate,
                  matcher_fills);
      std::fflush(stdout);
      t_last_status = now;
      last_events = events_written;
    }
  }

  std::printf("root_sink: stopping...\n");
  if (recorder.is_open()) recorder.close_unfinalized();

  zmq_close(sub);
  zmq_ctx_term(ctx);
  // ROOT objects (histograms, server, app) are process-lifetime; let the OS
  // reclaim them on exit rather than risk teardown-order surprises.
  std::printf("root_sink: bye (%lld events written total).\n", events_written);
  return 0;
}
