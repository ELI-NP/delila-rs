# thgem_sink — the 11B_208Pb experiment's ROOT sink + live monitor

TODO 88. 8 ThGEMs (trigger + XL / XR / YU / YD each, 40 ch) on DPP-PSD boards
(modules 1–3) and N_Wall's VX2730 as module 0 (beam 1/100, LaBr3, LS).
`thgem_sink` is a copy of `nwall_sink` (`../nwall/`) with the Si display
replaced by the ThGEM one; nwall_sink is not touched (one program per
experiment, TODO 79 §5.1). The receive / sort / event-build / record pipeline,
the trees, the time offsets, `BeamTime` / `rfPeriodNs`, the bunch clock and
the `/hits`, `/rf`, `/ls` display are nwall_sink's — see `../nwall/README.md`
and `../README.md`.

This directory also holds October's offline ThGEM tools (`thgem_reco.hpp`,
`thgem_events.C`, `delayline_offsets.C`), which know two ThGEMs only.

## Build

```sh
cd tools/root_sink/thgem
g++ -O2 -std=c++17 -fopenmp thgem_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o thgem_sink
install -m 755 thgem_sink ~/.local/bin/thgem_sink     # where start_daq.sh looks first
```

Unit tests (no ROOT): `g++ -std=c++17 -O0 -g -Wall -Wextra test_thgem_sink.cpp -o /tmp/tts && /tmp/tts`
(channel map, the ThGEMs of an event, the pair dt, the map vs event-def check).

## Run (start_daq.sh)

```toml
[network.root_sink]
program = "thgem_sink"
subscribe = "tcp://localhost:5557"
output_dir = "/data/11B_208Pb"            # = [network.recorder].output_dir
http_port = 8090
hits_tree = true
event_def_file = "config/event_def_11B_208Pb.json"
channel_map = "config/channel_map_11B_208Pb.txt"
# time_calib_file = "config/time_settings_11B_208Pb.json"   # XY centres, board offsets
safe_horizon_ms = 1000
compression = 0
# thgem_delay_ns = 0.0     # ThGEM trigger minus its bunch: picks the bunch of an event
# arm_window_ns = 1000     # an arm = nearest hit to its ThGEM's trigger within +-this
# rf_period_ns = 199.980   # nominal; measured per run from the beam channel
# rf_t0_ns = 0.0
# gamma_min_adc = 4000
```

## Inputs

- **Channel map** (`--channel-map`, `thgem_map.hpp`):
  `config/channel_map_11B_208Pb.txt`, the Grafana columns plus `det kind index`.
  `det` = `beam` / `LaBr` / `LS` / `pulser` / `ThGEM`; for ThGEM `kind` is the
  electrode (`T`, `XL`, `XR`, `YU`, `YD`) and `index` its number. Errors (the
  map is refused; at a run start the previous map is kept, `map STALE`) and
  warnings (a ThGEM without a trigger, without an arm, or spread over boards)
  carry line numbers. Re-read at every run start.
- **Event definition** (`config/event_def_11B_208Pb.json`): trigger = any
  ThGEM trigger, window −500 / +1500 ns, group 100 ns; L2 flag `pair` = two or
  more ThGEM triggers. It names the trigger channels a second time: the sink
  **warns at start-up and at every run start when the map's ThGEM triggers and
  the definition's triggers differ** (`WARNING map vs event def`).

## Display (http://<host>:8090/, JSROOT)

`/hits`, `/rf`, `/ls` as nwall_sink. The ThGEM part, from the built events
(`thgem_planes.hpp`: per ThGEM the earliest trigger of the event, each arm the
hit nearest to that trigger within ±`arm_window_ns`, ties to the later hit —
the rule of October's `thgem_reco.hpp`):

| histogram | what |
|---|---|
| `/thgem/xy/xy_T<n>` | ThGEM n: x = t_XR − t_XL vs y = t_YU − t_YD [ns], events with all four arms; ±250 ns, 1 ns bins |
| `/thgem/narms` | arms found (0–4) × ThGEM: the arm efficiency |
| `/thgem/tof` | ThGEM trigger − bunch at or before it − rf_t0 (0 … T) × ThGEM |
| `/thgem/pair_hits` | ThGEM × ThGEM in the same event (i < j); the diagonal counts events with that ThGEM |
| `/thgem/dt_pair/dt_T<j>_vs_T<i>` | (t_j − tb) vs (t_i − tb), i < j, every pair of the map (made empty when the map is read), tb = the bunch the event's trigger picked (nearest to t_ref − thgem_delay); ±1000 ns, 4 ns bins. October's `dt2_vs_dt1` (TODO 65) for any number of ThGEMs, on the RF instead of the beam × ThGEM coincidence |
| `/thgem/ls_dt` | t(LS) − t(event trigger) per LS channel |
| `/thgem/ls_tof/ls_tof_m<M>_ch<C>` | t(LS) − tb − rf_t0, one 1D per LS channel (user 2026-11-17: 24 1D plots rather than one 2D), made empty when the map is read, title = the map's name; −100 … 1000 ns, 2 ns bins. The neutron TOF: the ThGEM only picks the bunch |

`narms`, `tof` and `pair_hits` have ThGEM numbers 0–15 on their axes; a
higher number gets a warning (its `xy` and pairs are still made).

## Rehearsal without detectors

```sh
root -l -b -q 'make_test_run_11b.C+("thgem_test.root", 2.0)'
./thgem_sink --zmq tcp://127.0.0.1:15562 --out-dir /tmp/tt --hits-tree --exp-name 11B_208Pb \
    --event-def ../../../config/event_def_11B_208Pb.json \
    --channel-map ../../../config/channel_map_11B_208Pb.txt --http-port 18092 &
../common/test_publisher --file thgem_test.root --zmq tcp://127.0.0.1:15562 --run 3
root -l -b -q '../common/check_events.C+("/tmp/tt/run0003_0000_11B_208Pb.root")'
```

E2E (2026-11-17, macOS, ROOT 6.40, draft map): 325,522 hits received and
written, 8,968 events (NTriggers 1: 4,803 singles, 2: 4,161 reactions), late 0,
`check_events.C` IDENTICAL; T measured 199.9500 ns. `pair_hits` off the diagonal
only at (1,5), (2,6), (3,7), (4,8) (plus a few random 1s); `xy_T1` spot at
(−89.80, 70.03), `xy_T6` at (59.98, −29.94) for (−90, 70) and (60, −30) put in;
four arms in 84 % of ThGEM 1's events (0.95⁴ = 81 %); `dt_T5_vs_T1` mean
(25.2, 34.8), correlation −0.85 (−0.84 expected with the 4 ns bins); `tof` peaks
at 27 ns (ThGEM 1, 20–30 put in) and 34 ns (ThGEM 5, 30–40).
