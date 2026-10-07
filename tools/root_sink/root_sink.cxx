// root_sink.cxx — DELILA parallel ROOT sink + live Δt monitor.
//
// One process, two roles, subscribing to the DELILA merger's ZMQ PUB as an
// ADDITIONAL consumer (the Rust Recorder that writes `.delila` stays the
// authoritative recorder — this tool never gates or throttles the pipeline):
//
//   Role 1 (recorder): ZSTD-compressed ROOT TTrees, one file per run, named on
//     the EOS-carried run number. Up to two trees, each opt-in (at least one
//     is required — nothing recorded is a startup error):
//       * hits   (--hits-tree, name from --tree): every hit as 5 scalars, time
//         sorted. Opt-in since TODO 71: the time series is the .delila's job
//         (delila2root reproduces it any time);
//       * events (--event-def FILE, TODO 71): generic trigger-anchored events,
//         one entry per event with dictionary-free hit arrays.
//     The ThGEM `built` tree (--built-tree) was retired in TODO 71: its rows
//     are rebuilt offline from the events tree by thgem_events.C
//     (thgem_reco.hpp), with the same branches.
//
//   Role 2 (monitor): a coincidence Δt monitor for the ThGEM test (single
//     digitizer, gamma + ThGEM1 + ThGEM2 on configurable channels). Three
//     histograms (dt1, dt2, dt2-vs-dt1) plus channel occupancy, served live over
//     THttpServer (browse with a JSROOT-capable browser — zero frontend code).
//     With --hists, up to two delay-line XY monitors (--xy1-ch/--xy2-ch: a
//     trigger plus XL/XR/YU/YD arms each) additionally fill pos1/pos2-scope
//     histograms (x = t_XR - t_XL, y = t_YU - t_YD).
//
// Time offsets (--time-calib, TODO 70): a timeSettings.json of per-(module,
// channel) cable/delay offsets is applied once, in the Receiver, to every hit
// (aligned = raw - offset_ns) — so the hits tree, generic events and monitors are
// all on the aligned axis, while the .delila stays raw. The applied table is
// recorded per run (TTree UserInfo + <run>_timeSettings.json).
//
// Generic events (--event-def, TODO 71): the event definition JSON
// (event_def.hpp) follows the same reload/provenance policy as the offsets —
// re-read at every run start, frozen per run, recorded in the events tree's
// UserInfo + <run>_eventDef.json. Rules 1-2 (trigger grouping, reference) run
// in the single-threaded Sorter, rules 3-4 (window, hit assignment) in the
// Workers (eb_core.hpp section 7).
//
// All logic (envelope parse, decode, matcher, run state) is in sink_core.hpp and
// is unit-tested without ROOT/ZMQ. This file is only the wiring.
//
// Build (see README.md):
//   g++ -O2 -std=c++17 root_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o root_sink
//
// License: BSD-3-Clause (same as delila-rs).

#include <zmq.h>

#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <memory>
#include <set>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include "eb_core.hpp"
#include "event_def.hpp"
#include "hist_config.hpp"
#include "sink_core.hpp"
#include "time_offsets.hpp"

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
// HTTP-command request flags. The /Reset and /ReloadHists buttons are executed
// by Cling on the main thread; each command bakes a COMPILED function pointer
// (see main) that flips one of these flags. The ACTUAL work runs in the main
// loop right after ProcessEvents — so a reload can safely Unregister/delete the
// live histograms without doing it from inside a Cling call frame, and neither
// command bakes histogram pointers that would dangle after a reload.
// ---------------------------------------------------------------------------
static volatile sig_atomic_t g_reset = 0;
static volatile sig_atomic_t g_reload = 0;
static void request_reset() { g_reset = 1; }
static void request_reload() { g_reload = 1; }

// ---------------------------------------------------------------------------
// Built-in monitor histograms — file-scope so the main-loop /Reset handler can
// reach them without a --hists file. In --hists mode these stay null and the
// dynamic LiveHist vector owns the displayed set instead.
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
  std::string tree = "delila";  // same default as delila2root — one macro works on both
  bool tree_given = false;      // --tree seen (warn when the hits tree is off)
  // --hits-tree: write the per-hit tree. Opt-in since TODO 71 — the time series
  // is the .delila's job (delila2root reproduces it any time).
  bool hits_tree = false;
  std::string exp_name;       // --exp-name: explicit override (wins always)
  std::string operator_url;   // --operator: fetch experiment_name from /api/status
  std::string hists_file;     // --hists: declarative histogram set
  // --module M[,M2...]: keep only these digitizers' hits (empty = all boards).
  std::vector<int> modules;
  // Channel roles: "C" (any module) or "M:C" (sink_core.hpp ChSpec, TODO 70
  // Phase 2). Unset = role off.
  ChSpec gamma_ch;
  ChSpec thgem1_ch;
  ChSpec thgem2_ch;
  std::vector<ChSpec> xy1_ch;  // --xy1-ch T,XL,XR,YU,YD (empty = disabled)
  std::vector<ChSpec> xy2_ch;  // --xy2-ch likewise
  double window_ns = 1000.0;
  double margin_ns = 10000.0;
  int http_port = 8090;
  int dt_bins = 2000;
  double dt_min = -1000.0;
  double dt_max = 1000.0;
  int autosave_sec = 30;
  // Pipeline (TODO 66 §5): worker pool size and the Sorter's data-time knobs.
  int workers = 3;
  double chunk_span_ms = 100.0;    // emit a chunk per this much DATA time
  double safe_horizon_ms = 50.0;   // arrival-disorder absorption (>= max disorder)
  // --time-calib FILE: per-(module,channel) time offsets (timeSettings.json),
  // re-read at every run start (empty = off, timestamps recorded raw).
  std::string time_calib;
  // --event-def FILE: generic event definition (event_def.hpp) -> the "events"
  // tree; re-read at every run start like --time-calib (empty = off).
  std::string event_def;
};

// The generic events tree has a FIXED name: analysis macros and the UserInfo
// lookup ("eventDef") then work on every run file without a parameter.
static const char* const kEventsTreeName = "events";

static void print_usage(const char* argv0) {
  std::printf(
      "usage: %s [options]\n"
      "  --zmq ADDR          merger PUB endpoint (default tcp://localhost:5557)\n"
      "  --out-dir DIR       output directory for run*.root (default .)\n"
      "  --hits-tree         write the per-hit tree (5 scalars per hit, time sorted).\n"
      "                      OPT-IN since TODO 71 (the .delila holds the time series)\n"
      "  --tree NAME         hits tree name (default delila; needs --hits-tree)\n"
      "  --exp-name NAME     experiment name for the output filename (override)\n"
      "  --operator URL      operator base URL, e.g. http://localhost:9092; the\n"
      "                      experiment name is read from <URL>/api/status at run\n"
      "                      start (used only when --exp-name is not given)\n"
      "  --hists FILE        histogram definition JSON (see README); replaces the\n"
      "                      built-in dt1/dt2/dt2_vs_dt1/channels set\n"
      "  --module M[,M2...]  process only hits of these digitizers (modules) —\n"
      "                      trees, histograms, matchers and events alike\n"
      "                      (default: all). Needed when boards are NOT clock-\n"
      "                      synchronized (the Sorter and the matchers assume\n"
      "                      one time base)\n"
      "  Channels below are C or M:C (module:channel, 0..255 each). A bare C\n"
      "  matches channel C on ANY module — fine with one board or one --module;\n"
      "  with several boards give M:C (a bare channel seen on two modules in a\n"
      "  run is warned once)\n"
      "  --gamma-ch C        gamma detector channel (enables Δt monitor)\n"
      "  --thgem1-ch C       ThGEM1 channel\n"
      "  --thgem2-ch C       ThGEM2 channel\n"
      "                      (all three required; if any omitted -> recorder only)\n"
      "  --xy1-ch T,XL,XR,YU,YD  delay-line XY monitor 1: trigger + 4 arm channels\n"
      "  --xy2-ch T,XL,XR,YU,YD  delay-line XY monitor 2\n"
      "                      (5 channels that cannot match the same hit; fills\n"
      "                       pos1/pos2-scope histograms, so --hists is required)\n"
      "  --window-ns X       coincidence half-window (default 1000)\n"
      "  --margin-ns X       out-of-order tolerance / ripen delay (default 10000)\n"
      "  --http-port N       THttpServer port, 0 disables (default 8090)\n"
      "  --dt-bins N         Δt histogram bins (default 2000)\n"
      "  --dt-min X          Δt axis min ns (default -1000)\n"
      "  --dt-max X          Δt axis max ns (default 1000)\n"
      "  --autosave-sec N    TTree AutoSave interval (default 30)\n"
      "  --workers N         event-build worker threads (default 3)\n"
      "  --chunk-span-ms X   sorter chunk span in data time (default 100)\n"
      "  --safe-horizon-ms X sorter disorder absorption (default 50; must\n"
      "                      exceed the stream's max arrival disorder)\n"
      "  --time-calib FILE   per-(module,channel) time offsets: timeSettings.json\n"
      "                      tree form (or legacy time-calib output); aligned =\n"
      "                      raw - offset_ns for every consumer (hits tree, events,\n"
      "                      monitors). Re-read at every run start, frozen\n"
      "                      per run; recorded in TTree UserInfo + _timeSettings.json\n"
      "  --event-def FILE    generic event definition JSON (triggers, group, window;\n"
      "                      see README) -> writes the \"events\" tree (one entry per\n"
      "                      trigger-anchored event, hit arrays). Re-read at every\n"
      "                      run start, frozen per run; recorded in the events tree\n"
      "                      UserInfo + _eventDef.json\n"
      "                      At least one of --hits-tree / --event-def is required\n"
      "                      (otherwise nothing would be recorded). The ThGEM built\n"
      "                      tree is retired: thgem_events.C rebuilds it from events\n"
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
      opt.tree_given = true;
    } else if (a == "--hits-tree") {
      opt.hits_tree = true;  // a plain flag: takes no value
    } else if (a == "--exp-name") {
      if (!(v = need(i))) return false;
      opt.exp_name = v;
    } else if (a == "--operator") {
      if (!(v = need(i))) return false;
      opt.operator_url = v;
    } else if (a == "--hists") {
      if (!(v = need(i))) return false;
      opt.hists_file = v;
    } else if (a == "--module") {
      if (!(v = need(i))) return false;
      std::string why;
      if (!parse_module_list(v, opt.modules, &why)) {
        std::fprintf(stderr, "root_sink: --module: %s\n", why.c_str());
        return false;
      }
    } else if (a == "--gamma-ch" || a == "--thgem1-ch" || a == "--thgem2-ch") {
      if (!(v = need(i))) return false;
      ChSpec& dst = a == "--gamma-ch" ? opt.gamma_ch
                    : a == "--thgem1-ch" ? opt.thgem1_ch : opt.thgem2_ch;
      // "-1" = off, the old documented default; anything else must be a
      // channel (std::atoi used to turn junk silently into channel 0).
      std::string why;
      if (std::string(v) == "-1") {
        dst = ChSpec();
      } else if (!parse_ch_spec(v, dst, &why)) {
        std::fprintf(stderr, "root_sink: %s: %s\n", a.c_str(), why.c_str());
        return false;
      }
    } else if (a == "--xy1-ch" || a == "--xy2-ch") {
      if (!(v = need(i))) return false;
      std::string why;
      if (!parse_ch_list(v, 5, a == "--xy1-ch" ? opt.xy1_ch : opt.xy2_ch, &why)) {
        std::fprintf(stderr,
                     "root_sink: %s wants 5 channels 'T,XL,XR,YU,YD' (each C or M:C, "
                     "0..255) that cannot match the same hit — %s (got '%s')\n",
                     a.c_str(), why.c_str(), v);
        return false;
      }
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
    } else if (a == "--workers") {
      if (!(v = need(i))) return false;
      opt.workers = std::atoi(v);
      if (opt.workers < 1) opt.workers = 1;
    } else if (a == "--chunk-span-ms") {
      if (!(v = need(i))) return false;
      opt.chunk_span_ms = std::atof(v);
    } else if (a == "--safe-horizon-ms") {
      if (!(v = need(i))) return false;
      opt.safe_horizon_ms = std::atof(v);
    } else if (a == "--built-tree") {
      // Retired in TODO 71 — refuse loudly so an old TOML / command line
      // cannot silently lose its built tree.
      std::fprintf(stderr,
                   "root_sink: ERROR --built-tree was retired (TODO 71). Record the generic "
                   "events tree instead (--event-def, TOML event_def_file, e.g. "
                   "config/event_def_thgem.json) and rebuild the ThGEM rows offline with "
                   "tools/root_sink/thgem_events.C (same branches as the old built tree)\n");
      return false;
    } else if (a == "--time-calib") {
      if (!(v = need(i))) return false;
      opt.time_calib = v;
    } else if (a == "--event-def") {
      if (!(v = need(i))) return false;
      opt.event_def = v;
    } else {
      std::fprintf(stderr, "root_sink: unknown argument '%s' (try --help)\n", a.c_str());
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
    std::printf("root_sink: experiment_name = \"%s\" (source: --exp-name)\n",
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
        std::printf("root_sink: experiment_name = \"%s\" (source: %s/api/status)\n",
                    name.c_str(), opt.operator_url.c_str());
        return name;
      }
      std::fprintf(stderr,
                   "root_sink: WARNING operator /api/status had no experiment_name; "
                   "using \"data\"\n");
      return "data";
    }
    std::fprintf(stderr,
                 "root_sink: WARNING could not fetch experiment_name from %s (%s); "
                 "using \"data\"\n",
                 opt.operator_url.c_str(), err.c_str());
    return "data";
  }
  std::fprintf(stderr,
               "root_sink: WARNING no --exp-name/--operator given; output uses "
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

class Recorder {
 public:
  bool is_open() const { return file_ != nullptr; }
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
    provisional_ = dir + "/run_inprogress_" + std::to_string((long long)::time(nullptr)) + ".root";
    file_ = TFile::Open(provisional_.c_str(), "RECREATE", "", kDelilaCompression);
    if (!file_ || file_->IsZombie()) {
      std::fprintf(stderr, "root_sink: ERROR cannot create %s\n", provisional_.c_str());
      delete file_;
      file_ = nullptr;
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

  // One generic event -> one entry. EventID is a per-run counter from 0 (it
  // keeps counting across ROOT rollover parts). Never truncates: an event with
  // more hits than the buffers hold grows them first.
  void fill_event(const eb::GenericEvent& e) {
    if (!events_) return;
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
                     "root_sink: WARNING event %lld carries %zu L2 counter value(s), the run's "
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
    events_->Fill();
    ++events_entries_;
  }

  // Flush the current tree(s) so the in-progress file is openable in ROOT while
  // the run is still going (SaveSelf writes tree + keys without closing).
  void autosave() {
    for (TTree* t : {tree_, events_})
      if (t) t->AutoSave("SaveSelf");
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
      std::fprintf(stderr, "root_sink: WARNING could not rename %s -> %s (%s); file kept\n",
                   provisional_.c_str(), final.c_str(), std::strerror(errno));
      final = provisional_;
    }
    // Rename any ROOT-made continuation parts (verified naming: <stem>_k.root).
    std::string stem = provisional_.substr(0, provisional_.size() - 5);  // drop ".root"
    for (int k = 1; k <= parts; ++k) {
      std::string part = stem + "_" + std::to_string(k) + ".root";
      std::string part_final = pick_final_name(run, k);
      if (std::rename(part.c_str(), part_final.c_str()) != 0)
        std::fprintf(stderr, "root_sink: WARNING could not rename %s -> %s (%s); file kept\n",
                     part.c_str(), part_final.c_str(), std::strerror(errno));
      else
        std::printf("root_sink: rollover part %d -> %s\n", k, part_final.c_str());
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
    std::printf("root_sink: shutdown mid-run — kept provisional %s (%s, NOT finalized)\n",
                provisional_.c_str(), counts.c_str());
    if (parts > 0)
      std::printf("root_sink: (plus %d ROOT rollover part(s) kept as %s_*.root)\n", parts,
                  provisional_.substr(0, provisional_.size() - 5).c_str());
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
                     "root_sink: ERROR events-tree branch \"%s\" is missing from "
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
                     "root_sink: ERROR L2 name \"%s\" clashes with an existing events-tree "
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
  int close_all_parts() {
    TTree* lead = tree_ ? tree_ : events_;
    // lead is never null in practice (main() refuses to start without a
    // tree); without one there was no rollover either, so file_ is current.
    TFile* cur = lead ? lead->GetCurrentFile() : file_;
    bool rolled = cur && provisional_ != cur->GetName();
    int parts = 0;
    if (rolled) {
      // Count <stem>_1.root ... — earlier parts were already closed by ROOT.
      std::string stem = provisional_.substr(0, provisional_.size() - 5);
      while (path_exists(stem + "_" + std::to_string(parts + 1) + ".root")) ++parts;
      std::fprintf(stderr,
                   "root_sink: WARNING tree crossed MaxTreeSize — ROOT split the run into "
                   "%d extra part(s); renaming them with Recorder-style sequence numbers\n",
                   parts);
    }
    cur->cd();
    for (TTree* t : {tree_, events_})
      if (t) t->Write();  // all trees live in the same (current) file
    cur->Close();
    delete cur;  // == file_ only in the un-rolled case; file_ may be dangling
    file_ = nullptr;
    tree_ = nullptr;
    events_ = nullptr;
    br_hit_mod_ = br_hit_ch_ = br_hit_e_ = br_hit_es_ = br_hit_t_ = br_hit_trig_ = nullptr;
    br_hit_ac_ = nullptr;
    return parts;
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
  long long entries_ = 0;
  long long events_entries_ = 0;
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

// ---------------------------------------------------------------------------
// Declarative histograms (--hists)
// ---------------------------------------------------------------------------
struct LiveHist {
  HistDef def;
  TH1* h = nullptr;  // TH1D or TH2D; not owned by any TFile (AddDirectory off)
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

// The draw option to advertise for `d`: the explicit one, else "colz" for 2D,
// else empty (leave a 1D to JSROOT's default).
static std::string effective_drawopt(const HistDef& d) {
  if (!d.drawopt.empty()) return d.drawopt;
  return d.is2d ? std::string("colz") : std::string();
}

// Create + Register a TH1D/TH2D per def, set _drawopt, and collect LiveHists.
// Names are validated ROOT-safe (non-empty, no '/') by parse_hist_config.
static void build_live_hists(const std::vector<HistDef>& defs, THttpServer* server,
                             std::vector<LiveHist>& out) {
  out.clear();
  out.reserve(defs.size());
  for (const auto& d : defs) {
    TH1* h = nullptr;
    if (d.is2d) {
      h = new TH2D(d.name.c_str(), d.title.c_str(), d.xbins, d.xmin, d.xmax,
                   d.ybins, d.ymin, d.ymax);
    } else {
      h = new TH1D(d.name.c_str(), d.title.c_str(), d.xbins, d.xmin, d.xmax);
    }
    server->Register("/", h);
    std::string dopt = effective_drawopt(d);
    if (!dopt.empty())
      server->SetItemField((std::string("/") + d.name).c_str(), "_drawopt", dopt.c_str());
    out.push_back({d, h});
  }
}

// Fill every hit-scope histogram for one decoded event.
static void fill_hit_hists(std::vector<LiveHist>& live, const ScalarHit& h) {
  for (auto& lh : live) {
    if (lh.def.scope != Scope::Hit) continue;
    if (!pass_cut(lh.def.cut, h)) continue;
    double x = 0.0;
    if (!value_of(lh.def.x, h, x)) continue;
    if (lh.def.is2d) {
      double y = 0.0;
      if (!value_of(lh.def.y, h, y)) continue;
      static_cast<TH2*>(lh.h)->Fill(x, y);
    } else {
      lh.h->Fill(x);
    }
  }
}

// Fill every coinc-scope histogram for one ripe coincidence result.
static void fill_coinc_hists(std::vector<LiveHist>& live, const CoincResult& r) {
  for (auto& lh : live) {
    if (lh.def.scope != Scope::Coinc) continue;
    if (!pass_cut(lh.def.cut, r)) continue;
    double x = 0.0;
    if (!value_of(lh.def.x, r, x)) continue;
    if (lh.def.is2d) {
      double y = 0.0;
      if (!value_of(lh.def.y, r, y)) continue;
      static_cast<TH2*>(lh.h)->Fill(x, y);
    } else {
      lh.h->Fill(x);
    }
  }
}

// Fill every pos-scope histogram of `scope` for one ripe XY result. The SAME
// function serves both PositionMatcher instances: which detector `r` belongs to
// is expressed ONLY by the scope passed here (Pos1 for the xy1 matcher, Pos2 for
// xy2) — value_of computes identical math for xy1_*/xy2_* variables, so this
// filter is what keeps xy1 results out of xy2 histograms. Don't "simplify" it.
static void fill_pos_hists(std::vector<LiveHist>& live, const PosResult& r,
                           Scope scope) {
  for (auto& lh : live) {
    if (lh.def.scope != scope) continue;
    if (!pass_cut(lh.def.cut, r)) continue;
    double x = 0.0;
    if (!value_of(lh.def.x, r, x)) continue;
    if (lh.def.is2d) {
      double y = 0.0;
      if (!value_of(lh.def.y, r, y)) continue;
      static_cast<TH2*>(lh.h)->Fill(x, y);
    } else {
      lh.h->Fill(x);
    }
  }
}

// Snapshot the current --hists file next to the finalized ROOT file, named
// "<root path minus .root>_hists.json". By deriving from the (possibly
// collision-suffixed) final path, the copy stays paired with its run file.
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
    std::fprintf(stderr, "root_sink: WARNING could not write %s\n", dst.c_str());
    return "";
  }
  out << text;
  return dst;
}

static void copy_hists_sidecar(const std::string& hists_file,
                               const std::string& root_path) {
  if (root_path.empty()) return;
  std::string text;
  if (!read_file(hists_file, text)) {
    std::fprintf(stderr, "root_sink: WARNING could not read %s to pair with %s\n",
                 hists_file.c_str(), root_path.c_str());
    return;
  }
  std::string dst = write_text_sidecar(root_path, "_hists.json", text);
  if (!dst.empty()) std::printf("root_sink: histogram config copied -> %s\n", dst.c_str());
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
                   "root_sink: WARNING event def: trigger '%s' channel(s) %s are not on "
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
int main(int argc, char** argv) {
  Options opt;
  bool help = false;
  if (!parse_args(argc, argv, opt, help)) return help ? 0 : 2;

  const bool http_enabled = opt.http_port != 0;
  const bool coinc_enabled = opt.gamma_ch.set() && opt.thgem1_ch.set() && opt.thgem2_ch.set();
  const bool xy1_enabled = opt.xy1_ch.size() == 5;
  const bool xy2_enabled = opt.xy2_ch.size() == 5;

  // The Δt matcher tests its roles in order (gamma, ThGEM1, ThGEM2): two specs
  // that can match the same hit would let the first silently claim it.
  {
    const ChSpec* role[] = {&opt.gamma_ch, &opt.thgem1_ch, &opt.thgem2_ch};
    const char* flag[] = {"--gamma-ch", "--thgem1-ch", "--thgem2-ch"};
    for (int i = 0; i < 3; ++i)
      for (int j = i + 1; j < 3; ++j)
        if (role[i]->overlaps(*role[j])) {
          std::fprintf(stderr,
                       "root_sink: ERROR %s %s and %s %s can match the same hit (a bare C "
                       "matches every module's channel C) — give distinct channels\n",
                       flag[i], ch_spec_name(*role[i]).c_str(), flag[j],
                       ch_spec_name(*role[j]).c_str());
          return 2;
        }
  }

  const bool events_enabled = !opt.event_def.empty();

  // Since TODO 71 every tree is opt-in. An old command line (or TOML) that
  // relied on the hits tree being implicit would otherwise run "fine" and
  // write empty run files — refuse loudly instead.
  if (!opt.hits_tree && !events_enabled) {
    std::fprintf(stderr,
                 "root_sink: ERROR nothing would be recorded: give at least one of\n"
                 "root_sink:   --hits-tree        the per-hit tree (TOML hits_tree = true) —\n"
                 "root_sink:                      OPT-IN since TODO 71 (2026-10), it used to be\n"
                 "root_sink:                      written implicitly\n"
                 "root_sink:   --event-def FILE   the generic events tree (TOML event_def_file)\n");
    return 2;
  }
  if (opt.tree_given && !opt.hits_tree)
    std::fprintf(stderr,
                 "root_sink: WARNING --tree %s given without --hits-tree — the hits tree is "
                 "opt-in since TODO 71 and will NOT be written\n",
                 opt.tree.c_str());
  // Trees share one file and are looked up by name: a clash would make one
  // shadow the other (and macros would read the wrong tree).
  {
    std::vector<std::string> names;
    if (opt.hits_tree) names.push_back(opt.tree);
    if (events_enabled) names.push_back(kEventsTreeName);
    for (std::size_t i = 0; i < names.size(); ++i)
      for (std::size_t j = i + 1; j < names.size(); ++j)
        if (names[i] == names[j]) {
          std::fprintf(stderr,
                       "root_sink: ERROR two trees would both be named \"%s\" (the hits tree "
                       "is --tree, the generic events tree is always \"%s\") — pick a "
                       "distinct --tree\n",
                       names[i].c_str(), kEventsTreeName);
          return 2;
        }
  }

  // --hists only defines monitor histograms, so it needs the HTTP server. Warn
  // and ignore it rather than silently drop it when the server is off.
  bool use_hists = http_enabled && !opt.hists_file.empty();

  // Every channel spec a display consumer uses, labelled for the logs (TODO 70
  // Phase 2): the matcher roles and the hit-scope "channel" cuts of `defs`.
  auto spec_users = [&](const std::vector<HistDef>& defs) {
    std::vector<std::pair<std::string, ChSpec>> u;
    if (http_enabled && coinc_enabled) {  // the matcher lives on the HTTP display
      u.push_back({"--gamma-ch", opt.gamma_ch});
      u.push_back({"--thgem1-ch", opt.thgem1_ch});
      u.push_back({"--thgem2-ch", opt.thgem2_ch});
    }
    static const char* const arm[] = {"T", "XL", "XR", "YU", "YD"};
    for (int k = 0; k < 5; ++k) {  // XY matchers exist only with --hists
      if (use_hists && xy1_enabled) u.push_back({std::string("--xy1-ch ") + arm[k], opt.xy1_ch[k]});
      if (use_hists && xy2_enabled) u.push_back({std::string("--xy2-ch ") + arm[k], opt.xy2_ch[k]});
    }
    for (const HistDef& d : defs)
      if (d.cut.kind == CutKind::Channel)
        u.push_back({"hist '" + d.name + "' channel cut", d.cut.channel_spec()});
    return u;
  };
  // A spec naming a module that --module drops can never match: say so.
  auto warn_dropped_specs = [&](const std::vector<HistDef>& defs) {
    for (const auto& u : spec_users(defs))
      if (!u.second.bare() && !module_kept(opt.modules, u.second.module))
        std::fprintf(stderr,
                     "root_sink: WARNING %s %s names module %d, which --module %s drops — "
                     "it can never match\n",
                     u.first.c_str(), ch_spec_name(u.second).c_str(), u.second.module,
                     module_list_str(opt.modules).c_str());
  };
  if (!opt.hists_file.empty() && !http_enabled)
    std::fprintf(stderr,
                 "root_sink: WARNING --hists %s ignored because --http-port 0 "
                 "(no monitor server)\n",
                 opt.hists_file.c_str());

  // The XY monitors have no built-in fallback histograms: their results land
  // only in pos1/pos2-scope --hists entries. A channel list without --hists
  // would run the matcher into a void — warn loudly and disable instead.
  if ((xy1_enabled || xy2_enabled) && !use_hists)
    std::fprintf(stderr,
                 "root_sink: WARNING --xy1-ch/--xy2-ch given but XY histograms "
                 "only exist via --hists (pos1/pos2 scope) — XY monitor DISABLED\n");

  // Parse the histogram definitions up front — ANY error is fatal at startup.
  std::vector<HistDef> hist_defs;
  if (use_hists) {
    std::string text;
    if (!read_file(opt.hists_file, text)) {
      std::fprintf(stderr, "root_sink: ERROR cannot read --hists file %s\n",
                   opt.hists_file.c_str());
      return 2;
    }
    ParseResult pr = parse_hist_config(text);
    if (!pr.errors.empty()) {
      std::fprintf(stderr, "root_sink: %zu error(s) in %s:\n", pr.errors.size(),
                   opt.hists_file.c_str());
      for (const auto& e : pr.errors)
        std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
      return 2;
    }
    hist_defs = std::move(pr.defs);
    bool has_coinc = false, has_pos1 = false, has_pos2 = false;
    for (const auto& d : hist_defs) {
      if (d.scope == Scope::Coinc) has_coinc = true;
      if (d.scope == Scope::Pos1) has_pos1 = true;
      if (d.scope == Scope::Pos2) has_pos2 = true;
    }
    if (has_coinc && !coinc_enabled)
      std::fprintf(stderr,
                   "root_sink: WARNING --hists has coincidence histograms but the "
                   "Δt matcher is DISABLED (need --gamma-ch/--thgem1-ch/--thgem2-ch) "
                   "— those histograms will stay empty\n");
    if (has_pos1 && !xy1_enabled)
      std::fprintf(stderr,
                   "root_sink: WARNING --hists has pos1 (xy1_*) histograms but "
                   "--xy1-ch is not set — those histograms will stay empty\n");
    if (has_pos2 && !xy2_enabled)
      std::fprintf(stderr,
                   "root_sink: WARNING --hists has pos2 (xy2_*) histograms but "
                   "--xy2-ch is not set — those histograms will stay empty\n");
  }

  // --time-calib: load + validate up front — ANY error is fatal at startup
  // (at a later run start a bad edit keeps the previous table, STALE).
  const bool toff_enabled = !opt.time_calib.empty();
  const double safe_horizon_ns = opt.safe_horizon_ms * 1e6;
  ActiveOffsets toff_boot;
  if (toff_enabled) {
    std::vector<std::string> errs;
    if (!load_offsets_file(opt.time_calib, toff_boot, safe_horizon_ns, errs)) {
      std::fprintf(stderr, "root_sink: %zu error(s) in --time-calib %s:\n", errs.size(),
                   opt.time_calib.c_str());
      for (const auto& e : errs) std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
      return 2;
    }
  }

  // --event-def: same policy — ANY error is fatal at startup (later run-start
  // reload failures keep the previous definition, STALE). The lookback bound
  // is the safe horizon, like the offsets' spread: context the Sorter carries
  // back must stay well inside what it holds anyway.
  ActiveEventDef evdef_boot;
  if (events_enabled) {
    std::vector<std::string> errs;
    if (!load_event_def_file(opt.event_def, evdef_boot, safe_horizon_ns, errs)) {
      std::fprintf(stderr, "root_sink: %zu error(s) in --event-def %s:\n", errs.size(),
                   opt.event_def.c_str());
      for (const auto& e : errs) std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
      return 2;
    }
  }

  std::printf("root_sink: sub=%s out-dir=%s hits tree=%s\n", opt.zmq.c_str(),
              opt.out_dir.c_str(), opt.hits_tree ? opt.tree.c_str() : "OFF (opt-in: --hits-tree)");
  if (toff_enabled) {
    std::printf("root_sink: time offsets: %s (%s) — aligned_ts = raw_ts - offset_ns for "
                "every consumer; re-read at every run start\n",
                toff_boot.source_path.c_str(), offsets_summary(toff_boot).c_str());
    std::printf("root_sink: time offsets: %s\n", format_offsets_line(toff_boot.table).c_str());
    for (const auto& w : toff_boot.table.warnings)
      std::fprintf(stderr, "root_sink: WARNING time offsets: %s\n", w.c_str());
    if (toff_boot.table.spread_ns() >= opt.margin_ns)
      std::fprintf(stderr,
                   "root_sink: WARNING time offsets: spread %.3f ns >= --margin-ns %.0f — the "
                   "display matchers may ripen before an aligned partner arrives\n",
                   toff_boot.table.spread_ns(), opt.margin_ns);
  } else {
    std::printf("root_sink: time offsets: none (timestamp_ns recorded RAW)\n");
  }
  if (opt.modules.size() == 1)
    std::printf("root_sink: --module %d: only digitizer %d's hits are recorded, histogrammed "
                "and matched (the .delila keeps every board)\n",
                opt.modules[0], opt.modules[0]);
  else if (!opt.modules.empty())
    std::printf("root_sink: --module %s: only these digitizers' hits are recorded, "
                "histogrammed and matched (the .delila keeps every board)\n",
                module_list_str(opt.modules).c_str());
  if (use_hists)
    std::printf("root_sink: histograms defined by %s (%zu hist(s))\n",
                opt.hists_file.c_str(), hist_defs.size());
  if (coinc_enabled)
    std::printf("root_sink: monitor gamma=ch%s thgem1=ch%s thgem2=ch%s window=%.0fns margin=%.0fns\n",
                ch_spec_name(opt.gamma_ch).c_str(), ch_spec_name(opt.thgem1_ch).c_str(),
                ch_spec_name(opt.thgem2_ch).c_str(), opt.window_ns, opt.margin_ns);
  else
    std::printf("root_sink: Δt monitor DISABLED (need --gamma-ch/--thgem1-ch/--thgem2-ch) — recorder only\n");
  auto xy_line = [&](const char* name, const std::vector<ChSpec>& c) {
    std::printf("root_sink: %s monitor trig=ch%s XL=ch%s XR=ch%s YU=ch%s YD=ch%s window=%.0fns "
                "margin=%.0fns\n",
                name, ch_spec_name(c[0]).c_str(), ch_spec_name(c[1]).c_str(),
                ch_spec_name(c[2]).c_str(), ch_spec_name(c[3]).c_str(),
                ch_spec_name(c[4]).c_str(), opt.window_ns, opt.margin_ns);
  };
  if (use_hists && xy1_enabled) xy_line("xy1", opt.xy1_ch);
  if (use_hists && xy2_enabled) xy_line("xy2", opt.xy2_ch);
  warn_dropped_specs(hist_defs);
  // A bare channel matches every module's channel C: right with one board or
  // one --module, a mix with several. Say which specs are bare whenever more
  // than one module can reach them (the display WARNs if it really happens).
  if (opt.modules.size() != 1) {
    std::set<int> chans;
    std::vector<std::string> who;  // flag / hist names, first-seen order
    for (const auto& u : spec_users(hist_defs)) {
      if (!u.second.bare()) continue;
      chans.insert(u.second.channel);
      // "--xy1-ch XL" -> "--xy1-ch": one name per option
      std::string w = u.first.compare(0, 2, "--") == 0 ? u.first.substr(0, u.first.find(' '))
                                                       : u.first;
      if (std::find(who.begin(), who.end(), w) == who.end()) who.push_back(w);
    }
    if (!chans.empty()) {
      std::string cs, ws;
      for (int c : chans) cs += (cs.empty() ? "" : ",") + std::to_string(c);
      for (const auto& w : who) ws += (ws.empty() ? "" : ", ") + w;
      std::printf("root_sink: bare channel(s) %s (%s) match ANY module — fine with one "
                  "digitizer; with several give M:C (a bare channel seen on two modules in a "
                  "run is WARNED once)\n",
                  cs.c_str(), ws.c_str());
    }
  }
  if (events_enabled) {
    std::printf("root_sink: events tree \"%s\": %s (re-read at every run start)\n",
                kEventsTreeName, evdef_boot.source_path.c_str());
    std::printf("root_sink: event def: %s\n", format_event_def_line(*evdef_boot.def).c_str());
    for (const auto& w : evdef_boot.warnings)
      std::fprintf(stderr, "root_sink: WARNING event def: %s\n", w.c_str());
    warn_foreign_triggers(*evdef_boot.def, opt.modules);
  }

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  // ROOT auto-splits a TTree's file when it crosses TTree::GetMaxTreeSize
  // (ROOT 6 default: 100 GB). Raise it to 2 TB: at 14 B/event a scalar run
  // cannot realistically get there (~10^11 events), so a run stays one file.
  // If it IS ever crossed, Recorder::finalize renames the ROOT-made parts
  // with Recorder-style sequence numbers instead of crashing on the TFile*
  // that ChangeFile deletes (see Recorder::close_all_parts).
  // ROOTSINK_TEST_MAX_TREE_SIZE exists so the rollover path can be E2E-tested
  // with a tiny threshold (see README).
#ifdef ROOTSINK_TEST_MAX_TREE_SIZE
  TTree::SetMaxTreeSize(ROOTSINK_TEST_MAX_TREE_SIZE);
#else
  TTree::SetMaxTreeSize(2000000000000LL);  // 2 TB
#endif

  // --- ROOT monitor objects (created before any TFile; detached from gDirectory
  //     so they persist across runs and are never owned/deleted by a run file) ---
  THttpServer* server = nullptr;
  std::vector<LiveHist> live_hists;  // populated only in --hists mode
  if (http_enabled) {
    TH1::AddDirectory(kFALSE);

    // A TApplication must exist for gSystem->ProcessEvents() to drive the HTTP
    // server's timer. It is process-lifetime (never deleted) — don't store it in
    // an unused local. Pass an empty argv so it doesn't try to parse our flags.
    //
    // Batch mode is mandatory for a headless DAQ sink: without it, TApplication
    // connects to $DISPLAY when one is set (e.g. an ssh session), and ROOT's
    // X11 error handler TERMINATES the process when that X connection dies —
    // observed live on side3 (sink killed mid-run when its parent ssh session
    // closed). JSROOT renders client-side, so the HTTP monitor needs no X11.
    gROOT->SetBatch(kTRUE);
    static int fake_argc = 1;
    static char a0[] = "root_sink";
    static char* fake_argv[] = {a0, nullptr};
    new TApplication("root_sink", &fake_argc, fake_argv);

    server = new THttpServer(Form("http:%d", opt.http_port));
    // 2 s JSROOT auto-refresh for the whole tree.
    server->SetItemField("/", "_monitoring", "2000");
    // The sniffer's global-directory scan defaults ON and would expose every
    // open TFile (gROOT->GetListOfFiles()) over HTTP — including the run file
    // the Writer thread is actively filling; a browser click would then read
    // that file from the HTTP thread. Our histograms are Register()ed
    // explicitly, so the scan buys nothing: turn it off. (Verify: no /Files
    // node in the JSROOT hierarchy.)
    server->GetSniffer()->SetScanGlobalDir(kFALSE);

    if (use_hists) {
      build_live_hists(hist_defs, server, live_hists);
      // /ReloadHists re-reads the file live (handled in the main loop). Only
      // registered in --hists mode. Like /Reset it bakes a compiled function
      // pointer, not histogram pointers, so it survives a reload.
      server->RegisterCommand(
          "/ReloadHists",
          Form("((void(*)())%p)();", reinterpret_cast<void*>(&request_reload)),
          "button;Reload histogram definitions");
    } else {
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
      server->Register("/", g_h_dt1);
      server->Register("/", g_h_dt2);
      server->Register("/", g_h_dt2_vs_dt1);
      server->Register("/", g_h_channels);
      server->SetItemField("/dt2_vs_dt1", "_drawopt", "colz");
    }

    // /Reset zeroes whatever histograms are currently live (built-in or dynamic).
    // It flips g_reset, consumed by the main loop; the compiled function pointer
    // keeps the command valid across a reload (no baked histogram pointers to
    // dangle). No -rdynamic needed — the pointer is baked as a literal.
    server->RegisterCommand(
        "/Reset", Form("((void(*)())%p)();", reinterpret_cast<void*>(&request_reset)),
        "button;Reset histograms");
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

  // The two delay-line XY matchers share --window-ns/--margin-ns with the Δt
  // matcher (one disorder tolerance per stream). --hists required: see the
  // startup warning above.
  auto make_xy = [&](const std::vector<ChSpec>& chs) {
    PositionMatcher::Config cfg;
    cfg.trig_ch = chs[0];
    cfg.xl_ch = chs[1];
    cfg.xr_ch = chs[2];
    cfg.yu_ch = chs[3];
    cfg.yd_ch = chs[4];
    cfg.window_ns = opt.window_ns;
    cfg.margin_ns = opt.margin_ns;
    return std::unique_ptr<PositionMatcher>(new PositionMatcher(cfg));
  };
  std::unique_ptr<PositionMatcher> xy1, xy2;
  if (use_hists && xy1_enabled) xy1 = make_xy(opt.xy1_ch);
  if (use_hists && xy2_enabled) xy2 = make_xy(opt.xy2_ch);

  // --- ZMQ SUB (HWM=0 per the data-preservation rule) ---
  // Created + connected on the main thread so a bad endpoint still exits with
  // code 3 before anything spawns, then handed to the Receiver thread. ZMQ
  // permits socket migration between threads as long as use never overlaps —
  // the std::thread constructor provides the required memory barrier, and
  // only the Receiver touches the socket afterwards (it also closes it).
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

  // ROOT is about to run on two threads at once (TH1 fills on this Display
  // thread, TFile/TTree exclusively on the Writer thread) — arm the global
  // locks BEFORE any of them starts.
  ROOT::EnableThreadSafety();

  // --- Cross-thread plumbing (TODO 66 §5) ----------------------------------
  // Ownership map: Receiver owns the socket + RunState; Sorter owns the
  // disorder buffer + seq counter; Workers are stateless; Writer owns the
  // Recorder (ALL TFile/TTree access); Display (this thread) owns every TH1 +
  // the THttpServer. The atomic counters are the only shared mutable state.
  struct Counters {
    std::atomic<long long> received{0};       // decoded hits (Receiver)
    std::atomic<long long> written{0};        // hits filled to the tree (Writer)
    std::atomic<long long> chunks{0};         // Sorter emissions
    std::atomic<long long> matcher_fills{0};  // Display
    std::atomic<long long> xy_fills{0};       // Display
    std::atomic<long long> display_drops{0};  // tee overflow (counted + logged)
    std::atomic<long long> other_module{0};   // hits dropped by --module (Receiver)
    std::atomic<long long> late{0};           // hits behind the emitted core (Sorter)
    std::atomic<bool> writing{false};         // stream run state (status line)
    std::atomic<int> toff_entries{-1};        // time offsets in force (-1 = off)
    std::atomic<bool> toff_stale{false};      // last run-start reload failed
    std::atomic<long long> toff_unmapped{0};  // keys without an entry, this run
    // Generic events (Writer), cumulative over the process. contested / orphans are
    // the overlap watch of TODO 71 §4: a rising contested share = windows
    // overlapping more (beam intensity) — visible here before anyone analyses.
    std::atomic<long long> events{0};
    std::atomic<long long> ev_contested{0};
    std::atomic<long long> ev_orphans{0};
    std::atomic<bool> evdef_stale{false};     // last run-start reload failed
    // Phase C (cumulative too): AC-flagged hits, events with L2Accept. Shown
    // only while the current run's definition has AC / an accept.
    std::atomic<long long> ev_ac{0};
    std::atomic<long long> ev_l2acc{0};
    std::atomic<bool> run_has_ac{false};
    std::atomic<bool> run_has_l2acc{false};
  } counters;
  if (toff_enabled) counters.toff_entries = static_cast<int>(toff_boot.table.abs_ns.size());

  const int n_workers = opt.workers;

  eb::Channel<eb::SorterMsg> to_sorter;  // record path: unbounded, never drops
  eb::Channel<eb::WorkMsg> work_q;       // record path: unbounded
  eb::Channel<eb::WorkMsg> writer_q;     // record path: unbounded
  // Display tee: bounded; DATA may drop (counted — the CLAUDE.md Monitor
  // exception; the record path above is untouched). Control markers use the
  // BLOCKING push instead: a dropped RunOpen would leave stale matcher clocks
  // across a run boundary.
  eb::Channel<eb::DisplayMsg> display_q(1000);

  // --- Receiver: ZMQ recv + decode + run state -> in-band markers ----------
  auto receiver_loop = [&] {
    RunState run_state;
    std::set<std::string> warned;
    // Time offsets in force — owned by this thread from here on. Re-read at
    // every RunOpen and frozen for the run (SPEC §4.3.1).
    ActiveOffsets toff = toff_boot;
    std::set<uint16_t> toff_reported;
    // Run-end summary of the (module, channel) keys this run had no offset for
    // (each was WARNed once when first seen). Log only, by choice: the run's
    // provenance already holds the exact table applied, so "unmapped" = any key
    // present in the trees but absent from resolved_abs_ns, derivable offline.
    auto report_unmapped = [&](const std::string& what) {
      if (!toff_enabled || toff_reported.empty()) return;
      std::string keys;
      for (uint16_t k : toff_reported) keys += (keys.empty() ? "" : " ") + mc_name(k);
      std::printf("root_sink: %s time offsets: %zu (module, channel) key(s) had no entry and "
                  "stayed RAW: %s\n",
                  what.c_str(), toff_reported.size(), keys.c_str());
    };
    // Event definition in force — same ownership and per-run freeze. The
    // shared_ptr<const EventDef> handed to the Sorter is never mutated: a
    // reload adopts a NEW object (reload_event_def).
    ActiveEventDef evdef = evdef_boot;
    auto process = [&](const uint8_t* data, size_t size) {
      Envelope env = parse_envelope(data, size);
      switch (env.kind) {
        case MsgKind::Data: {
          // Decode into OWNED hits before the zmq_msg buffer dies (the
          // envelope payload borrows it) and before crossing any thread.
          uint32_t sid = 0;
          std::vector<ScalarHit> hits;
          long n = decode_batch_into(env.payload, env.payload_size, sid, hits);
          if (n < 0) {
            std::fprintf(stderr, "root_sink: WARNING malformed Data batch (skipped)\n");
            break;
          }
          run_state.on_data(sid, [&] {
            counters.writing = true;
            eb::SorterMsg m;
            m.ctrl = eb::Ctrl::RunOpen;
            if (toff_enabled) {
              bool was_stale = toff.stale;
              // The previous run's table (the startup load before the first
              // run): an edit between runs is logged per channel, so a moved
              // image or TOF peak can be traced to the run that changed it.
              const OffsetTable before = toff.table;
              std::vector<std::string> errs;
              if (!load_offsets_file(opt.time_calib, toff, safe_horizon_ns, errs)) {
                std::fprintf(stderr,
                             "root_sink: ERROR time offsets reload failed at run start — "
                             "KEEPING THE PREVIOUS TABLE for this run:\n");
                for (const auto& e : errs) std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
              } else {
                if (was_stale) std::printf("root_sink: time offsets reloaded OK (was STALE)\n");
                const std::string diff = offsets_diff(before, toff.table);
                if (!diff.empty())
                  std::printf("root_sink: time offsets CHANGED vs the previous run: %s\n",
                              diff.c_str());
              }
              toff_reported.clear();
              counters.toff_unmapped = 0;
              counters.toff_stale = toff.stale;
              counters.toff_entries = static_cast<int>(toff.table.abs_ns.size());
              std::printf("root_sink: run time offsets [%s%s]: %s\n",
                          offsets_summary(toff).c_str(), toff.stale ? ", STALE" : "",
                          format_offsets_line(toff.table).c_str());
              for (const auto& w : toff.table.warnings)
                std::fprintf(stderr, "root_sink: WARNING time offsets: %s\n", w.c_str());
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
                             "root_sink: ERROR event definition reload failed at run start — "
                             "KEEPING THE PREVIOUS DEFINITION for this run:\n");
                for (const auto& e : errs) std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
              } else if (was_stale) {
                std::printf("root_sink: event definition reloaded OK (was STALE)\n");
              }
              counters.evdef_stale = evdef.stale;
              // evdef.def is never null here: the startup load succeeded and a
              // failed reload keeps the previous definition.
              std::printf("root_sink: run event def [%s%s]: %s\n", evdef.source_path.c_str(),
                          evdef.stale ? ", STALE" : "",
                          format_event_def_line(*evdef.def).c_str());
              // Warnings of the definition IN FORCE (kept with it on a STALE
              // run), every run — like the time-offset warnings above.
              for (const auto& w : evdef.warnings)
                std::fprintf(stderr, "root_sink: WARNING event def: %s\n", w.c_str());
              warn_foreign_triggers(*evdef.def, opt.modules);
              m.event_def = evdef.def;
              m.prov.event_def_meta = event_def_provenance_json(evdef);
              m.prov.event_def_source = evdef.source_text;
            }
            to_sorter.push(std::move(m));
            eb::DisplayMsg d;
            d.ctrl = eb::Ctrl::RunOpen;
            display_q.push(std::move(d));  // blocking: control is never dropped
            std::printf("root_sink: run started (source %u)\n", sid);
            std::fflush(stdout);
          });
          // --module, then time alignment: both before ANY consumer. The Sorter
          // and the watermark matchers assume one time base (sink_core.hpp), so
          // unsynchronized boards go here; aligning here puts the Sorter, the
          // hits tree, the event layer and the display matchers on ONE aligned
          // axis (time_offsets.hpp). The Sorter sorts on timestamp_ns, so this
          // must never move after it.
          std::size_t decoded = hits.size();
          keep_modules(hits, opt.modules);
          counters.other_module += static_cast<long long>(decoded - hits.size());
          if (toff_enabled)
            apply_time_offsets(hits, toff.table, toff_reported, [&](uint16_t k) {
              ++counters.toff_unmapped;
              std::fprintf(stderr,
                           "root_sink: WARNING time offsets: module %u channel %u has no entry "
                           "in %s — recorded with offset 0 (reported once per run)\n",
                           static_cast<unsigned>(k >> 8), static_cast<unsigned>(k & 0xFF),
                           toff.source_path.c_str());
            });
          counters.received += static_cast<long long>(hits.size());
          if (http_enabled) {
            eb::DisplayMsg d;
            d.hits = hits;  // tee copy; the original moves to the Sorter below
            if (!display_q.try_push(std::move(d))) ++counters.display_drops;
          }
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
                eb::DisplayMsg d;
                d.ctrl = eb::Ctrl::RunClose;
                display_q.push(std::move(d));
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
            std::fprintf(stderr,
                         "root_sink: WARNING unknown message variant '%s' (skipping; further ones silenced)\n",
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
    }
    if (run_state.is_writing()) report_unmapped("unfinalized run");
    // Orderly teardown: one Shutdown down each path. The Sorter fans it out
    // to the workers (after flushing); the Display loop exits on its copy.
    eb::SorterMsg m;
    m.ctrl = eb::Ctrl::Shutdown;
    to_sorter.push(std::move(m));
    eb::DisplayMsg d;
    d.ctrl = eb::Ctrl::Shutdown;
    display_q.push(std::move(d));
    zmq_close(sub);  // closed on the thread that used it
  };

  // --- Sorter: disorder buffer -> sorted chunks with sequence numbers ------
  auto sorter_loop = [&] {
    eb::Sorter::Config scfg;
    scfg.chunk_span_ns = opt.chunk_span_ms * 1e6;
    scfg.safe_horizon_ns = opt.safe_horizon_ms * 1e6;
    scfg.lookback_ns = opt.window_ns;  // context carry >= coincidence window
    eb::Sorter sorter(scfg);
    // Generic events, rules 1-2 (TODO 71 §5): trigger grouping is sequential
    // over the whole run, so it lives here, on the one thread that sees every
    // chunk in order; the Workers then only do the local rules 3-4.
    eb::TriggerGrouper grouper;
    uint64_t seq = 0;
    // No silent loss: hits that arrived behind an emitted core (disorder beyond
    // --safe-horizon-ms) are in none of the trees (hits, events).
    // Reported once per run (RunClose, or Shutdown of a still-open run).
    uint64_t reported_late = 0;
    auto report_late = [&](const eb::Sorter& so) {
      counters.late = static_cast<long long>(so.late_hits());
      if (so.late_hits() > reported_late)
        std::fprintf(stderr,
                     "root_sink: WARNING %llu hit(s) this run arrived more than %.0f ms behind "
                     "the newest hit and are MISSING from every tree of the .root file "
                     "(the .delila has them). Unsynchronized digitizers? Use --module, or "
                     "raise --safe-horizon-ms.\n",
                     static_cast<unsigned long long>(so.late_hits()), opt.safe_horizon_ms);
      reported_late = so.late_hits();
    };
    // EVERY emitted chunk passes through here — data splits and the RunClose /
    // Shutdown flush (core_end = +inf) alike. The grouper is stateful (open
    // groups carry across chunk boundaries) and must see them all, in emission
    // order: the flush chunk is what closes the run's last groups.
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
          counters.late = static_cast<long long>(sorter.late_hits());  // status line
          break;
        case eb::Ctrl::RunOpen: {
          sorter.reset();  // R11: forget the previous run's clock, or the new
                           // run's hits all fall below core_start and vanish
          // The context carried across a chunk boundary is what the generic
          // layer needs: G + 2*pre + post, plus ac_pre + ac_post with
          // anti_compton (EventDef::lookback_ns, eb_core.hpp section 7). Per
          // run, since the event definition is re-read at every RunOpen. No
          // definition = hits tree only, which needs no context (core_range).
          const double prev_lookback = sorter.lookback_ns();
          const double lookback = in.event_def ? in.event_def->lookback_ns() : 0.0;
          sorter.set_lookback_ns(lookback);
          grouper.reset(in.event_def);  // null = generic events off for this run
          if (lookback != prev_lookback)
            std::printf("root_sink: sorter lookback %.0f ns (event def "
                        "G+2*pre+post[+ac_pre+ac_post])\n",
                        lookback);
          reported_late = 0;
          eb::WorkMsg m;
          m.ctrl = eb::Ctrl::RunOpen;
          m.seq = seq++;
          m.prov = std::move(in.prov);  // time-offset record -> Writer (UserInfo)
          // The run's frozen definition -> Writer: it names the events tree's
          // L2 branches and the run summary's flag/accept counts.
          m.event_def = in.event_def;
          work_q.push(std::move(m));
          break;
        }
        case eb::Ctrl::RunClose: {
          sorter.flush(emit_chunk);  // the run's tail becomes the last chunk
          report_late(sorter);
          eb::WorkMsg m;
          m.ctrl = eb::Ctrl::RunClose;
          m.seq = seq++;
          m.run_number = in.run_number;
          work_q.push(std::move(m));
          break;
        }
        case eb::Ctrl::Shutdown: {
          // Flush BEFORE the pills: a mid-run shutdown's buffered tail must
          // still reach the file (close_unfinalized keeps it). The pills then
          // carry the highest seqs, so the Writer drains all real work first.
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
      // Rules 3-4 of the generic layer: a pure function of the chunk and the
      // plan the Sorter attached (plan.def null = off for this run).
      if (m.ctrl == eb::Ctrl::None && m.plan.def)
        m.events = eb::build_generic_events(m.chunk, m.plan, m.gstats);
      bool shutdown = (m.ctrl == eb::Ctrl::Shutdown);
      writer_q.push(std::move(m));
      if (shutdown) return;  // exactly one pill per worker
    }
  };

  // --- Writer: seq-ordered ROOT I/O (the ONLY thread touching the run file) -
  auto writer_loop = [&] {
    Recorder recorder;
    eb::SeqReorder<eb::WorkMsg> reorder;
    int pills = 0;
    auto t_last_autosave = Clock::now();
    bool done = false;
    // Generic-event counters of the current run (summed per chunk), and the
    // number of core-range hits they must account for: hits_in_events +
    // orphans == core hits (GenericStats contract) — checked at run end so a
    // hit the layer lost or doubled is reported, never silent.
    eb::GenericStats run_gstats;
    long long run_core_hits = 0;
    // The run's frozen event definition (RunOpen): names the L2 flags and
    // accepts in the run summary, says whether AC / L2 exist this run.
    std::shared_ptr<const EventDef> run_def;
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
      char ac[64] = "";
      if (run_def && run_def->has_ac())
        std::snprintf(ac, sizeof ac, ", AC-flagged hits %llu",
                      static_cast<unsigned long long>(run_gstats.ac_flagged));
      std::printf("root_sink: %s events: %llu (NTriggers %s), hits in events %llu, "
                  "contested %llu, orphans %llu%s\n",
                  what.c_str(), static_cast<unsigned long long>(run_gstats.events),
                  dist.empty() ? "-" : dist.c_str(),
                  static_cast<unsigned long long>(run_gstats.hits_in_events),
                  static_cast<unsigned long long>(run_gstats.contested),
                  static_cast<unsigned long long>(run_gstats.orphans), ac);
      // L2 (flag-only): events with each flag / accept true, then L2Accept.
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
        std::printf("root_sink: %s L2: %s\n", what.c_str(), l2.c_str());
      }
      const unsigned long long accounted = run_gstats.hits_in_events + run_gstats.orphans;
      if (accounted != static_cast<unsigned long long>(run_core_hits))
        std::fprintf(stderr,
                     "root_sink: WARNING %s: the generic event layer accounted for %llu hits "
                     "(in events + orphans) but the Sorter emitted %lld — a hit was lost or "
                     "doubled (eb_core.hpp section 7 bug; the .delila is complete)\n",
                     what.c_str(), accounted, run_core_hits);
    };
    while (!done) {
      eb::WorkMsg m;
      // Value → process; Timeout → fall through to the autosave tick below;
      // Closed → exit. Orderly shutdown still arrives as in-band pills (the
      // queue is never close()d today), but honoring Closed keeps this loop
      // correct if a future caller terminates via close() — pop_for guarantees
      // drain-then-Closed, so exiting here can never strand queued work.
      const auto pr = writer_q.pop_for(m, 200);
      if (pr == eb::PopResult::Closed) break;
      if (pr == eb::PopResult::Value) {
        reorder.push(m.seq, std::move(m));
        eb::WorkMsg r;
        while (!done && reorder.pop_ready(r)) {
          switch (r.ctrl) {
            case eb::Ctrl::None: {
              // Core range ONLY: chunks deliberately carry duplicated context
              // (lookback head + post-core tail) for the builders; writing the
              // full chunk would duplicate hits in the tree. Built and generic
              // events were already core-gated by their builders.
              auto range = eb::core_range(r.chunk);
              const long long n_core = static_cast<long long>(range.second - range.first);
              if (opt.hits_tree) {
                for (std::size_t i = range.first; i < range.second; ++i)
                  recorder.fill(r.chunk.hits[i]);
                counters.written += n_core;
              }
              if (r.plan.def) {
                for (const eb::GenericEvent& ev : r.events) recorder.fill_event(ev);
                run_gstats.add(r.gstats);
                run_core_hits += n_core;
                counters.events += static_cast<long long>(r.events.size());
                counters.ev_contested += static_cast<long long>(r.gstats.contested);
                counters.ev_orphans += static_cast<long long>(r.gstats.orphans);
                counters.ev_ac += static_cast<long long>(r.gstats.ac_flagged);
                counters.ev_l2acc += static_cast<long long>(r.gstats.l2_accepted);
              }
              break;
            }
            case eb::Ctrl::RunOpen: {
              // exp_name resolves at the run boundary on THIS thread — the
              // bounded (~2 s) HTTP fetch only delays file naming while the
              // unbounded queues upstream absorb the stream.
              std::string exp = resolve_exp_name(opt);
              run_gstats = eb::GenericStats();
              run_core_hits = 0;
              run_def = r.event_def;
              if (events_enabled && !run_def)  // the Receiver always attaches one
                std::fprintf(stderr,
                             "root_sink: ERROR RunOpen without an event definition — no events "
                             "tree this run (bug)\n");
              counters.run_has_ac = run_def && run_def->has_ac();
              counters.run_has_l2acc = run_def && run_def->has_l2() && !run_def->accepts.empty();
              if (!recorder.open_run(opt.out_dir, opt.hits_tree ? opt.tree : std::string(), exp,
                                     run_def, r.prov))
                std::fprintf(stderr,
                             "root_sink: run start FAILED — dropping this run's events\n");
              else
                std::printf("root_sink: run file -> %s\n", recorder.provisional().c_str());
              std::fflush(stdout);
              break;
            }
            case eb::Ctrl::RunClose: {
              // Seq order makes this a barrier for free: every chunk of the
              // run has already been written when the marker pops.
              std::string counts = recorder.entry_counts();  // before finalize closes
              std::string fn = recorder.finalize(r.run_number);
              std::printf("root_sink: run %u finalized -> %s (%s)\n", r.run_number, fn.c_str(),
                          counts.c_str());
              if (events_enabled) report_events("run " + std::to_string(r.run_number));
              if (use_hists) copy_hists_sidecar(opt.hists_file, recorder.final_path());
              // The FROZEN RunOpen snapshots (unlike _hists.json, which re-reads
              // the live file here): a re-loadable timeSettings.json (also for
              // the Rust EB) and the event definition that built this run.
              if (!recorder.provenance().time_offsets_source.empty()) {
                std::string ts = write_text_sidecar(recorder.final_path(), "_timeSettings.json",
                                                    recorder.provenance().time_offsets_source);
                if (!ts.empty()) std::printf("root_sink: time settings snapshot -> %s\n", ts.c_str());
              }
              if (!recorder.provenance().event_def_source.empty()) {
                std::string ed = write_text_sidecar(recorder.final_path(), "_eventDef.json",
                                                    recorder.provenance().event_def_source);
                if (!ed.empty())
                  std::printf("root_sink: event definition snapshot -> %s\n", ed.c_str());
              }
              if (http_enabled)
                std::printf("root_sink: (monitor histograms kept across the run boundary)\n");
              std::fflush(stdout);
              break;
            }
            case eb::Ctrl::Shutdown:
              if (++pills >= n_workers) done = true;  // all real work drained
              break;
          }
        }
      }
      auto now = Clock::now();
      if (recorder.is_open() &&
          std::chrono::duration_cast<std::chrono::seconds>(now - t_last_autosave)
                  .count() >= opt.autosave_sec) {
        recorder.autosave();
        t_last_autosave = now;
      }
    }
    if (recorder.is_open()) {
      recorder.close_unfinalized();
      if (events_enabled) report_events("unfinalized run");
    }
  };

  std::thread th_receiver(receiver_loop);
  std::thread th_sorter(sorter_loop);
  std::vector<std::thread> th_workers;
  for (int i = 0; i < n_workers; ++i) th_workers.emplace_back(worker_loop);
  std::thread th_writer(writer_loop);

  // --- Display (this thread): histograms + matchers + HTTP + status --------
  // Fed by the bounded tee in ARRIVAL order (same as the old single-threaded
  // fill path), so the watermark matchers behave exactly as before.

  // Emit any coincidence results a matcher produced into the histograms. In
  // --hists mode the declarative coinc-scope set decides what gets filled; the
  // built-in mode keeps the hard-coded dt1/dt2/dt2_vs_dt1 fills.
  auto emit = [&](const CoincResult& r) {
    ++counters.matcher_fills;
    if (use_hists) {
      fill_coinc_hists(live_hists, r);
    } else {
      if (r.has_dt1) g_h_dt1->Fill(r.dt1);
      if (r.has_dt2) g_h_dt2->Fill(r.dt2);
      if (r.has_dt1 && r.has_dt2) g_h_dt2_vs_dt1->Fill(r.dt1, r.dt2);
    }
  };

  // XY results route by scope: the xy1 matcher fills pos1 histograms, xy2 fills
  // pos2. `live_hists` is captured by reference and resolved per call, so a
  // /ReloadHists rebuild is safe (nothing bakes a TH1*). xy matchers only exist
  // in --hists mode, so no built-in branch here.
  auto emit_xy1 = [&](const PosResult& r) {
    ++counters.xy_fills;
    fill_pos_hists(live_hists, r, Scope::Pos1);
  };
  auto emit_xy2 = [&](const PosResult& r) {
    ++counters.xy_fills;
    fill_pos_hists(live_hists, r, Scope::Pos2);
  };

  // Bare channel specs (any module) vs the stream: the first time a bare
  // channel number arrives from a second module in a run, the matcher / cut
  // using it starts mixing boards — WARN once per channel per run (TODO 70
  // Phase 2). Rebuilt on /ReloadHists (the cuts may change).
  BareChannelWatch bare_watch;
  std::array<std::string, 256> bare_who;  // channel -> the specs using it bare
  bool bare_watch_on = false;
  auto rebuild_bare_watch = [&](const std::vector<HistDef>& defs) {
    bare_watch.clear_watched();
    for (auto& w : bare_who) w.clear();
    for (const auto& u : spec_users(defs))
      if (u.second.bare()) {
        bare_watch.watch(u.second);
        std::string& w = bare_who[static_cast<std::size_t>(u.second.channel)];
        w += (w.empty() ? "" : ", ") + u.first;
      }
    bare_watch_on = bare_watch.any();
  };
  rebuild_bare_watch(hist_defs);
  auto on_bare_ambiguous = [&](int ch, int m1, int m2) {
    std::fprintf(stderr,
                 "root_sink: WARNING bare channel %d (%s) arrived from modules %d and %d this "
                 "run — a bare channel matches BOTH, so the monitor mixes boards; give M:C "
                 "(e.g. %d:%d) or --module (warned once per channel per run)\n",
                 ch, bare_who[static_cast<std::size_t>(ch)].c_str(), m1, m2, m1, ch);
  };

  bool display_done = false;
  auto handle_display = [&](eb::DisplayMsg& d) {
    switch (d.ctrl) {
      case eb::Ctrl::None:
        if (http_enabled) {
          for (const ScalarHit& h : d.hits) {
            if (bare_watch_on) bare_watch.see(h, on_bare_ambiguous);
            if (use_hists)
              fill_hit_hists(live_hists, h);
            else if (h.channel < 64)
              g_h_channels->Fill(h.channel);
            if (matcher) matcher->push(h, emit);
            if (xy1) xy1->push(h, emit_xy1);
            if (xy2) xy2->push(h, emit_xy2);
          }
        }
        break;
      case eb::Ctrl::RunOpen:
        bare_watch.reset();             // the ambiguity warning is per run
        if (matcher) matcher->reset();  // clock may restart between runs
        if (xy1) xy1->reset();
        if (xy2) xy2->reset();
        break;
      case eb::Ctrl::RunClose:
        if (matcher) matcher->flush(emit);  // ripen the final partial window
        if (xy1) xy1->flush(emit_xy1);
        if (xy2) xy2->flush(emit_xy2);
        break;
      case eb::Ctrl::Shutdown:
        display_done = true;
        break;
    }
  };

  std::printf("root_sink: pipeline receiver -> sorter(span=%.0fms horizon=%.0fms) -> %d worker(s) -> writer\n",
              opt.chunk_span_ms, opt.safe_horizon_ms, n_workers);
  std::printf("root_sink: running. Ctrl-C or SIGTERM to stop.\n");
  std::fflush(stdout);

  auto t_last_status = Clock::now();
  long long last_received = 0;

  while (!display_done) {
    eb::DisplayMsg d;
    auto got = display_q.pop_for(d, 100);
    while (got == eb::PopResult::Value && !display_done) {
      handle_display(d);
      got = display_q.pop_for(d, 0);  // drain whatever queued meanwhile
    }
    // Shutdown normally arrives as an in-band pill (handled above); Closed is
    // honored too so a close()-terminated tee cannot strand this loop. pop_for
    // only reports Closed once drained, so no queued display work is lost.
    if (got == eb::PopResult::Closed) display_done = true;

    // Service HTTP requests on this thread (the histograms' owner, so the
    // no-lock invariant of the single-threaded design carries over 1:1).
    if (server) gSystem->ProcessEvents();

    // Apply any HTTP command that fired during ProcessEvents. Doing the work here
    // (not inside the Cling call) lets a reload safely delete the live objects.
    if (g_reset) {
      g_reset = 0;
      if (use_hists) {
        for (auto& lh : live_hists)
          if (lh.h) lh.h->Reset();
      } else if (http_enabled) {
        if (g_h_dt1) g_h_dt1->Reset();
        if (g_h_dt2) g_h_dt2->Reset();
        if (g_h_dt2_vs_dt1) g_h_dt2_vs_dt1->Reset();
        if (g_h_channels) g_h_channels->Reset();
      }
      std::printf("root_sink: /Reset — histograms zeroed\n");
      std::fflush(stdout);
    }
    if (g_reload) {
      g_reload = 0;  // /ReloadHists is only registered in --hists mode
      std::string text;
      if (!read_file(opt.hists_file, text)) {
        std::fprintf(stderr,
                     "root_sink: /ReloadHists could not read %s — keeping current set\n",
                     opt.hists_file.c_str());
      } else {
        ParseResult pr = parse_hist_config(text);
        if (!pr.errors.empty()) {
          std::fprintf(stderr, "root_sink: /ReloadHists — %zu error(s), keeping current set:\n",
                       pr.errors.size());
          for (const auto& e : pr.errors)
            std::fprintf(stderr, "root_sink:   - %s\n", e.c_str());
        } else {
          for (auto& lh : live_hists) {
            if (lh.h) {
              server->Unregister(lh.h);
              delete lh.h;
            }
          }
          live_hists.clear();
          build_live_hists(pr.defs, server, live_hists);
          warn_dropped_specs(pr.defs);
          rebuild_bare_watch(pr.defs);
          std::printf("root_sink: /ReloadHists — %zu histogram(s) now live\n",
                      live_hists.size());
        }
      }
      std::fflush(stdout);
    }

    // (autosave moved to the Writer thread — the tree's owner.)
    auto now = Clock::now();
    using std::chrono::duration;
    using std::chrono::duration_cast;
    using std::chrono::seconds;
    if (duration_cast<seconds>(now - t_last_status).count() >= 10) {
      double dt = duration_cast<duration<double>>(now - t_last_status).count();
      long long rec = counters.received;
      double rate = dt > 0 ? (double)(rec - last_received) / dt : 0.0;
      // toff=off | N | N STALE | N (k unmapped)
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
      // written=N | hits tree off
      char written_s[48];
      if (opt.hits_tree)
        std::snprintf(written_s, sizeof written_s, "written=%lld", (long long)counters.written);
      else
        std::snprintf(written_s, sizeof written_s, "hits tree off");
      // ev=off | N (cont=C orph=O[ ac=A][ l2acc=L])[ STALE]
      char ev_s[160];
      if (!events_enabled) {
        std::snprintf(ev_s, sizeof ev_s, "off");
      } else {
        char extra[64] = "";
        std::size_t used = 0;
        if (counters.run_has_ac)
          used += std::snprintf(extra + used, sizeof extra - used, " ac=%lld",
                                (long long)counters.ev_ac);
        if (counters.run_has_l2acc && used < sizeof extra)
          std::snprintf(extra + used, sizeof extra - used, " l2acc=%lld",
                        (long long)counters.ev_l2acc);
        std::snprintf(ev_s, sizeof ev_s, "%lld (cont=%lld orph=%lld%s)%s",
                      (long long)counters.events, (long long)counters.ev_contested,
                      (long long)counters.ev_orphans, extra,
                      counters.evdef_stale ? " STALE" : "");
      }
      std::printf(
          "root_sink: %s | events=%lld (%s) | %.0f ev/s | chunks=%lld | "
          "ev=%s | matcher_fills=%lld | xy_fills=%lld | q[sort=%zu work=%zu "
          "write=%zu disp=%zu] | display_drops=%lld | other_module=%lld | late=%lld | "
          "toff=%s\n",
          counters.writing ? "WRITING" : "idle", rec, written_s,
          rate, (long long)counters.chunks, ev_s,
          (long long)counters.matcher_fills, (long long)counters.xy_fills,
          to_sorter.size(), work_q.size(), writer_q.size(), display_q.size(),
          (long long)counters.display_drops, (long long)counters.other_module,
          (long long)counters.late, toff_s);
      std::fflush(stdout);
      t_last_status = now;
      last_received = rec;
    }
  }

  // Shutdown cascade: the Receiver already sent Shutdown down both paths (that
  // is what ended the display loop above); joining in pipeline order lets each
  // stage drain into the next. The Writer's close_unfinalized runs inside its
  // own thread before it exits.
  std::printf("root_sink: stopping...\n");
  std::fflush(stdout);
  th_receiver.join();
  th_sorter.join();
  for (auto& t : th_workers) t.join();
  th_writer.join();

  zmq_ctx_term(ctx);  // the Receiver closed the socket on its way out
  // ROOT objects (histograms, server, app) are process-lifetime; let the OS
  // reclaim them on exit rather than risk teardown-order surprises.
  if (opt.hits_tree)
    std::printf("root_sink: bye (%lld hits received, %lld written).\n",
                (long long)counters.received, (long long)counters.written);
  else
    std::printf("root_sink: bye (%lld hits received; hits tree off).\n",
                (long long)counters.received);
  return 0;
}
