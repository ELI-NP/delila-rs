// nwall_sink.cxx — the N_Wall experiment's ROOT sink + live monitor (TODO 79).
//
// One process subscribing to the DELILA merger's ZMQ PUB as an ADDITIONAL
// consumer (the Rust Recorder that writes the `.delila` stays the
// authoritative recorder — this tool never gates or throttles the pipeline).
//
// Per-experiment program (TODO 79 §5.1): the receive / sort / event-build /
// record pipeline is a copy of the standard root_sink (../standard/), the
// display is N_Wall's own. Once both have run, the shared part becomes a
// library (../common/ already holds the headers).
//
//   Recorder: the same trees as root_sink, each opt-in — hits (--hits-tree)
//     and the generic events tree (--event-def; for N_Wall the E-sector
//     trigger of config/event_def_nwall.json). Same write-failure handling,
//     time offsets (--time-calib) and per-run provenance.
//
//   Display (THttpServer / JSROOT): fed from the WRITER — time-sorted core
//     hits plus the built events of every chunk — through a bounded queue
//     (display only; drops are counted, the record path never drops). It
//     replaces root_sink's arrival-order tee, so there is still exactly one
//     display drop point. The detector role of every channel comes from the
//     channel map (--channel-map, nwall_map.hpp); bunch times from the
//     downscaled beam channel (bunch_clock.hpp):
//       /hits    hitmap (module x channel), energy_m<M> (energy x channel)
//       /rf      tof_rf_m<M>: t - (bunch at or before t) - rf_t0, per channel
//       /ls      ls_psd: (E - Es)/E vs E of the LS channels
//       /events  pid (dE vs E), sector_dE_vs_E, dE_ring_vs_sector,
//                ls_dt_si (t_LS - t_trigger), si_phase (Si trigger vs bunch),
//                ls_tof_si (t_LS - the bunch the Si picked - rf_t0)
//
// Build (see README.md):
//   g++ -O2 -std=c++17 nwall_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o nwall_sink
//
// License: BSD-3-Clause (same as delila-rs).

#include <zmq.h>

#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/time.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <cmath>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <map>
#include <memory>
#include <set>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include "../common/eb_core.hpp"
#include "../common/event_def.hpp"
#include "../common/sink_core.hpp"
#include "../common/time_offsets.hpp"
#include "bunch_clock.hpp"
#include "nwall_map.hpp"

#include "TApplication.h"
#include "TBranch.h"
#include "TFile.h"
#include "TH1.h"
#include "TH2.h"
#include "THttpServer.h"
#include "TList.h"
#include "TNamed.h"
#include "TROOT.h"
#include "TRootSniffer.h"
#include "TString.h"
#include "TSystem.h"
#include "TTree.h"

using namespace rootsink;
using Clock = std::chrono::steady_clock;

// ROOT compression = algorithm*100 + level. 505 = ZSTD level 5 (as delila2root).
static const int kDelilaCompression = 505;

// SIGINT + SIGTERM flip a flag; the main loop drains and closes the file.
static volatile sig_atomic_t g_stop = 0;
static void on_signal(int) { g_stop = 1; }

// /Reset flips this flag (a compiled function pointer baked into the command);
// the main loop does the work after ProcessEvents.
static volatile sig_atomic_t g_reset = 0;
static void request_reset() { g_reset = 1; }

// ---------------------------------------------------------------------------
// CLI options
// ---------------------------------------------------------------------------
struct Options {
  std::string zmq = "tcp://localhost:5557";
  std::string out_dir = ".";
  std::string tree = "delila";
  bool tree_given = false;
  bool hits_tree = false;
  std::string exp_name;
  std::string operator_url;
  std::vector<int> modules;  // --module (empty = all boards; N_Wall boards are synchronized)
  int http_port = 8090;
  int autosave_sec = 30;
  int workers = 3;
  double chunk_span_ms = 100.0;
  double safe_horizon_ms = 1000.0;  // DIG1 aggregates arrive up to ~100 ms late (pf-qaq run 52)
  std::string time_calib;
  std::string event_def;
  // N_Wall display
  std::string channel_map;       // --channel-map: roles of the channels (nwall_map.hpp)
  double rf_period_ns = 199.980;  // nominal RF period (the bunch clock measures it per run)
  double rf_t0_ns = 0.0;          // constant shift of every RF TOF (beam signal vs target)
  double si_delay_ns = 0.0;       // Si trigger time minus its bunch time (from si_phase)
};

static const char* const kEventsTreeName = "events";

static void print_usage(const char* argv0) {
  std::printf(
      "usage: %s [options]\n"
      "  --zmq ADDR          merger PUB endpoint (default tcp://localhost:5557)\n"
      "  --out-dir DIR       output directory for run*.root (default .)\n"
      "  --hits-tree         write the per-hit tree (opt-in)\n"
      "  --tree NAME         hits tree name (default delila; needs --hits-tree)\n"
      "  --event-def FILE    generic event definition JSON -> the \"events\" tree\n"
      "                      (N_Wall: config/event_def_nwall.json). Re-read at every\n"
      "                      run start. At least one of --hits-tree / --event-def\n"
      "  --exp-name NAME     experiment name for the output filename (override)\n"
      "  --operator URL      operator base URL; experiment name from /api/status\n"
      "  --module M[,M2...]  process only these digitizers (default: all)\n"
      "  --time-calib FILE   per-(module,channel) time offsets (timeSettings.json),\n"
      "                      aligned = raw - offset_ns; re-read at every run start\n"
      "  --http-port N       THttpServer port, 0 disables the display (default 8090)\n"
      "  --channel-map FILE  channel roles (config/channel_map_N_Wall.txt, columns\n"
      "                      5-7 'det kind index'); re-read at every run start.\n"
      "                      Without it only the per-board histograms are filled\n"
      "  --rf-period-ns X    nominal RF period (default 199.980; measured per run\n"
      "                      from the beam channel)\n"
      "  --rf-t0-ns X        shift of every RF TOF (default 0): set it so the\n"
      "                      gamma flash sits at L/c\n"
      "  --si-delay-ns X     Si trigger time minus its bunch time (default 0): the\n"
      "                      peak of /events/si_phase; picks the bunch of an event\n"
      "  --autosave-sec N    TTree AutoSave interval (default 30)\n"
      "  --workers N         event-build worker threads (default 3)\n"
      "  --chunk-span-ms X   sorter chunk span in data time (default 100)\n"
      "  --safe-horizon-ms X sorter disorder absorption (default 1000)\n"
      "  --help              this message\n",
      argv0);
}

static bool parse_args(int argc, char** argv, Options& opt, bool& help) {
  help = false;
  auto need = [&](int& i) -> const char* {
    if (i + 1 >= argc) {
      std::fprintf(stderr, "nwall_sink: missing value for %s\n", argv[i]);
      return nullptr;
    }
    return argv[++i];
  };
  // A number flag must be a number: std::atof would turn junk into 0 silently.
  auto number = [&](const char* flag, const char* v, double& out) {
    char* end = nullptr;
    out = std::strtod(v, &end);
    if (end == v || *end != '\0') {
      std::fprintf(stderr, "nwall_sink: %s wants a number, got '%s'\n", flag, v);
      return false;
    }
    return true;
  };
  for (int i = 1; i < argc; ++i) {
    std::string a = argv[i];
    const char* v = nullptr;
    double x = 0.0;
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
      opt.tree_given = true;
    } else if (a == "--hits-tree") {
      opt.hits_tree = true;
    } else if (a == "--exp-name") {
      if (!(v = need(i))) return false;
      opt.exp_name = v;
    } else if (a == "--operator") {
      if (!(v = need(i))) return false;
      opt.operator_url = v;
    } else if (a == "--module") {
      if (!(v = need(i))) return false;
      std::string why;
      if (!parse_module_list(v, opt.modules, &why)) {
        std::fprintf(stderr, "nwall_sink: --module: %s\n", why.c_str());
        return false;
      }
    } else if (a == "--http-port") {
      if (!(v = need(i)) || !number(a.c_str(), v, x)) return false;
      opt.http_port = static_cast<int>(x);
    } else if (a == "--autosave-sec") {
      if (!(v = need(i)) || !number(a.c_str(), v, x)) return false;
      opt.autosave_sec = static_cast<int>(x);
    } else if (a == "--workers") {
      if (!(v = need(i)) || !number(a.c_str(), v, x)) return false;
      opt.workers = std::max(1, static_cast<int>(x));
    } else if (a == "--chunk-span-ms") {
      if (!(v = need(i)) || !number(a.c_str(), v, opt.chunk_span_ms)) return false;
    } else if (a == "--safe-horizon-ms") {
      if (!(v = need(i)) || !number(a.c_str(), v, opt.safe_horizon_ms)) return false;
    } else if (a == "--time-calib") {
      if (!(v = need(i))) return false;
      opt.time_calib = v;
    } else if (a == "--event-def") {
      if (!(v = need(i))) return false;
      opt.event_def = v;
    } else if (a == "--channel-map") {
      if (!(v = need(i))) return false;
      opt.channel_map = v;
    } else if (a == "--rf-period-ns") {
      if (!(v = need(i)) || !number(a.c_str(), v, opt.rf_period_ns)) return false;
      if (opt.rf_period_ns <= 0.0) {
        std::fprintf(stderr, "nwall_sink: --rf-period-ns must be > 0\n");
        return false;
      }
    } else if (a == "--rf-t0-ns") {
      if (!(v = need(i)) || !number(a.c_str(), v, opt.rf_t0_ns)) return false;
    } else if (a == "--si-delay-ns") {
      if (!(v = need(i)) || !number(a.c_str(), v, opt.si_delay_ns)) return false;
    } else {
      std::fprintf(stderr, "nwall_sink: unknown argument '%s' (try --help)\n", a.c_str());
      return false;
    }
  }
  return true;
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.0 GET (experiment-name lookup via --operator)
// ---------------------------------------------------------------------------
//
// A dependency-free, TIME-BOUNDED GET. It fetches `<base>/api/status` from the
// operator so the output filename can mirror the Rust Recorder's exp_name. It
// must NEVER hang the sink: connect is non-blocking with a select() deadline and
// the socket carries SO_RCVTIMEO/SO_SNDTIMEO, all inside a ~2 s total budget. A
// stall here is acceptable — ZMQ (HWM=0) buffers batches while we wait. Only
// http:// is supported; https:// is rejected at startup with a clear error.

// Parse `http://host[:port][/base/path]`. Fills host/port and the request path
// (base path with any trailing slash trimmed, then "/api/status" appended).
// Returns false with `err` set for a non-http scheme or a missing host.
static bool parse_operator_url(const std::string& url, std::string& host,
                              std::string& port, std::string& request_path,
                              std::string& err) {
  const std::string http = "http://";
  const std::string https = "https://";
  if (url.rfind(https, 0) == 0) {
    err = "https:// is not supported (use a plain http:// operator URL)";
    return false;
  }
  if (url.rfind(http, 0) != 0) {
    err = "operator URL must start with http://";
    return false;
  }
  std::string rest = url.substr(http.size());
  std::string authority = rest;
  std::string base_path;
  std::size_t slash = rest.find('/');
  if (slash != std::string::npos) {
    authority = rest.substr(0, slash);
    base_path = rest.substr(slash);  // includes the leading '/'
  }
  if (authority.empty()) {
    err = "operator URL has no host";
    return false;
  }
  std::size_t colon = authority.find(':');
  if (colon != std::string::npos) {
    host = authority.substr(0, colon);
    port = authority.substr(colon + 1);
  } else {
    host = authority;
    port = "80";
  }
  if (host.empty()) {
    err = "operator URL has no host";
    return false;
  }
  // Trim a trailing '/' from the base path so we don't produce "//api/status".
  while (!base_path.empty() && base_path.back() == '/') base_path.pop_back();
  request_path = base_path + "/api/status";
  return true;
}

// GET the operator status and return the raw response text in `out`. Returns
// false with `err` set on any failure (unresolvable host, connect timeout, send
// or recv error). Bounded to `budget_ms` total wall time.
static bool http_get_status(const std::string& url, int budget_ms, std::string& out,
                            std::string& err) {
  std::string host, port, path;
  if (!parse_operator_url(url, host, port, path, err)) return false;

  using Clock = std::chrono::steady_clock;
  auto deadline = Clock::now() + std::chrono::milliseconds(budget_ms);
  auto remaining_ms = [&]() -> long {
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(deadline - Clock::now()).count();
    return ms < 0 ? 0 : static_cast<long>(ms);
  };

  struct addrinfo hints;
  std::memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  struct addrinfo* ai = nullptr;
  int gr = ::getaddrinfo(host.c_str(), port.c_str(), &hints, &ai);
  if (gr != 0 || !ai) {
    err = "getaddrinfo(" + host + ":" + port + ") failed: " + gai_strerror(gr);
    return false;
  }

  int fd = -1;
  bool connected = false;
  for (struct addrinfo* p = ai; p && !connected; p = p->ai_next) {
    fd = ::socket(p->ai_family, p->ai_socktype, p->ai_protocol);
    if (fd < 0) continue;
    // Non-blocking connect so a dead host cannot block past our deadline.
    int flags = ::fcntl(fd, F_GETFL, 0);
    ::fcntl(fd, F_SETFL, flags | O_NONBLOCK);
    int rc = ::connect(fd, p->ai_addr, p->ai_addrlen);
    if (rc == 0) {
      connected = true;
    } else if (errno == EINPROGRESS) {
      fd_set wset;
      FD_ZERO(&wset);
      FD_SET(fd, &wset);
      struct timeval tv;
      long ms = remaining_ms();
      tv.tv_sec = ms / 1000;
      tv.tv_usec = (ms % 1000) * 1000;
      int sr = ::select(fd + 1, nullptr, &wset, nullptr, &tv);
      if (sr > 0) {
        int soerr = 0;
        socklen_t len = sizeof(soerr);
        if (::getsockopt(fd, SOL_SOCKET, SO_ERROR, &soerr, &len) == 0 && soerr == 0)
          connected = true;
      }
    }
    if (!connected) {
      ::close(fd);
      fd = -1;
    }
  }
  ::freeaddrinfo(ai);
  if (!connected) {
    err = "connect to " + host + ":" + port + " timed out or failed";
    if (fd >= 0) ::close(fd);
    return false;
  }

  // Back to blocking + apply per-op timeouts for the remainder of the budget.
  int flags = ::fcntl(fd, F_GETFL, 0);
  ::fcntl(fd, F_SETFL, flags & ~O_NONBLOCK);
  auto set_timeout = [&](int opt) {
    long ms = remaining_ms();
    if (ms <= 0) ms = 1;
    struct timeval tv;
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    ::setsockopt(fd, SOL_SOCKET, opt, &tv, sizeof(tv));
  };
  set_timeout(SO_SNDTIMEO);
  set_timeout(SO_RCVTIMEO);

  std::string req = "GET " + path + " HTTP/1.0\r\nHost: " + host +
                    "\r\nConnection: close\r\n\r\n";
  std::size_t sent = 0;
  while (sent < req.size()) {
    ssize_t w = ::send(fd, req.data() + sent, req.size() - sent, 0);
    if (w <= 0) {
      err = "send to operator failed";
      ::close(fd);
      return false;
    }
    sent += static_cast<std::size_t>(w);
    if (remaining_ms() == 0) break;
  }

  out.clear();
  char buf[4096];
  while (remaining_ms() > 0) {
    ssize_t n = ::recv(fd, buf, sizeof(buf), 0);
    if (n > 0) {
      out.append(buf, static_cast<std::size_t>(n));
    } else if (n == 0) {
      break;  // peer closed (Connection: close) — full response received
    } else {
      break;  // timeout / error — return whatever we have (may be empty)
    }
  }
  ::close(fd);
  if (out.empty()) {
    err = "empty response from operator";
    return false;
  }
  return true;
}

// Resolve the experiment name for the current run, honouring the priority order
// --exp-name > --operator (/api/status) > "data". Always logs the resolved name
// and its source; warns visibly on the "data" fallback (never silent).
static std::string resolve_exp_name(const Options& opt) {
  if (!opt.exp_name.empty()) {
    std::printf("nwall_sink: experiment_name = \"%s\" (source: --exp-name)\n",
                opt.exp_name.c_str());
    return opt.exp_name;
  }
  if (!opt.operator_url.empty()) {
    std::string raw, err;
    if (http_get_status(opt.operator_url, 2000, raw, err)) {
      std::string hdr, body;
      split_http_response(raw, hdr, body);
      std::string name = extract_experiment_name(body);
      if (!name.empty()) {
        std::printf("nwall_sink: experiment_name = \"%s\" (source: %s/api/status)\n",
                    name.c_str(), opt.operator_url.c_str());
        return name;
      }
      std::fprintf(stderr,
                   "nwall_sink: WARNING operator /api/status had no experiment_name; "
                   "using \"data\"\n");
      return "data";
    }
    std::fprintf(stderr,
                 "nwall_sink: WARNING could not fetch experiment_name from %s (%s); "
                 "using \"data\"\n",
                 opt.operator_url.c_str(), err.c_str());
    return "data";
  }
  std::fprintf(stderr,
               "nwall_sink: WARNING no --exp-name/--operator given; output uses "
               "experiment_name \"data\"\n");
  return "data";
}

// ---------------------------------------------------------------------------
// ROOT scalar recorder
// ---------------------------------------------------------------------------
static bool path_exists(const std::string& p) {
  struct stat st;
  return ::stat(p.c_str(), &st) == 0;
}

// Nanoseconds since the Unix epoch — the collision suffix, matching the Rust
// Recorder's `_{unix_ns}` scheme so a clash appends the same kind of tag.
static long long unix_nanos() {
  return std::chrono::duration_cast<std::chrono::nanoseconds>(
             std::chrono::system_clock::now().time_since_epoch())
      .count();
}

// "0.0 MB free in /data" — for the write-failure ERROR (disk full vs I/O error).
static std::string free_space_str(const std::string& dir) {
  struct statvfs sv;
  if (::statvfs(dir.c_str(), &sv) != 0) return "free space in " + dir + " unknown";
  char buf[64];
  std::snprintf(buf, sizeof buf, "%.1f MB free in ",
                static_cast<double>(sv.f_bavail) * static_cast<double>(sv.f_frsize) / 1e6);
  return buf + dir;
}

// Write failures (TODO 69 Phase 3). ROOT never throws on an I/O error: TTree::Fill
// returns -1 when a basket write fails, and TFile sets kWriteError at the first
// failed write — also for a SHORT write (the usual first symptom of a full disk:
// errno stays 0 and the file still claims to be writable). On the first failure
// of a run the Recorder prints one ERROR, latches write_failed_ and stops
// writing the run file for the rest of the run: every further Fill would fail
// again (3 ROOT error lines per basket) and only make the file harder to
// recover. Hits/events offered after that are counted as unwritten; the stream,
// the event build and the monitor carry on. The file is still closed (no
// TTree::Write) and finalized under its normal name; opening it recovers the
// trees as of the last AutoSave before the failure (TFile::Recover). There is
// no Operator stop from here: the .delila is the authoritative record, and the
// Operator stops the run on the Rust Recorder's own write failure (Phase 1).
class Recorder {
 public:
  bool is_open() const { return file_ != nullptr; }
  // This run's write-failure latch and what was not written after it — kept
  // after finalize for the run summary, reset by the next open_run.
  bool write_failed() const { return write_failed_; }
  long long unwritten_hits() const { return unwritten_hits_; }
  long long unwritten_events() const { return unwritten_events_; }
  const std::string& provisional() const { return provisional_; }
  long long entries() const { return entries_; }
  // The path chosen by the last finalize() (for pairing the hists-config copy).
  const std::string& final_path() const { return final_path_; }
  // What open_run recorded about this run (time offsets, event definition) —
  // for the sidecars.
  const eb::RunProvenance& provenance() const { return prov_; }

  // Open a provisional file `<dir>/run_inprogress_<unixtime>.root` + empty trees.
  // `exp_name` is remembered for the final filename (resolved at run start so it
  // matches the Rust Recorder even though EOS/finalize come later). Every tree
  // is optional and they all live in the SAME file: a non-empty
  // `hits_tree_name` adds the per-hit tree, a non-null `event_def` the generic
  // "events" tree — whose L2 branches are named by that (run-frozen) definition.
  // `prov` (frozen at RunOpen by the Receiver) goes into the trees' UserInfo
  // so every file says which time offsets — if any — its times carry, and
  // which event definition built its events.
  bool open_run(const std::string& dir, const std::string& hits_tree_name,
                const std::string& exp_name, std::shared_ptr<const EventDef> event_def,
                const eb::RunProvenance& prov) {
    out_dir_ = dir;
    prov_ = prov;
    event_def_ = std::move(event_def);
    l2_size_warned_ = false;
    exp_name_ = exp_name.empty() ? "data" : exp_name;  // mirror the Rust fallback
    entries_ = 0;
    events_entries_ = 0;
    final_path_.clear();
    hits_on_ = !hits_tree_name.empty();
    events_on_ = event_def_ != nullptr;
    write_failed_ = false;
    fail_op_.clear();
    unwritten_hits_ = 0;
    unwritten_events_ = 0;
    provisional_ = dir + "/run_inprogress_" + std::to_string((long long)::time(nullptr)) + ".root";
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", kDelilaCompression);
    if (!file_ || file_->IsZombie()) {
      delete file_;
      file_ = nullptr;
      // No file at all: the whole run is unwritten (fill/fill_event count it).
      mark_failed("TFile::Open", provisional_);
      return false;
    }
    const bool aligned = !prov_.time_offsets_source.empty();
    tree_ = nullptr;
    if (!hits_tree_name.empty()) {
      tree_ = new TTree(hits_tree_name.c_str(),
                        aligned ? "DELILA scalar events (timestamp_ns ALIGNED: raw - "
                                  "timeSettings offset; see UserInfo)"
                                : "DELILA scalar events");
      tree_->Branch("module", &module_, "module/b");
      tree_->Branch("channel", &channel_, "channel/b");
      tree_->Branch("energy", &energy_, "energy/s");
      tree_->Branch("energy_short", &energy_short_, "energy_short/s");
      tree_->Branch("timestamp_ns", &timestamp_ns_, "timestamp_ns/D");
    }
    events_ = nullptr;
    if (event_def_) {
      // One entry per generic event (TODO 71 §4): dictionary-free leaflists,
      // so a bare TTree::Draw / any ROOT version reads it without a library.
      // The per-hit columns are variable-length arrays counted by NHits; their
      // buffers are re-pointed whenever they grow (ensure_hit_capacity).
      events_ = new TTree(kEventsTreeName,
                          "DELILA generic events (TODO 71; aligned time axis, see "
                          "UserInfo eventDef)");
      events_->Branch("EventID", &ev_id_, "EventID/l");
      events_->Branch("TriggerTime", &ev_t_, "TriggerTime/D");
      events_->Branch("TriggerMod", &ev_mod_, "TriggerMod/b");
      events_->Branch("TriggerCh", &ev_ch_, "TriggerCh/b");
      events_->Branch("TriggerIdx", &ev_idx_, "TriggerIdx/b");
      events_->Branch("NTriggers", &ev_ntrig_, "NTriggers/i");
      events_->Branch("NHits", &ev_nhits_, "NHits/i");
      hit_cap_ = 0;
      alloc_hit_buffers(kInitialHitCapacity);
      br_hit_mod_ = events_->Branch("HitMod", hit_mod_.data(), "HitMod[NHits]/b");
      br_hit_ch_ = events_->Branch("HitCh", hit_ch_.data(), "HitCh[NHits]/b");
      br_hit_e_ = events_->Branch("HitEnergy", hit_e_.data(), "HitEnergy[NHits]/s");
      br_hit_es_ = events_->Branch("HitEnergyShort", hit_es_.data(), "HitEnergyShort[NHits]/s");
      br_hit_t_ = events_->Branch("HitTime", hit_t_.data(), "HitTime[NHits]/D");
      br_hit_trig_ = events_->Branch("HitIsTrigger", hit_trig_.get(), "HitIsTrigger[NHits]/O");
      // ALWAYS present (all false without anti_compton): the tree's shape must
      // not depend on the definition, so TChains and macros keep working.
      br_hit_ac_ = events_->Branch("HitWithAC", hit_ac_.get(), "HitWithAC[NHits]/O");
      check_fixed_branch_names();
      add_l2_branches(*event_def_);
    }
    // UserInfo is part of the TTree object: written by every AutoSave and into
    // every MaxTreeSize part (a file-level key would land in the last part only).
    if (!prov_.time_offsets_meta.empty()) {
      for (TTree* t : {tree_, events_})
        if (t) t->GetUserInfo()->Add(new TNamed("timeSettings", prov_.time_offsets_meta.c_str()));
    }
    if (events_ && !prov_.event_def_meta.empty())
      events_->GetUserInfo()->Add(new TNamed("eventDef", prov_.event_def_meta.c_str()));
    // A full disk can already fail the header write inside TFile::Open (the
    // file then exists but carries kWriteError): the run starts failed.
    check_file("TFile::Open");
    return true;
  }

  // Returns true when the hit went into the tree. After a write failure (or
  // without a run file) the hit is only counted as unwritten.
  bool fill(const ScalarHit& h) {
    if (!hits_on_) return false;
    if (write_failed_ || !tree_) {
      ++unwritten_hits_;
      return false;
    }
    module_ = h.module;
    channel_ = h.channel;
    energy_ = h.energy;
    energy_short_ = h.energy_short;
    timestamp_ns_ = h.timestamp_ns;
    if (tree_->Fill() < 0) {  // a basket write failed (ROOT printed why just above)
      ++unwritten_hits_;
      mark_failed("TTree::Fill (hits tree)", current_file_name());
      return false;
    }
    ++entries_;
    return true;
  }

  // One generic event -> one entry. EventID is a per-run counter from 0 (it
  // keeps counting across ROOT rollover parts). Never truncates: an event with
  // more hits than the buffers hold grows them first.
  void fill_event(const eb::GenericEvent& e) {
    if (!events_on_) return;
    if (write_failed_ || !events_) {
      ++unwritten_events_;
      return;
    }
    ensure_hit_capacity(e.hits.size());
    ev_id_ = static_cast<ULong64_t>(events_entries_);
    ev_t_ = e.t;
    ev_mod_ = e.mod;
    ev_ch_ = e.ch;
    ev_idx_ = e.trig_idx;
    ev_ntrig_ = e.n_triggers;
    ev_nhits_ = static_cast<UInt_t>(e.hits.size());
    for (std::size_t i = 0; i < e.hits.size(); ++i) {
      const eb::EventHit& h = e.hits[i];
      hit_mod_[i] = h.mod;
      hit_ch_[i] = h.ch;
      hit_e_[i] = h.energy;
      hit_es_[i] = h.energy_short;
      hit_t_[i] = h.dt;
      hit_trig_[i] = h.is_trigger;
      hit_ac_[i] = h.with_ac;
    }
    if (!l2_counts_.empty()) {
      // One value per counter is the core's contract (GenericEvent::l2_counts);
      // a short vector would leave stale values behind — fill 0 and say so.
      if (e.l2_counts.size() != l2_counts_.size() && !l2_size_warned_) {
        std::fprintf(stderr,
                     "nwall_sink: WARNING event %lld carries %zu L2 counter value(s), the run's "
                     "definition has %zu — missing ones are written as 0 (reported once per run)\n",
                     events_entries_, e.l2_counts.size(), l2_counts_.size());
        l2_size_warned_ = true;
      }
      for (std::size_t i = 0; i < l2_counts_.size(); ++i)
        l2_counts_[i] = i < e.l2_counts.size() ? e.l2_counts[i] : 0;
      for (std::size_t i = 0; i < n_l2_flags_; ++i) l2_flags_[i] = (e.l2_flags >> i) & 1u;
      for (std::size_t i = 0; i < n_l2_accepts_; ++i) l2_accepts_[i] = (e.l2_accepts >> i) & 1u;
      l2_accept_ = e.l2_accept;
    }
    if (events_->Fill() < 0) {
      ++unwritten_events_;
      mark_failed("TTree::Fill (events tree)", current_file_name());
      return;
    }
    ++events_entries_;
  }

  // Flush the current tree(s) so the in-progress file is openable in ROOT while
  // the run is still going (SaveSelf writes tree + keys without closing).
  // Never on a failed file: the last good AutoSave is what a recovery reads.
  void autosave() {
    for (TTree* t : {tree_, events_}) {
      if (!t || write_failed_) continue;
      t->AutoSave("SaveSelf");
      check_file("TTree::AutoSave");
    }
  }

  // kWriteError / zombie on the CURRENT file (a MaxTreeSize rollover replaces
  // it). Called once per written chunk as well: it also catches a failure
  // ROOT did not report through Fill's return value (e.g. inside an
  // auto-flush), at the cost of a few pointer loads.
  void check_file(const char* op) {
    if (write_failed_ || !file_) return;
    TFile* cur = current_file();
    if (!cur) {  // a failed rollover leaves the tree without a file
      mark_failed(op, provisional_ + " (ROOT lost the current file)");
      return;
    }
    if (cur->TestBit(TFile::kWriteError) || cur->IsZombie()) mark_failed(op, cur->GetName());
  }

  // Close and rename to run%04u_<seq>_<exp>.root — identical to the Rust
  // Recorder filename but for the extension. Normally one file per run
  // (seq 0000); if the tree crossed TTree::GetMaxTreeSize, ROOT's automatic
  // ChangeFile has split it into <stem>_1.root, <stem>_2.root, ... — those
  // parts are renamed with the Recorder's sequence convention (0001, 0002, ...).
  // A collision appends _<unix_ns>. Returns the seq-0 final path (or the
  // provisional path if that rename failed).
  std::string finalize(uint32_t run) {
    if (!file_) return "";
    int parts = close_all_parts();
    std::string final = pick_final_name(run, 0);
    if (std::rename(provisional_.c_str(), final.c_str()) != 0) {
      std::fprintf(stderr, "nwall_sink: WARNING could not rename %s -> %s (%s); file kept\n",
                   provisional_.c_str(), final.c_str(), std::strerror(errno));
      final = provisional_;
    }
    // Rename any ROOT-made continuation parts (verified naming: <stem>_k.root).
    std::string stem = provisional_.substr(0, provisional_.size() - 5);  // drop ".root"
    for (int k = 1; k <= parts; ++k) {
      std::string part = stem + "_" + std::to_string(k) + ".root";
      std::string part_final = pick_final_name(run, k);
      if (std::rename(part.c_str(), part_final.c_str()) != 0)
        std::fprintf(stderr, "nwall_sink: WARNING could not rename %s -> %s (%s); file kept\n",
                     part.c_str(), part_final.c_str(), std::strerror(errno));
      else
        std::printf("nwall_sink: rollover part %d -> %s\n", k, part_final.c_str());
    }
    final_path_ = final;
    return final;
  }

  // Shutdown mid-run: write + close but keep the provisional name (the run was
  // never finalized, so it must not masquerade as a complete run%04d file).
  void close_unfinalized() {
    if (!file_) return;
    std::string counts = entry_counts();
    int parts = close_all_parts();
    std::printf("nwall_sink: shutdown mid-run — kept provisional %s (%s, NOT finalized)\n",
                provisional_.c_str(), counts.c_str());
    if (write_failed_)
      std::printf("nwall_sink: ... and INCOMPLETE — %s\n", failure_summary().c_str());
    if (parts > 0)
      std::printf("nwall_sink: (plus %d ROOT rollover part(s) kept as %s_*.root)\n", parts,
                  provisional_.substr(0, provisional_.size() - 5).c_str());
  }

  // "WRITE FAILED (TTree::Fill (hits tree)): not written after the failure:
  // 123 hits, 4 events" — the run summary's verdict ("" for a healthy run).
  std::string failure_summary() const {
    if (!write_failed_) return "";
    std::string s = "WRITE FAILED (" + fail_op_ + "): not written after the failure:";
    if (hits_on_) s += " " + std::to_string(unwritten_hits_) + " hits";
    if (hits_on_ && events_on_) s += ",";
    if (events_on_) s += " " + std::to_string(unwritten_events_) + " events";
    return s;
  }

  // "205950 hits, 30120 events" — only the trees that exist.
  std::string entry_counts() const {
    std::string s;
    auto add = [&](bool on, long long n, const char* what) {
      if (!on) return;
      if (!s.empty()) s += ", ";
      s += std::to_string(n) + " " + what;
    };
    add(tree_ != nullptr, entries_, "hits");
    add(events_ != nullptr, events_entries_, "events");
    return s.empty() ? std::string("no run file") : s;  // open_run had failed
  }

 private:
  static constexpr std::size_t kInitialHitCapacity = 64;

  // (Re)allocate the hit-array buffers for `cap` hits. Contents need not
  // survive: fill_event rewrites every element up to NHits before each Fill.
  void alloc_hit_buffers(std::size_t cap) {
    hit_mod_.assign(cap, 0);
    hit_ch_.assign(cap, 0);
    hit_e_.assign(cap, 0);
    hit_es_.assign(cap, 0);
    hit_t_.assign(cap, 0.0);
    // Bool_t[] for the /O leaves, NOT std::vector<bool> (bit-packed: no data()).
    hit_trig_.reset(new Bool_t[cap]());
    hit_ac_.reset(new Bool_t[cap]());
    hit_cap_ = cap;
  }

  // The fixed events-tree branch names live in two places: here (the branches
  // created above, plus L2Accept) and edef_detail::is_fixed_branch, which the
  // parser uses to reject L2 names that would clash. If a fixed branch is added
  // here but not there, a definition could name an L2 branch after it — so
  // check every fixed name against the parser's list, once per process.
  void check_fixed_branch_names() {
    static bool checked = false;
    if (checked) return;
    checked = true;
    std::vector<std::string> names{"L2Accept"};
    for (TObject* b : *events_->GetListOfBranches()) names.push_back(b->GetName());
    for (const std::string& n : names)
      if (!edef_detail::is_fixed_branch(n))
        std::fprintf(stderr,
                     "nwall_sink: ERROR events-tree branch \"%s\" is missing from "
                     "edef_detail::is_fixed_branch (event_def.hpp) — an L2 name could clash "
                     "with it; add it there\n",
                     n.c_str());
  }

  // Phase C L2 (TODO/71_phase_c_proposal.md §2.3): one scalar branch per
  // counter (<name>/i), flag and accept (<name>/O), then L2Accept/O when any
  // accept exists — named by the definition frozen at RunOpen, so
  // `events->Draw("HitEnergy", "GeClean")` works. The buffers are sized here,
  // once per run, and never resized: the branches hold their addresses for
  // the whole run (unlike the hit arrays, these do not depend on the event).
  void add_l2_branches(const EventDef& d) {
    l2_counts_.assign(d.has_l2() ? d.counters.size() : 0, 0);
    n_l2_flags_ = d.has_l2() ? d.flags.size() : 0;
    n_l2_accepts_ = d.has_l2() ? d.accepts.size() : 0;
    l2_flags_.reset(new Bool_t[n_l2_flags_ + 1]());  // +1: never a zero-length new[]
    l2_accepts_.reset(new Bool_t[n_l2_accepts_ + 1]());
    l2_accept_ = false;
    if (!d.has_l2()) return;
    // The parser rejects names that are not identifiers or clash with each
    // other / the fixed branches (edef_detail::is_fixed_branch, kept in sync by
    // check_fixed_branch_names); re-check here because a clash would make
    // ROOT keep two same-named branches and Draw read the wrong one.
    auto add = [&](const std::string& name, void* addr, const char* type) {
      if (events_->GetBranch(name.c_str())) {
        std::fprintf(stderr,
                     "nwall_sink: ERROR L2 name \"%s\" clashes with an existing events-tree "
                     "branch — NOT written this run (fix the event definition)\n",
                     name.c_str());
        return;
      }
      events_->Branch(name.c_str(), addr, (name + "/" + type).c_str());
    };
    for (std::size_t i = 0; i < d.counters.size(); ++i) add(d.counters[i].name, &l2_counts_[i], "i");
    for (std::size_t i = 0; i < n_l2_flags_; ++i) add(d.flags[i].name, &l2_flags_[i], "O");
    for (std::size_t i = 0; i < n_l2_accepts_; ++i) add(d.accepts[i].name, &l2_accepts_[i], "O");
    if (n_l2_accepts_ > 0) add("L2Accept", &l2_accept_, "O");
  }

  // ROOT copies NHits elements from each array branch's ADDRESS at Fill, so a
  // buffer smaller than the event would read past its end. Grow (double) and
  // re-point every array branch BEFORE that Fill — reallocation moves the
  // data, and a stale address would be a use-after-free, not just truncation.
  void ensure_hit_capacity(std::size_t n) {
    if (n <= hit_cap_) return;
    std::size_t cap = hit_cap_ ? hit_cap_ : kInitialHitCapacity;
    while (cap < n) cap *= 2;
    alloc_hit_buffers(cap);
    br_hit_mod_->SetAddress(hit_mod_.data());
    br_hit_ch_->SetAddress(hit_ch_.data());
    br_hit_e_->SetAddress(hit_e_.data());
    br_hit_es_->SetAddress(hit_es_.data());
    br_hit_t_->SetAddress(hit_t_.data());
    br_hit_trig_->SetAddress(hit_trig_.get());
    br_hit_ac_->SetAddress(hit_ac_.get());
  }

  // Write + close the run's trees, via the CURRENT file. When a tree crossed
  // MaxTreeSize, ROOT's ChangeFile DELETED our original TFile and moved EVERY
  // tree of that directory to <stem>_k.root (each one's entries so far are
  // saved into the old part first) — `file_` is then dangling and must not be
  // touched (empirically the allocator often reuses the same address, so a
  // pointer compare can lie; the file NAME is the only reliable rollover
  // signal). Since all trees move together, any existing one reports the
  // current file; which trees exist is a per-run choice (all optional), so
  // ask the first one present.
  // Returns the number of continuation parts (0 = the normal single-file case).
  // A failed file (write_failed_) is closed WITHOUT TTree::Write — ROOT still
  // writes its keys + header if it can, and the trees stay as of the last
  // good AutoSave, which is what a recovery reads.
  int close_all_parts() {
    TFile* cur = current_file();
    if (!cur) {
      // A failed rollover (ROOT could not create the next part) leaves the
      // trees without a file: nothing to write or close, and their ownership
      // is ROOT's — abandon them rather than risk a double delete.
      mark_failed("close", provisional_ + " (ROOT lost the current file)");
      reset_file_pointers();
      return 0;
    }
    bool rolled = provisional_ != cur->GetName();
    int parts = 0;
    if (rolled) {
      // Count <stem>_1.root ... — earlier parts were already closed by ROOT.
      std::string stem = provisional_.substr(0, provisional_.size() - 5);
      while (path_exists(stem + "_" + std::to_string(parts + 1) + ".root")) ++parts;
      std::fprintf(stderr,
                   "nwall_sink: WARNING tree crossed MaxTreeSize — ROOT split the run into "
                   "%d extra part(s); renaming them with Recorder-style sequence numbers\n",
                   parts);
    }
    cur->cd();
    if (!write_failed_) {
      for (TTree* t : {tree_, events_})
        if (t) t->Write();  // all trees live in the same (current) file
      check_file("TTree::Write");
    }
    const bool failed_before_close = write_failed_;
    cur->Close();  // deletes the trees: only `cur` may be looked at from here
    if (!failed_before_close && (cur->TestBit(TFile::kWriteError) || cur->IsZombie()))
      mark_failed("TFile::Close", cur->GetName());
    delete cur;  // == file_ only in the un-rolled case; file_ may be dangling
    reset_file_pointers();
    return parts;
  }

  void reset_file_pointers() {
    file_ = nullptr;
    tree_ = nullptr;
    events_ = nullptr;
    br_hit_mod_ = br_hit_ch_ = br_hit_e_ = br_hit_es_ = br_hit_t_ = br_hit_trig_ = nullptr;
    br_hit_ac_ = nullptr;
  }

  // The file the trees write to NOW: after a MaxTreeSize rollover `file_` is
  // dangling (see close_all_parts). All trees move together, so any existing
  // one reports it; lead is never null in practice (main() refuses to start
  // without a tree), and without one there was no rollover either.
  TFile* current_file() const {
    TTree* lead = tree_ ? tree_ : events_;
    return lead ? lead->GetCurrentFile() : file_;
  }
  std::string current_file_name() const {
    TFile* f = current_file();
    return f ? std::string(f->GetName()) : provisional_;
  }

  // The first failure of the run: ONE loud line, then the latch (fill /
  // fill_event / autosave / close_all_parts all honour it). No errno here: the
  // first symptom of a full disk is a SHORT write, which leaves a stale errno
  // behind (E2E: "No such file or directory" from an earlier stat). ROOT's own
  // (Sys)Error line, printed just before, has the cause; the free space tells
  // a full disk from an I/O error.
  void mark_failed(const char* op, const std::string& path) {
    if (write_failed_) return;
    write_failed_ = true;
    fail_op_ = op;
    std::fprintf(stderr,
                 "nwall_sink: ERROR run file write FAILED (%s) on %s (cause: ROOT's error just "
                 "above; %s). Nothing more is written to this run's .root, which is INCOMPLETE; "
                 "receiving, event building and the monitor go on, and the hits/events not "
                 "written are counted. The .delila is the authoritative record\n",
                 op, path.c_str(), free_space_str(out_dir_).c_str());
  }

  std::string pick_final_name(uint32_t run, int seq) const {
    char base[128];
    std::snprintf(base, sizeof(base), "run%04u_%04d_%s", run, seq, exp_name_.c_str());
    std::string cand = out_dir_ + "/" + base + ".root";
    if (!path_exists(cand)) return cand;
    // Collision -> append _<unix_ns> before the extension (Rust Recorder scheme).
    std::string c = out_dir_ + "/" + base + "_" + std::to_string(unix_nanos()) + ".root";
    return c;
  }

  TFile* file_ = nullptr;
  TTree* tree_ = nullptr;    // optional hits tree (--hits-tree)
  TTree* events_ = nullptr;  // optional generic events tree, same file
  std::string out_dir_;
  std::string exp_name_ = "data";
  std::string provisional_;
  std::string final_path_;
  long long entries_ = 0;         // successful Fills only
  long long events_entries_ = 0;
  // Which trees this run asked for (open_run) — they count unwritten entries
  // even when open_run could not create the file.
  bool hits_on_ = false;
  bool events_on_ = false;
  bool write_failed_ = false;  // this run's latch (mark_failed), reset by open_run
  std::string fail_op_;        // the operation that failed first
  long long unwritten_hits_ = 0;
  long long unwritten_events_ = 0;
  // Branch buffers (types match the leaflist: /b UChar_t, /s UShort_t, /D Double_t).
  UChar_t module_ = 0, channel_ = 0;
  UShort_t energy_ = 0, energy_short_ = 0;
  Double_t timestamp_ns_ = 0.0;
  // Events-tree buffers: per-event scalars (/l ULong64_t, /i UInt_t) ...
  ULong64_t ev_id_ = 0;
  Double_t ev_t_ = 0.0;
  UChar_t ev_mod_ = 0, ev_ch_ = 0, ev_idx_ = 0;
  UInt_t ev_ntrig_ = 0, ev_nhits_ = 0;
  // ... and the per-hit arrays, hit_cap_ elements each (ensure_hit_capacity).
  std::vector<UChar_t> hit_mod_, hit_ch_;
  std::vector<UShort_t> hit_e_, hit_es_;
  std::vector<Double_t> hit_t_;
  std::unique_ptr<Bool_t[]> hit_trig_;
  std::unique_ptr<Bool_t[]> hit_ac_;
  std::size_t hit_cap_ = 0;
  TBranch* br_hit_mod_ = nullptr;
  TBranch* br_hit_ch_ = nullptr;
  TBranch* br_hit_e_ = nullptr;
  TBranch* br_hit_es_ = nullptr;
  TBranch* br_hit_t_ = nullptr;
  TBranch* br_hit_trig_ = nullptr;
  TBranch* br_hit_ac_ = nullptr;
  // L2 scalars, sized once per run by add_l2_branches (never resized).
  std::vector<UInt_t> l2_counts_;
  std::unique_ptr<Bool_t[]> l2_flags_, l2_accepts_;
  std::size_t n_l2_flags_ = 0, n_l2_accepts_ = 0;
  Bool_t l2_accept_ = false;
  bool l2_size_warned_ = false;
  std::shared_ptr<const EventDef> event_def_;  // this run's frozen definition (open_run)
  eb::RunProvenance prov_;  // this run's time-offset + event-def record (open_run)
};

// Read an entire file into `out`. Returns false on open failure.
static bool read_file(const std::string& path, std::string& out) {
  std::ifstream f(path, std::ios::binary);
  if (!f) return false;
  std::ostringstream ss;
  ss << f.rdbuf();
  out = ss.str();
  return true;
}

// Write `text` to "<root path minus .root><suffix>" so the sidecar stays paired
// with its (possibly collision-suffixed) run file. Returns the path written, or
// "" (with a WARNING) on failure.
static std::string write_text_sidecar(const std::string& root_path, const std::string& suffix,
                                      const std::string& text) {
  if (root_path.empty()) return "";
  std::string base = root_path;
  const std::string ext = ".root";
  if (base.size() >= ext.size() &&
      base.compare(base.size() - ext.size(), ext.size(), ext) == 0)
    base.erase(base.size() - ext.size());
  std::string dst = base + suffix;
  std::ofstream out(dst, std::ios::binary | std::ios::trunc);
  if (!out) {
    std::fprintf(stderr, "nwall_sink: WARNING could not write %s\n", dst.c_str());
    return "";
  }
  out << text;
  return dst;
}

// ---------------------------------------------------------------------------
// Time offsets (--time-calib): file I/O around time_offsets.hpp
// ---------------------------------------------------------------------------
static std::string now_utc_iso() {
  std::time_t t = std::time(nullptr);
  std::tm tmv;
  gmtime_r(&t, &tmv);
  char buf[32];
  std::strftime(buf, sizeof buf, "%Y-%m-%dT%H:%M:%SZ", &tmv);
  return buf;
}

// Read + parse + resolve `path` into `a` (reload_offsets policy: on failure the
// previous table is kept and marked stale). The recorded path is the realpath.
static bool load_offsets_file(const std::string& path, ActiveOffsets& a, double max_spread_ns,
                              std::vector<std::string>& errors) {
  char resolved[4096];
  std::string real = ::realpath(path.c_str(), resolved) ? std::string(resolved) : path;
  std::string text;
  if (!read_file(path, text)) {
    std::string why = std::strerror(errno);
    return reload_offsets(a, nullptr, why, real, max_spread_ns, now_utc_iso(), errors);
  }
  return reload_offsets(a, &text, "", real, max_spread_ns, now_utc_iso(), errors);
}

static std::string offsets_summary(const ActiveOffsets& a) {
  char buf[256];
  std::snprintf(buf, sizeof buf, "%s, %zu entries, %d root(s), depth %d, spread %.3f ns",
                a.table.format.c_str(), a.table.abs_ns.size(), a.table.roots, a.table.max_depth,
                a.table.spread_ns());
  return buf;
}

// ---------------------------------------------------------------------------
// Generic event definition (--event-def): file I/O around event_def.hpp
// ---------------------------------------------------------------------------

// Read + parse + validate `path` into `a` (reload_event_def policy, identical
// to the offsets: on failure the previous definition is kept and marked
// stale). The recorded path is the realpath. The adopted definition's parser
// warnings are in a.warnings — the caller prints them.
static bool load_event_def_file(const std::string& path, ActiveEventDef& a,
                                double max_lookback_ns, std::vector<std::string>& errors) {
  char resolved[4096];
  std::string real = ::realpath(path.c_str(), resolved) ? std::string(resolved) : path;
  std::string text;
  if (!read_file(path, text)) {
    std::string why = std::strerror(errno);
    return reload_event_def(a, nullptr, why, real, max_lookback_ns, now_utc_iso(), errors);
  }
  return reload_event_def(a, &text, "", real, max_lookback_ns, now_utc_iso(), errors);
}

// "0" / "0,4" — the --module list as given.
static std::string module_list_str(const std::vector<int>& m) {
  std::string s;
  for (int x : m) s += (s.empty() ? "" : ",") + std::to_string(x);
  return s;
}

static bool module_kept(const std::vector<int>& modules, int m) {
  return modules.empty() || std::find(modules.begin(), modules.end(), m) != modules.end();
}

// Trigger channels on a digitizer that --module drops: those hits never reach
// the Sorter, so such a trigger can never open an event. Not fatal (one
// definition may serve several setups), but never silent.
static void warn_foreign_triggers(const EventDef& d, const std::vector<int>& modules) {
  if (modules.empty()) return;
  for (const TriggerDef& t : d.triggers) {
    std::string foreign;
    for (uint16_t k : t.channels)
      if (!module_kept(modules, k >> 8)) foreign += (foreign.empty() ? "" : ",") + mc_name(k);
    if (!foreign.empty())
      std::fprintf(stderr,
                   "nwall_sink: WARNING event def: trigger '%s' channel(s) %s are not on "
                   "--module %s, whose filter drops them before the Sorter — they can "
                   "never trigger an event\n",
                   t.name.c_str(), foreign.c_str(), module_list_str(modules).c_str());
  }
}

// Per-channel differences between the previous run's offset table and this
// run's, for the run log ("" = unchanged). %.3f like format_offsets_line; the
// delta in %.4g so a sub-display-precision edit still shows.
static std::string offsets_diff(const OffsetTable& before, const OffsetTable& after) {
  std::string s;
  auto add = [&](const std::string& item) { s += (s.empty() ? "" : ", ") + item; };
  char buf[128];
  for (const auto& kv : after.abs_ns) {
    auto it = before.abs_ns.find(kv.first);
    if (it == before.abs_ns.end()) {
      std::snprintf(buf, sizeof buf, "%s added (%+.3f)", mc_name(kv.first).c_str(), kv.second);
      add(buf);
    } else if (it->second != kv.second) {
      std::snprintf(buf, sizeof buf, "%s %+.3f -> %+.3f (delta %+.4g)", mc_name(kv.first).c_str(),
                    it->second, kv.second, kv.second - it->second);
      add(buf);
    }
  }
  for (const auto& kv : before.abs_ns)
    if (!after.abs_ns.count(kv.first)) {
      std::snprintf(buf, sizeof buf, "%s removed (was %+.3f)", mc_name(kv.first).c_str(),
                    kv.second);
      add(buf);
    }
  return s;
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Channel map (--channel-map): file I/O around nwall_map.hpp
// ---------------------------------------------------------------------------
static bool load_channel_map(const std::string& path, nwall::ChannelMap& out,
                             std::vector<std::string>& errors) {
  errors.clear();
  std::string text;
  if (!read_file(path, text)) {
    errors.push_back("cannot read " + path + ": " + std::strerror(errno));
    return false;
  }
  nwall::MapParseResult r = nwall::parse_channel_map(text, path);
  if (!r.errors.empty()) {
    errors = std::move(r.errors);
    return false;
  }
  out = std::move(r.map);
  return true;
}

static std::string map_summary(const nwall::ChannelMap& m) {
  using nwall::Det;
  char buf[256];
  std::snprintf(buf, sizeof buf,
                "%zu channel(s) with a role: beam %zu, LaBr %zu, LS %zu, dE %zu, E %zu, "
                "pulser %zu",
                m.roles.size(), m.count(Det::Beam), m.count(Det::LaBr), m.count(Det::LS),
                m.count(Det::DE), m.count(Det::E), m.count(Det::Pulser));
  return buf;
}

// ---------------------------------------------------------------------------
// N_Wall display (main thread only: owns every histogram and the THttpServer)
// ---------------------------------------------------------------------------

// Writer -> Display (bounded; data may be dropped-and-counted, control markers
// use the blocking push). Unlike root_sink's arrival-order tee, the display
// sees what the record path built: time-sorted core hits and their events.
struct DisplayMsg {
  eb::Ctrl ctrl = eb::Ctrl::None;
  std::vector<ScalarHit> hits;           // the chunk's core range, time sorted
  std::vector<eb::GenericEvent> events;  // the events emitted with this chunk
};

class Display {
 public:
  Display(THttpServer* server, const Options& opt) : server_(server), opt_(opt) {
    nwall::BunchClock::Config cc;
    cc.period_ns = opt.rf_period_ns;
    clock_ = nwall::BunchClock(cc);
    roles_.assign(65536, nwall::Role());
    const double T = opt.rf_period_ns;
    hitmap_ = add2("/hits", "hitmap", "hits;channel;module", 32, 0, 32, 16, 0, 16);
    ls_psd_ = add2("/ls", "ls_psd",
                   "LS pulse shape;E_{long} [ADC];(E_{long} - E_{short}) / E_{long}", 512, 0,
                   65536, 240, -0.2, 1.0);
    pid_ = add2("/events", "pid", "dE vs E (largest sector hit of each);E [ADC];dE [ADC]", 512, 0,
                32768, 512, 0, 32768);
    sector_ = add2("/events", "sector_dE_vs_E", "sector pairs in an event;E sector;dE sector", 16,
                   0, 16, 16, 0, 16);
    ring_sector_ = add2("/events", "dE_ring_vs_sector",
                        "dE ring vs dE sector in an event;dE sector;dE ring", 16, 0, 16, 45, 0, 45);
    ls_dt_si_ = add2("/events", "ls_dt_si", "t(LS) - t(trigger);#Deltat [ns];channel", 1000, -500,
                     1500, 32, 0, 32);
    si_phase_ = add1("/events", "si_phase",
                     "trigger - bunch at or before it (--si-delay-ns = the peak);"
                     "t - t_{bunch} [ns];events",
                     400, 0, T);
    ls_tof_si_ = add2("/events", "ls_tof_si",
                      "t(LS) - bunch picked by the Si - rf_t0;TOF [ns];channel", 550, -100, 1000,
                      32, 0, 32);
  }

  void set_map(const nwall::ChannelMap& m) {
    std::fill(roles_.begin(), roles_.end(), nwall::Role());
    for (const auto& kv : m.roles) roles_[kv.first] = kv.second;
    beam_key_ = m.beam_key;
    have_map_ = true;
  }

  // Run start: the digitizer clocks restart, so does the bunch clock.
  void run_open() { clock_.reset(); }

  void chunk(const DisplayMsg& d) {
    const bool beam = have_map_ && beam_key_ >= 0;
    // Chunks come in time order and an event reaches back at most its
    // window, so beam hits 10 ms before this chunk are no longer needed.
    if (beam && !d.hits.empty()) clock_.forget_before(d.hits.front().timestamp_ns - 10e6);
    // Beam hits first, so every hit of the chunk can use the nearest one
    // (either side), not just the last one before it.
    if (beam)
      for (const ScalarHit& h : d.hits)
        if (nwall::map_key(h.module, h.channel) == beam_key_) clock_.add_beam(h.timestamp_ns);
    for (const ScalarHit& h : d.hits) {
      hitmap_->Fill(h.channel, h.module);
      per_module(energy_, h.module, "/hits", "energy_m%d", "board %d;channel;energy [ADC]", 32,
                 0, 32, 1024, 0, 65536)
          ->Fill(h.channel, h.energy);
      const nwall::Role& r = roles_[nwall::map_key(h.module, h.channel)];
      if (r.det == nwall::Det::LS && h.energy > 0)
        ls_psd_->Fill(h.energy, (static_cast<double>(h.energy) - h.energy_short) / h.energy);
      if (beam) {
        const double u = h.timestamp_ns - opt_.rf_t0_ns;
        double tb;
        if (clock_.bunch_at_or_before(u, tb))
          per_module(tof_rf_, h.module, "/rf", "tof_rf_m%d",
                     "board %d: t - bunch at or before it - rf_t0;TOF mod T [ns];channel", 400, 0,
                     opt_.rf_period_ns, 32, 0, 32)
              ->Fill(u - tb, h.channel);
      }
    }
    if (have_map_)
      for (const eb::GenericEvent& ev : d.events) event(ev, beam);
  }

  void reset_hists() {
    for (TH1* h : all_) h->Reset();
  }

  std::string status() const {
    if (!have_map_ || beam_key_ < 0) return "beam=off";
    char buf[128];
    std::snprintf(buf, sizeof buf, "beam=%zu T=%.4f%s offgrid=%zu%s", clock_.beam_hits(),
                  clock_.period_ns(), clock_.period_measured() ? "" : "(nominal)",
                  clock_.off_grid(), clock_.capped() ? " CAPPED" : "");
    return buf;
  }

 private:
  struct SiHit {
    int index;
    uint16_t energy;
  };

  void event(const eb::GenericEvent& ev, bool beam) {
    using nwall::Det;
    using nwall::Kind;
    std::vector<SiHit> e_sec, de_sec, de_ring;
    for (const eb::EventHit& h : ev.hits) {
      const nwall::Role& r = roles_[nwall::map_key(h.mod, h.ch)];
      if (r.det == Det::E && r.kind == Kind::Sector) e_sec.push_back({r.index, h.energy});
      else if (r.det == Det::DE && r.kind == Kind::Sector) de_sec.push_back({r.index, h.energy});
      else if (r.det == Det::DE && r.kind == Kind::Ring) de_ring.push_back({r.index, h.energy});
    }
    auto largest = [](const std::vector<SiHit>& v) {
      return std::max_element(v.begin(), v.end(), [](const SiHit& a, const SiHit& b) {
        return a.energy < b.energy;
      });
    };
    if (!e_sec.empty() && !de_sec.empty()) pid_->Fill(largest(e_sec)->energy, largest(de_sec)->energy);
    for (const SiHit& e : e_sec)
      for (const SiHit& de : de_sec) sector_->Fill(e.index, de.index);
    for (const SiHit& s : de_sec)
      for (const SiHit& ring : de_ring) ring_sector_->Fill(s.index, ring.index);

    // The Si picks the bunch (TODO 79 §5.2): nearest bunch to t_ref - si_delay.
    // Only the bunch choice depends on the Si time (±T/2 tolerance); the TOF
    // itself is LS vs beam, both on the VX2730.
    double tb_pick = 0.0;
    bool picked = false;
    if (beam) {
      double tb;
      if (clock_.bunch_at_or_before(ev.t, tb)) si_phase_->Fill(ev.t - tb);
      picked = clock_.nearest_bunch(ev.t - opt_.si_delay_ns, tb_pick);
    }
    for (const eb::EventHit& h : ev.hits) {
      if (roles_[nwall::map_key(h.mod, h.ch)].det != Det::LS) continue;
      ls_dt_si_->Fill(h.dt, h.ch);
      if (picked) ls_tof_si_->Fill(ev.t + h.dt - tb_pick - opt_.rf_t0_ns, h.ch);
    }
  }

  TH1D* add1(const char* folder, const char* name, const char* title, int nx, double x0,
             double x1) {
    TH1D* h = new TH1D(name, title, nx, x0, x1);
    reg(folder, h, false);
    return h;
  }
  TH2D* add2(const char* folder, const char* name, const char* title, int nx, double x0,
             double x1, int ny, double y0, double y1) {
    TH2D* h = new TH2D(name, title, nx, x0, x1, ny, y0, y1);
    reg(folder, h, true);
    return h;
  }
  void reg(const char* folder, TH1* h, bool colz) {
    all_.push_back(h);
    if (!server_) return;
    server_->Register(folder, h);
    if (colz) server_->SetItemField(Form("%s/%s", folder, h->GetName()), "_drawopt", "colz");
  }
  // One histogram per board, created the first time the board shows up.
  TH2D* per_module(std::map<int, TH2D*>& m, int module, const char* folder, const char* name_fmt,
                   const char* title_fmt, int nx, double x0, double x1, int ny, double y0,
                   double y1) {
    auto it = m.find(module);
    if (it != m.end()) return it->second;
    TH2D* h = add2(folder, Form(name_fmt, module), Form(title_fmt, module), nx, x0, x1, ny, y0, y1);
    m[module] = h;
    return h;
  }

  THttpServer* server_;
  const Options& opt_;
  nwall::BunchClock clock_;
  std::vector<nwall::Role> roles_;  // by map_key; Det::None = no role
  int beam_key_ = -1;
  bool have_map_ = false;
  std::vector<TH1*> all_;
  TH2D* hitmap_ = nullptr;
  TH2D* ls_psd_ = nullptr;
  TH2D* pid_ = nullptr;
  TH2D* sector_ = nullptr;
  TH2D* ring_sector_ = nullptr;
  TH2D* ls_dt_si_ = nullptr;
  TH1D* si_phase_ = nullptr;
  TH2D* ls_tof_si_ = nullptr;
  std::map<int, TH2D*> energy_, tof_rf_;
};

int main(int argc, char** argv) {
  Options opt;
  bool help = false;
  if (!parse_args(argc, argv, opt, help)) return help ? 0 : 2;

  const bool http_enabled = opt.http_port != 0;
  const bool events_enabled = !opt.event_def.empty();
  const bool map_enabled = !opt.channel_map.empty();

  if (!opt.hits_tree && !events_enabled) {
    std::fprintf(stderr,
                 "nwall_sink: ERROR nothing would be recorded: give at least one of\n"
                 "nwall_sink:   --hits-tree        the per-hit tree (TOML hits_tree = true)\n"
                 "nwall_sink:   --event-def FILE   the generic events tree (TOML event_def_file)\n");
    return 2;
  }
  if (opt.tree_given && !opt.hits_tree)
    std::fprintf(stderr,
                 "nwall_sink: WARNING --tree %s given without --hits-tree — the hits tree will "
                 "NOT be written\n",
                 opt.tree.c_str());
  if (opt.hits_tree && events_enabled && opt.tree == kEventsTreeName) {
    std::fprintf(stderr,
                 "nwall_sink: ERROR two trees would both be named \"%s\" — pick a distinct "
                 "--tree\n",
                 kEventsTreeName);
    return 2;
  }
  if (map_enabled && !http_enabled)
    std::fprintf(stderr,
                 "nwall_sink: WARNING --channel-map %s is only used by the display, which "
                 "--http-port 0 turns off\n",
                 opt.channel_map.c_str());

  // Startup loads: ANY error is fatal here (at a later run start a bad edit
  // keeps the previous table / definition / map, STALE).
  const bool toff_enabled = !opt.time_calib.empty();
  const double safe_horizon_ns = opt.safe_horizon_ms * 1e6;
  ActiveOffsets toff_boot;
  if (toff_enabled) {
    std::vector<std::string> errs;
    if (!load_offsets_file(opt.time_calib, toff_boot, safe_horizon_ns, errs)) {
      std::fprintf(stderr, "nwall_sink: %zu error(s) in --time-calib %s:\n", errs.size(),
                   opt.time_calib.c_str());
      for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
      return 2;
    }
  }
  ActiveEventDef evdef_boot;
  if (events_enabled) {
    std::vector<std::string> errs;
    if (!load_event_def_file(opt.event_def, evdef_boot, safe_horizon_ns, errs)) {
      std::fprintf(stderr, "nwall_sink: %zu error(s) in --event-def %s:\n", errs.size(),
                   opt.event_def.c_str());
      for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
      return 2;
    }
  }
  nwall::ChannelMap map_boot;
  if (map_enabled) {
    std::vector<std::string> errs;
    if (!load_channel_map(opt.channel_map, map_boot, errs)) {
      std::fprintf(stderr, "nwall_sink: %zu error(s) in --channel-map %s:\n", errs.size(),
                   opt.channel_map.c_str());
      for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
      return 2;
    }
  }

  std::printf("nwall_sink: sub=%s out-dir=%s hits tree=%s\n", opt.zmq.c_str(),
              opt.out_dir.c_str(), opt.hits_tree ? opt.tree.c_str() : "OFF (opt-in: --hits-tree)");
  if (toff_enabled) {
    std::printf("nwall_sink: time offsets: %s (%s) — aligned_ts = raw_ts - offset_ns; re-read at "
                "every run start\n",
                toff_boot.source_path.c_str(), offsets_summary(toff_boot).c_str());
    for (const auto& w : toff_boot.table.warnings)
      std::fprintf(stderr, "nwall_sink: WARNING time offsets: %s\n", w.c_str());
  } else {
    std::printf("nwall_sink: time offsets: none (timestamp_ns recorded RAW)\n");
  }
  if (!opt.modules.empty())
    std::printf("nwall_sink: --module %s: only these digitizers' hits are recorded and shown "
                "(the .delila keeps every board)\n",
                module_list_str(opt.modules).c_str());
  if (events_enabled) {
    std::printf("nwall_sink: events tree \"%s\": %s (re-read at every run start)\n",
                kEventsTreeName, evdef_boot.source_path.c_str());
    std::printf("nwall_sink: event def: %s\n", format_event_def_line(*evdef_boot.def).c_str());
    for (const auto& w : evdef_boot.warnings)
      std::fprintf(stderr, "nwall_sink: WARNING event def: %s\n", w.c_str());
    warn_foreign_triggers(*evdef_boot.def, opt.modules);
  }
  if (map_enabled) {
    std::printf("nwall_sink: channel map %s: %s\n", opt.channel_map.c_str(),
                map_summary(map_boot).c_str());
    if (map_boot.beam_key < 0)
      std::fprintf(stderr,
                   "nwall_sink: WARNING the channel map has no beam channel — no bunch clock, "
                   "so no RF TOF, si_phase or ls_tof_si\n");
  } else if (http_enabled) {
    std::fprintf(stderr,
                 "nwall_sink: WARNING no --channel-map: only the per-board histograms (hitmap, "
                 "energy) are filled\n");
  }
  std::printf("nwall_sink: RF period %.4f ns (nominal; measured per run), rf_t0 %.3f ns, "
              "si_delay %.3f ns\n",
              opt.rf_period_ns, opt.rf_t0_ns, opt.si_delay_ns);

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  // A run stays one file (see root_sink.cxx for the rollover handling).
#ifdef ROOTSINK_TEST_MAX_TREE_SIZE
  TTree::SetMaxTreeSize(ROOTSINK_TEST_MAX_TREE_SIZE);
#else
  TTree::SetMaxTreeSize(2000000000000LL);  // 2 TB
#endif

  // --- ROOT display objects (before any TFile; detached from gDirectory) ---
  THttpServer* server = nullptr;
  std::unique_ptr<Display> display;
  if (http_enabled) {
    TH1::AddDirectory(kFALSE);
    // Batch mode: a headless sink must never connect to $DISPLAY (ROOT's X11
    // error handler kills the process when that connection dies).
    gROOT->SetBatch(kTRUE);
    static int fake_argc = 1;
    static char a0[] = "nwall_sink";
    static char* fake_argv[] = {a0, nullptr};
    new TApplication("nwall_sink", &fake_argc, fake_argv);

    server = new THttpServer(Form("http:%d", opt.http_port));
    server->SetItemField("/", "_monitoring", "2000");
    // Never expose the Writer's live run file over HTTP (see root_sink.cxx).
    server->GetSniffer()->SetScanGlobalDir(kFALSE);
    display.reset(new Display(server, opt));
    if (map_enabled) display->set_map(map_boot);
    server->RegisterCommand(
        "/Reset", Form("((void(*)())%p)();", reinterpret_cast<void*>(&request_reset)),
        "button;Reset histograms");
    server->CreateItem("/WriteStatus", "run file write status (TODO 69)");
    server->SetItemField("/WriteStatus", "_kind", "Text");
    server->SetItemField("/WriteStatus", "value", "OK");
    std::printf("nwall_sink: THttpServer on http://<host>:%d/  (open in a browser; JSROOT)\n",
                opt.http_port);
  }

  // --- ZMQ SUB (HWM=0 per the data-preservation rule) ---
  void* ctx = zmq_ctx_new();
  void* sub = zmq_socket(ctx, ZMQ_SUB);
  int zero = 0;
  zmq_setsockopt(sub, ZMQ_RCVHWM, &zero, sizeof(zero));
  zmq_setsockopt(sub, ZMQ_SUBSCRIBE, "", 0);
  if (zmq_connect(sub, opt.zmq.c_str()) != 0) {
    std::fprintf(stderr, "nwall_sink: zmq_connect(%s) failed: %s\n", opt.zmq.c_str(),
                 zmq_strerror(zmq_errno()));
    return 3;
  }

  ROOT::EnableThreadSafety();

  struct Counters {
    std::atomic<long long> received{0};
    std::atomic<long long> written{0};
    std::atomic<bool> write_failed{false};
    std::atomic<long long> unwritten_hits{0};
    std::atomic<long long> unwritten_events{0};
    std::atomic<long long> chunks{0};
    std::atomic<long long> display_drops{0};
    std::atomic<long long> other_module{0};
    std::atomic<long long> late{0};
    std::atomic<bool> writing{false};
    std::atomic<int> toff_entries{-1};
    std::atomic<bool> toff_stale{false};
    std::atomic<long long> toff_unmapped{0};
    std::atomic<long long> events{0};
    std::atomic<long long> ev_contested{0};
    std::atomic<long long> ev_orphans{0};
    std::atomic<bool> evdef_stale{false};
    std::atomic<long long> ev_l2acc{0};
    std::atomic<bool> run_has_l2acc{false};
  } counters;
  if (toff_enabled) counters.toff_entries = static_cast<int>(toff_boot.table.abs_ns.size());

  const int n_workers = opt.workers;

  eb::Channel<eb::SorterMsg> to_sorter;  // record path: unbounded, never drops
  eb::Channel<eb::WorkMsg> work_q;       // record path: unbounded
  eb::Channel<eb::WorkMsg> writer_q;     // record path: unbounded
  // Display queue: bounded; DATA may drop (counted — the CLAUDE.md Monitor
  // exception; the record path above is untouched). Control markers use the
  // blocking push.
  eb::Channel<DisplayMsg> display_q(1000);

  // --- Receiver: ZMQ recv + decode + run state -> in-band markers ----------
  auto receiver_loop = [&] {
    RunState run_state;
    std::set<std::string> warned;
    ActiveOffsets toff = toff_boot;
    std::set<uint16_t> toff_reported;
    auto report_unmapped = [&](const std::string& what) {
      if (!toff_enabled || toff_reported.empty()) return;
      std::string keys;
      for (uint16_t k : toff_reported) keys += (keys.empty() ? "" : " ") + mc_name(k);
      std::printf("nwall_sink: %s time offsets: %zu (module, channel) key(s) had no entry and "
                  "stayed RAW: %s\n",
                  what.c_str(), toff_reported.size(), keys.c_str());
    };
    ActiveEventDef evdef = evdef_boot;
    auto process = [&](const uint8_t* data, size_t size) {
      Envelope env = parse_envelope(data, size);
      switch (env.kind) {
        case MsgKind::Data: {
          uint32_t sid = 0;
          std::vector<ScalarHit> hits;
          long n = decode_batch_into(env.payload, env.payload_size, sid, hits);
          if (n < 0) {
            std::fprintf(stderr, "nwall_sink: WARNING malformed Data batch (skipped)\n");
            break;
          }
          run_state.on_data(sid, [&] {
            counters.writing = true;
            eb::SorterMsg m;
            m.ctrl = eb::Ctrl::RunOpen;
            if (toff_enabled) {
              bool was_stale = toff.stale;
              const OffsetTable before = toff.table;
              std::vector<std::string> errs;
              if (!load_offsets_file(opt.time_calib, toff, safe_horizon_ns, errs)) {
                std::fprintf(stderr,
                             "nwall_sink: ERROR time offsets reload failed at run start — "
                             "KEEPING THE PREVIOUS TABLE for this run:\n");
                for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
              } else {
                if (was_stale) std::printf("nwall_sink: time offsets reloaded OK (was STALE)\n");
                const std::string diff = offsets_diff(before, toff.table);
                if (!diff.empty())
                  std::printf("nwall_sink: time offsets CHANGED vs the previous run: %s\n",
                              diff.c_str());
              }
              toff_reported.clear();
              counters.toff_unmapped = 0;
              counters.toff_stale = toff.stale;
              counters.toff_entries = static_cast<int>(toff.table.abs_ns.size());
              std::printf("nwall_sink: run time offsets [%s%s]: %s\n",
                          offsets_summary(toff).c_str(), toff.stale ? ", STALE" : "",
                          format_offsets_line(toff.table).c_str());
              m.prov.time_offsets_meta = offsets_provenance_json(toff);
              m.prov.time_offsets_source = toff.source_text;
            } else {
              m.prov.time_offsets_meta = offsets_disabled_json();
            }
            if (events_enabled) {
              bool was_stale = evdef.stale;
              std::vector<std::string> errs;
              if (!load_event_def_file(opt.event_def, evdef, safe_horizon_ns, errs)) {
                std::fprintf(stderr,
                             "nwall_sink: ERROR event definition reload failed at run start — "
                             "KEEPING THE PREVIOUS DEFINITION for this run:\n");
                for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
              } else if (was_stale) {
                std::printf("nwall_sink: event definition reloaded OK (was STALE)\n");
              }
              counters.evdef_stale = evdef.stale;
              std::printf("nwall_sink: run event def [%s%s]: %s\n", evdef.source_path.c_str(),
                          evdef.stale ? ", STALE" : "",
                          format_event_def_line(*evdef.def).c_str());
              for (const auto& w : evdef.warnings)
                std::fprintf(stderr, "nwall_sink: WARNING event def: %s\n", w.c_str());
              warn_foreign_triggers(*evdef.def, opt.modules);
              m.event_def = evdef.def;
              m.prov.event_def_meta = event_def_provenance_json(evdef);
              m.prov.event_def_source = evdef.source_text;
            }
            to_sorter.push(std::move(m));
            std::printf("nwall_sink: run started (source %u)\n", sid);
            std::fflush(stdout);
          });
          // --module, then time alignment: both before ANY consumer (the Sorter
          // sorts on timestamp_ns).
          std::size_t decoded = hits.size();
          keep_modules(hits, opt.modules);
          counters.other_module += static_cast<long long>(decoded - hits.size());
          if (toff_enabled)
            apply_time_offsets(hits, toff.table, toff_reported, [&](uint16_t k) {
              ++counters.toff_unmapped;
              std::fprintf(stderr,
                           "nwall_sink: WARNING time offsets: module %u channel %u has no "
                           "entry in %s — recorded with offset 0 (reported once per run)\n",
                           static_cast<unsigned>(k >> 8), static_cast<unsigned>(k & 0xFF),
                           toff.source_path.c_str());
            });
          counters.received += static_cast<long long>(hits.size());
          eb::SorterMsg m;
          m.hits = std::move(hits);
          to_sorter.push(std::move(m));
          break;
        }
        case MsgKind::EndOfStream: {
          run_state.on_eos(
              env.source_id, env.run_number,
              [&](uint32_t rn) {
                counters.writing = false;
                report_unmapped("run " + std::to_string(rn));
                eb::SorterMsg m;
                m.ctrl = eb::Ctrl::RunClose;
                m.run_number = rn;
                to_sorter.push(std::move(m));
              },
              [&](uint32_t sid, uint32_t rn) {
                std::printf("nwall_sink: ignoring stale EOS (source %u, run %u) while idle\n",
                            sid, rn);
              });
          break;
        }
        case MsgKind::Heartbeat:
          break;
        case MsgKind::Unknown:
          if (warned.insert(env.variant).second)
            std::fprintf(stderr,
                         "nwall_sink: WARNING unknown message variant '%s' (skipping; further "
                         "ones silenced)\n",
                         env.variant.c_str());
          break;
      }
    };
    while (!g_stop) {
      zmq_pollitem_t items[1];
      items[0].socket = sub;
      items[0].fd = 0;
      items[0].events = ZMQ_POLLIN;
      items[0].revents = 0;
      int rc = zmq_poll(items, 1, 100);
      if (rc < 0) {
        if (zmq_errno() == EINTR) continue;
        std::fprintf(stderr, "nwall_sink: zmq_poll error: %s\n", zmq_strerror(zmq_errno()));
        break;
      }
      if (items[0].revents & ZMQ_POLLIN) {
        while (true) {
          zmq_msg_t msg;
          zmq_msg_init(&msg);
          int nb = zmq_msg_recv(&msg, sub, ZMQ_DONTWAIT);
          if (nb < 0) {
            zmq_msg_close(&msg);
            break;
          }
          process(static_cast<const uint8_t*>(zmq_msg_data(&msg)), zmq_msg_size(&msg));
          zmq_msg_close(&msg);
        }
      }
    }
    if (run_state.is_writing()) report_unmapped("unfinalized run");
    // Orderly teardown: Shutdown goes down the record path; the Writer passes
    // it on to the display once it has drained.
    eb::SorterMsg m;
    m.ctrl = eb::Ctrl::Shutdown;
    to_sorter.push(std::move(m));
    zmq_close(sub);
  };

  // --- Sorter: disorder buffer -> sorted chunks with sequence numbers ------
  auto sorter_loop = [&] {
    eb::Sorter::Config scfg;
    scfg.chunk_span_ns = opt.chunk_span_ms * 1e6;
    scfg.safe_horizon_ns = opt.safe_horizon_ms * 1e6;
    scfg.lookback_ns = 0.0;  // set per run from the event definition
    eb::Sorter sorter(scfg);
    eb::TriggerGrouper grouper;
    uint64_t seq = 0;
    uint64_t reported_late = 0;
    auto report_late = [&](const eb::Sorter& so) {
      counters.late = static_cast<long long>(so.late_hits());
      if (so.late_hits() > reported_late)
        std::fprintf(stderr,
                     "nwall_sink: WARNING %llu hit(s) this run arrived more than %.0f ms behind "
                     "the newest hit and are MISSING from every tree of the .root file (the "
                     ".delila has them). Raise --safe-horizon-ms (a slow low-rate DIG1 board?)\n",
                     static_cast<unsigned long long>(so.late_hits()), opt.safe_horizon_ms);
      reported_late = so.late_hits();
    };
    auto emit_chunk = [&](eb::SortedChunk&& c) {
      eb::WorkMsg m;
      m.seq = seq++;
      m.chunk = std::move(c);
      if (grouper.enabled()) m.plan = grouper.plan_chunk(m.chunk);
      ++counters.chunks;
      work_q.push(std::move(m));
    };
    eb::SorterMsg in;
    while (to_sorter.pop(in)) {
      switch (in.ctrl) {
        case eb::Ctrl::None:
          sorter.push_batch(std::move(in.hits), emit_chunk);
          counters.late = static_cast<long long>(sorter.late_hits());
          break;
        case eb::Ctrl::RunOpen: {
          sorter.reset();
          const double prev_lookback = sorter.lookback_ns();
          const double lookback = in.event_def ? in.event_def->lookback_ns() : 0.0;
          sorter.set_lookback_ns(lookback);
          grouper.reset(in.event_def);
          if (lookback != prev_lookback)
            std::printf("nwall_sink: sorter lookback %.0f ns (event def G+2*pre+post)\n",
                        lookback);
          reported_late = 0;
          eb::WorkMsg m;
          m.ctrl = eb::Ctrl::RunOpen;
          m.seq = seq++;
          m.prov = std::move(in.prov);
          m.event_def = in.event_def;
          work_q.push(std::move(m));
          break;
        }
        case eb::Ctrl::RunClose: {
          sorter.flush(emit_chunk);
          report_late(sorter);
          eb::WorkMsg m;
          m.ctrl = eb::Ctrl::RunClose;
          m.seq = seq++;
          m.run_number = in.run_number;
          work_q.push(std::move(m));
          break;
        }
        case eb::Ctrl::Shutdown: {
          sorter.flush(emit_chunk);
          report_late(sorter);
          for (int i = 0; i < n_workers; ++i) {
            eb::WorkMsg m;
            m.ctrl = eb::Ctrl::Shutdown;
            m.seq = seq++;
            work_q.push(std::move(m));
          }
          return;
        }
      }
    }
  };

  // --- Workers: the pure-function event build; control passes through -------
  auto worker_loop = [&] {
    eb::WorkMsg m;
    while (work_q.pop(m)) {
      if (m.ctrl == eb::Ctrl::None && m.plan.def)
        m.events = eb::build_generic_events(m.chunk, m.plan, m.gstats);
      bool shutdown = (m.ctrl == eb::Ctrl::Shutdown);
      writer_q.push(std::move(m));
      if (shutdown) return;
    }
  };

  // --- Writer: seq-ordered ROOT I/O (the ONLY thread touching the run file),
  //     and the source of the display queue ---------------------------------
  auto writer_loop = [&] {
    Recorder recorder;
    eb::SeqReorder<eb::WorkMsg> reorder;
    int pills = 0;
    auto t_last_autosave = Clock::now();
    bool done = false;
    eb::GenericStats run_gstats;
    long long run_core_hits = 0;
    std::shared_ptr<const EventDef> run_def;
    auto control_to_display = [&](eb::Ctrl c) {
      DisplayMsg d;
      d.ctrl = c;
      display_q.push(std::move(d));  // blocking: control is never dropped
    };
    auto report_events = [&](const std::string& what) {
      std::string dist;
      for (std::size_t i = 0; i < run_gstats.ntrig.size(); ++i) {
        if (run_gstats.ntrig[i] == 0) continue;
        char b[48];
        std::snprintf(b, sizeof b, "%s%zu%s:%llu", dist.empty() ? "" : " ", i + 1,
                      i + 1 == run_gstats.ntrig.size() ? "+" : "",
                      static_cast<unsigned long long>(run_gstats.ntrig[i]));
        dist += b;
      }
      std::printf("nwall_sink: %s events: %llu (NTriggers %s), hits in events %llu, "
                  "contested %llu, orphans %llu\n",
                  what.c_str(), static_cast<unsigned long long>(run_gstats.events),
                  dist.empty() ? "-" : dist.c_str(),
                  static_cast<unsigned long long>(run_gstats.hits_in_events),
                  static_cast<unsigned long long>(run_gstats.contested),
                  static_cast<unsigned long long>(run_gstats.orphans));
      if (run_def && run_def->has_l2()) {
        const EventDef& d = *run_def;
        std::string l2;
        auto add = [&](const std::string& name, uint64_t n) {
          if (!l2.empty() && l2.back() != ' ') l2 += ' ';
          l2 += name + "=" + std::to_string(static_cast<unsigned long long>(n));
        };
        for (std::size_t i = 0; i < d.flags.size() && i < run_gstats.flag_true.size(); ++i)
          add(d.flags[i].name, run_gstats.flag_true[i]);
        if (!d.accepts.empty()) {
          if (!l2.empty()) l2 += " | ";
          for (std::size_t i = 0; i < d.accepts.size() && i < run_gstats.accept_true.size(); ++i)
            add(d.accepts[i].name, run_gstats.accept_true[i]);
          l2 += " | ";
          add("L2Accept", run_gstats.l2_accepted);
        }
        if (l2.empty()) l2 = std::to_string(d.counters.size()) + " counter(s), no flags";
        std::printf("nwall_sink: %s L2: %s\n", what.c_str(), l2.c_str());
      }
      const unsigned long long accounted = run_gstats.hits_in_events + run_gstats.orphans;
      if (accounted != static_cast<unsigned long long>(run_core_hits))
        std::fprintf(stderr,
                     "nwall_sink: WARNING %s: the generic event layer accounted for %llu hits "
                     "(in events + orphans) but the Sorter emitted %lld — a hit was lost or "
                     "doubled (eb_core.hpp section 7 bug; the .delila is complete)\n",
                     what.c_str(), accounted, run_core_hits);
    };
    while (!done) {
      eb::WorkMsg m;
      const auto pr = writer_q.pop_for(m, 200);
      if (pr == eb::PopResult::Closed) break;
      if (pr == eb::PopResult::Value) {
        reorder.push(m.seq, std::move(m));
        eb::WorkMsg r;
        while (!done && reorder.pop_ready(r)) {
          switch (r.ctrl) {
            case eb::Ctrl::None: {
              // Core range ONLY (chunks carry duplicated context for the builders).
              auto range = eb::core_range(r.chunk);
              const long long n_core = static_cast<long long>(range.second - range.first);
              if (opt.hits_tree) {
                long long filled = 0;
                for (std::size_t i = range.first; i < range.second; ++i)
                  if (recorder.fill(r.chunk.hits[i])) ++filled;
                counters.written += filled;
              }
              if (r.plan.def) {
                for (const eb::GenericEvent& ev : r.events) recorder.fill_event(ev);
                run_gstats.add(r.gstats);
                run_core_hits += n_core;
                counters.events += static_cast<long long>(r.events.size());
                counters.ev_contested += static_cast<long long>(r.gstats.contested);
                counters.ev_orphans += static_cast<long long>(r.gstats.orphans);
                counters.ev_l2acc += static_cast<long long>(r.gstats.l2_accepted);
              }
              recorder.check_file("TTree::Fill");
              // Display copy AFTER the record path is served: the sorted core
              // hits and the events, dropped (and counted) if the display lags.
              if (http_enabled) {
                DisplayMsg d;
                d.hits.assign(r.chunk.hits.begin() + static_cast<std::ptrdiff_t>(range.first),
                              r.chunk.hits.begin() + static_cast<std::ptrdiff_t>(range.second));
                d.events = r.events;
                if (!display_q.try_push(std::move(d))) ++counters.display_drops;
              }
              break;
            }
            case eb::Ctrl::RunOpen: {
              std::string exp = resolve_exp_name(opt);
              run_gstats = eb::GenericStats();
              run_core_hits = 0;
              run_def = r.event_def;
              if (events_enabled && !run_def)
                std::fprintf(stderr,
                             "nwall_sink: ERROR RunOpen without an event definition — no events "
                             "tree this run (bug)\n");
              counters.run_has_l2acc = run_def && run_def->has_l2() && !run_def->accepts.empty();
              if (!recorder.open_run(opt.out_dir, opt.hits_tree ? opt.tree : std::string(), exp,
                                     run_def, r.prov))
                std::fprintf(stderr,
                             "nwall_sink: run start FAILED — this run is NOT recorded to .root "
                             "(counted as unwritten)\n");
              else
                std::printf("nwall_sink: run file -> %s\n", recorder.provisional().c_str());
              std::fflush(stdout);
              control_to_display(eb::Ctrl::RunOpen);
              break;
            }
            case eb::Ctrl::RunClose: {
              std::string counts = recorder.entry_counts();
              std::string fn = recorder.finalize(r.run_number);
              if (!recorder.write_failed())
                std::printf("nwall_sink: run %u finalized -> %s (%s)\n", r.run_number, fn.c_str(),
                            counts.c_str());
              else if (fn.empty())
                std::printf("nwall_sink: run %u NOT recorded (no run file) — %s. The .delila is "
                            "the authoritative record\n",
                            r.run_number, recorder.failure_summary().c_str());
              else
                std::printf("nwall_sink: run %u finalized -> %s INCOMPLETE (%s filled) — %s. "
                            "Opening it recovers the trees as of the last AutoSave before the "
                            "failure (TFile::Recover); the .delila is the authoritative record\n",
                            r.run_number, fn.c_str(), counts.c_str(),
                            recorder.failure_summary().c_str());
              if (events_enabled) report_events("run " + std::to_string(r.run_number));
              if (!recorder.provenance().time_offsets_source.empty()) {
                std::string ts = write_text_sidecar(recorder.final_path(), "_timeSettings.json",
                                                    recorder.provenance().time_offsets_source);
                if (!ts.empty()) std::printf("nwall_sink: time settings snapshot -> %s\n", ts.c_str());
              }
              if (!recorder.provenance().event_def_source.empty()) {
                std::string ed = write_text_sidecar(recorder.final_path(), "_eventDef.json",
                                                    recorder.provenance().event_def_source);
                if (!ed.empty())
                  std::printf("nwall_sink: event definition snapshot -> %s\n", ed.c_str());
              }
              std::fflush(stdout);
              control_to_display(eb::Ctrl::RunClose);
              break;
            }
            case eb::Ctrl::Shutdown:
              if (++pills >= n_workers) done = true;
              break;
          }
        }
      }
      auto now = Clock::now();
      if (recorder.is_open() &&
          std::chrono::duration_cast<std::chrono::seconds>(now - t_last_autosave).count() >=
              opt.autosave_sec) {
        recorder.autosave();
        t_last_autosave = now;
      }
      counters.write_failed = recorder.write_failed();
      counters.unwritten_hits = recorder.unwritten_hits();
      counters.unwritten_events = recorder.unwritten_events();
    }
    if (recorder.is_open()) {
      recorder.close_unfinalized();
      if (events_enabled) report_events("unfinalized run");
    }
    control_to_display(eb::Ctrl::Shutdown);
  };

  std::thread th_receiver(receiver_loop);
  std::thread th_sorter(sorter_loop);
  std::vector<std::thread> th_workers;
  for (int i = 0; i < n_workers; ++i) th_workers.emplace_back(worker_loop);
  std::thread th_writer(writer_loop);

  // --- Display (this thread): histograms + HTTP + status line --------------
  bool display_done = false;
  nwall::ChannelMap map_in_force = map_boot;
  bool map_stale = false;
  auto handle_display = [&](DisplayMsg& d) {
    switch (d.ctrl) {
      case eb::Ctrl::None:
        if (display) display->chunk(d);
        break;
      case eb::Ctrl::RunOpen:
        // The map follows the reload policy of the time offsets: re-read at
        // every run start; a bad edit keeps the previous map (STALE).
        if (map_enabled) {
          std::vector<std::string> errs;
          nwall::ChannelMap fresh;
          if (load_channel_map(opt.channel_map, fresh, errs)) {
            if (map_stale) std::printf("nwall_sink: channel map reloaded OK (was STALE)\n");
            map_in_force = std::move(fresh);
            map_stale = false;
          } else {
            std::fprintf(stderr,
                         "nwall_sink: ERROR channel map reload failed at run start — KEEPING "
                         "THE PREVIOUS MAP for this run:\n");
            for (const auto& e : errs) std::fprintf(stderr, "nwall_sink:   - %s\n", e.c_str());
            map_stale = true;
          }
          std::printf("nwall_sink: run channel map [%s%s]: %s\n", opt.channel_map.c_str(),
                      map_stale ? ", STALE" : "", map_summary(map_in_force).c_str());
          if (display) display->set_map(map_in_force);
        }
        if (display) display->run_open();
        break;
      case eb::Ctrl::RunClose:
        if (display)
          std::printf("nwall_sink: run closed — %s (histograms kept across runs; /Reset zeroes)\n",
                      display->status().c_str());
        break;
      case eb::Ctrl::Shutdown:
        display_done = true;
        break;
    }
  };

  std::printf("nwall_sink: pipeline receiver -> sorter(span=%.0fms horizon=%.0fms) -> %d "
              "worker(s) -> writer -> display\n",
              opt.chunk_span_ms, opt.safe_horizon_ms, n_workers);
  std::printf("nwall_sink: running. Ctrl-C or SIGTERM to stop.\n");
  std::fflush(stdout);

  auto t_last_status = Clock::now();
  long long last_received = 0;
  auto write_failure_text = [&]() -> std::string {
    if (!counters.write_failed) return "";
    std::string s = "WRITE FAILED: run file INCOMPLETE, unwritten";
    if (opt.hits_tree) s += " hits=" + std::to_string((long long)counters.unwritten_hits);
    if (events_enabled) s += " events=" + std::to_string((long long)counters.unwritten_events);
    return s;
  };
  std::string shown_write_status = "OK";

  while (!display_done) {
    DisplayMsg d;
    auto got = display_q.pop_for(d, 100);
    while (got == eb::PopResult::Value && !display_done) {
      handle_display(d);
      got = display_q.pop_for(d, 0);
    }
    if (got == eb::PopResult::Closed) display_done = true;

    if (server) {
      std::string ws = write_failure_text();
      ws = ws.empty() ? "OK"
                      : ws + " (see the nwall_sink log; the .delila is the authoritative record)";
      if (ws != shown_write_status) {
        server->SetItemField("/WriteStatus", "value", ws.c_str());
        shown_write_status = ws;
      }
      gSystem->ProcessEvents();
    }
    if (g_reset) {
      g_reset = 0;
      if (display) display->reset_hists();
      std::printf("nwall_sink: /Reset — histograms zeroed\n");
      std::fflush(stdout);
    }

    auto now = Clock::now();
    using std::chrono::duration;
    using std::chrono::duration_cast;
    using std::chrono::seconds;
    if (duration_cast<seconds>(now - t_last_status).count() >= 10) {
      double dt = duration_cast<duration<double>>(now - t_last_status).count();
      long long rec = counters.received;
      double rate = dt > 0 ? (double)(rec - last_received) / dt : 0.0;
      char toff_s[64];
      int te = counters.toff_entries;
      if (te < 0) {
        std::snprintf(toff_s, sizeof toff_s, "off");
      } else {
        long long um = counters.toff_unmapped;
        std::snprintf(toff_s, sizeof toff_s, "%d%s", te, counters.toff_stale ? " STALE" : "");
        if (um > 0) {
          std::size_t used = std::strlen(toff_s);
          std::snprintf(toff_s + used, sizeof toff_s - used, " (%lld unmapped)", um);
        }
      }
      char written_s[48];
      if (opt.hits_tree)
        std::snprintf(written_s, sizeof written_s, "written=%lld", (long long)counters.written);
      else
        std::snprintf(written_s, sizeof written_s, "hits tree off");
      char ev_s[160];
      if (!events_enabled) {
        std::snprintf(ev_s, sizeof ev_s, "off");
      } else {
        char extra[48] = "";
        if (counters.run_has_l2acc)
          std::snprintf(extra, sizeof extra, " l2acc=%lld", (long long)counters.ev_l2acc);
        std::snprintf(ev_s, sizeof ev_s, "%lld (cont=%lld orph=%lld%s)%s",
                      (long long)counters.events, (long long)counters.ev_contested,
                      (long long)counters.ev_orphans, extra, counters.evdef_stale ? " STALE" : "");
      }
      std::string wf = write_failure_text();
      if (!wf.empty()) wf = " | " + wf;
      std::printf(
          "nwall_sink: %s%s | hits=%lld (%s) | %.0f hits/s | chunks=%lld | ev=%s | %s%s | "
          "q[sort=%zu work=%zu write=%zu disp=%zu] | display_drops=%lld | other_module=%lld | "
          "late=%lld | toff=%s\n",
          counters.writing ? "WRITING" : "idle", wf.c_str(), rec, written_s, rate,
          (long long)counters.chunks, ev_s, display ? display->status().c_str() : "display off",
          map_stale ? " map STALE" : "", to_sorter.size(), work_q.size(), writer_q.size(),
          display_q.size(), (long long)counters.display_drops, (long long)counters.other_module,
          (long long)counters.late, toff_s);
      std::fflush(stdout);
      t_last_status = now;
      last_received = rec;
    }
  }

  std::printf("nwall_sink: stopping...\n");
  std::fflush(stdout);
  th_receiver.join();
  th_sorter.join();
  for (auto& t : th_workers) t.join();
  th_writer.join();
  zmq_ctx_term(ctx);
  if (opt.hits_tree)
    std::printf("nwall_sink: bye (%lld hits received, %lld written).\n",
                (long long)counters.received, (long long)counters.written);
  else
    std::printf("nwall_sink: bye (%lld hits received; hits tree off).\n",
                (long long)counters.received);
  return 0;
}
