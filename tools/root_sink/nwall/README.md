# nwall_sink — the N_Wall experiment's ROOT sink + live monitor

TODO 79. The N_Wall sink is its own program (one sink per experiment, the
shared part becomes a library later — see `../README.md` *Layout*). The
receive / sort / event-build / record pipeline is a copy of the standard
`root_sink` (`../standard/`), so the trees, the time offsets, the event
definition, the write-failure handling and the per-run provenance behave
exactly as described in `../README.md`. What differs is the display.

## Build

```sh
cd tools/root_sink/nwall
g++ -O2 -std=c++17 nwall_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o nwall_sink
install -m 755 nwall_sink ~/.local/bin/nwall_sink     # where start_daq.sh looks first
```

Unit tests (no ROOT): `g++ -std=c++17 -O0 -g test_nwall.cpp -o /tmp/tnw && /tmp/tnw`
(channel map parser, bunch clock).

## Run (start_daq.sh)

`start_daq.sh` starts the program named by `program` in `[network.root_sink]`
(default `root_sink`):

```toml
[network.root_sink]
program = "nwall_sink"
subscribe = "tcp://localhost:5557"
output_dir = "/data/DELILA_data"          # = [network.recorder].output_dir
http_port = 8090
hits_tree = true
event_def_file = "config/event_def_nwall.json"
channel_map = "config/channel_map_N_Wall.txt"
time_calib_file = "config/time_settings_nwall.json"   # once measured (TODO 72)
safe_horizon_ms = 1000
# rf_period_ns = 199.980   # nominal; measured per run from the beam channel
# rf_t0_ns = 0.0           # see "Calibration" below
# si_delay_ns = 0.0
```

All boards are clock- and start-synchronized, so no `module` filter is needed.

## Inputs

- **Channel map** (`--channel-map`, `nwall_map.hpp`): the shared
  `config/channel_map_N_Wall.txt` — Grafana's four columns plus
  `det kind index` (beam / LaBr / LS / dE / E / pulser; ring / sector; ring or
  sector number). Every role comes from here, none from channel numbers. It is
  re-read at every run start; a bad edit keeps the previous map (`map STALE`
  in the status line). Without a map only `/hits` is filled.
- **Event definition** (`config/event_def_nwall.json`): trigger = any E sector;
  the L2 accept `dEE` marks events that also have a dE sector hit.
- **Bunch clock** (`bunch_clock.hpp`): the beam channel carries every 100th RF
  pulse. Every other bunch is inferred from the nearest beam hit
  (b + n·T); T is measured per run from the gaps between beam hits (they are
  integer multiples of T). The status line shows `beam=N T=... offgrid=K`.

## Display (http://<host>:8090/, JSROOT)

Fed by the Writer: time-sorted core hits and the built events of each chunk
(bounded queue, display only: drops are counted as `display_drops`; the trees
never drop). Histograms accumulate across runs until `/Reset`.

| folder | histogram | what |
|---|---|---|
| `/hits` | `hitmap` | module × channel |
| | `energy_m<M>` | energy × channel, one per board |
| `/rf` | `tof_rf_m<M>` | t − (bunch at or before t) − rf_t0, per channel (0 … T): the beam itself at 0, the gamma flash at L/c |
| `/ls` | `ls_psd` | (E_long − E_short) / E_long vs E_long of the LS channels (n/γ) |
| `/events` | `pid` | dE vs E — the largest dE-sector and E-sector hit of each event |
| | `sector_dE_vs_E` | dE sector × E sector, every pair in an event (the diagonal = the sector order) |
| | `dE_ring_vs_sector` | dE ring × dE sector in an event |
| | `ls_dt_si` | t(LS) − t(E trigger), per LS channel |
| | `si_phase` | E trigger − bunch at or before it (0 … T) |
| | `ls_tof_si` | t(LS) − (the bunch the Si picked) − rf_t0: the neutron TOF of the event, not folded at T |

### How the TOF works (TODO 79 §5.2)

LS, LaBr3 and the beam are on the same board (VX2730), so the RF TOF needs no
board-to-board alignment and is immune to the run-to-run 8 ns start jumps
(TODO 77). In an event the Si only *chooses* the bunch: the bunch nearest to
`t_trigger − si_delay`. Bunches are 200 ns apart, so the Si time may be off by
up to ±100 ns before the wrong bunch is picked — Si board offsets and the 8 ns
jumps do not matter. The TOF of a slow neutron is therefore not folded at T.

### Calibration

1. `si_delay_ns`: the peak of `/events/si_phase` (where the Si trigger sits
   relative to its bunch).
2. `rf_t0_ns`: in `/rf/tof_rf_m<VX2730>` the gamma flash of each LS channel
   must sit at L/c. A per-channel difference (cables) goes into the time
   offsets file; the common remainder is `rf_t0_ns`.

## Rehearsal without detectors

`make_test_run.C` writes a synthetic 2 s N_Wall run (beam 1/100 with
T = 199.95 ns, dE/E reactions with neutrons and prompt gammas, LS singles,
LaBr3, pulser) as a hits tree; `../common/test_publisher` replays it:

```sh
root -l -b -q 'make_test_run.C+("nwall_test.root", 2.0)'
./nwall_sink --zmq tcp://127.0.0.1:15560 --out-dir /tmp/nw --hits-tree \
    --event-def ../../../config/event_def_nwall.json \
    --channel-map ../../../config/channel_map_N_Wall.txt --si-delay-ns 15 &
../common/test_publisher --file nwall_test.root --zmq tcp://127.0.0.1:15560 --run 8
```

E2E (2026-10-30, macOS, ROOT 6.40): 273,824 hits / 3,939 events written;
`check_events.C` IDENTICAL; measured T = 199.9500 ns; `si_phase` peak 15 ns;
LaBr3 at 3 ns, gamma flash at 5.0 ns (L/c for 1.5 m); every hit got an RF TOF;
`sector_dE_vs_E` 99.95 % diagonal; runs repeated with the same clock gave
identical results (the bunch clock restarts per run).
