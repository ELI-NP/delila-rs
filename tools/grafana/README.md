# Grafana rate dashboards (TODO 74)

`gen_dashboards.py` builds the per-experiment dashboard **Rates — &lt;experiment&gt;**
from the DAQ TOML and a channel map. Python 3.11+, standard library only.

## Generate

On the DAQ host, from the repository root:

```sh
python3 tools/grafana/gen_dashboards.py --config config.toml
# options: --map config/channel_map_<experiment>.txt   (this is the default)
#          --out docker/grafana/provisioning/dashboards/json   (default; the file goes to <out>/<experiment>/)
#          --datasource-uid P951FEA4DE68E13C5                            (default)
```

- From the TOML it reads `[operator] experiment_name` and every
  `[[network.sources]]`: module = `module_id` (else `id`), board name = `name`,
  channel count = `num_channels` of its digitizer JSON (`config_file`).
  Channels disabled in that JSON and missing from the map are treated as unused.
- Output: `rates_<slug>.json` (uid `rates-<slug>`) in
  `docker/grafana/provisioning/dashboards/json/<experiment_name>/`, which is the
  experiment's Grafana folder (`foldersFromFilesStructure: true`). Generated
  files are gitignored (`json/*/rates_*.json`): generate on each host, never
  commit. Grafana loads new or changed files **within ~10 s, no restart**.
- Without a map file every channel is shown as `M<module>/Ch<channel>`.

## Rename channels

InfluxDB holds only `(module, channel)`. Names are applied in each panel's
SQL, so a rename also applies to past data:

1. Edit `config/channel_map_<experiment>.txt`
   (one channel per line: `module channel name group`, see the header of the
   file; columns 5+ are ignored, `#` starts a comment).
2. Re-run the generator.
3. Reload the dashboard in the browser after ~10 s.

The map is checked first. Every error is listed with its line number and
nothing is written until the map is clean.

## Editing in the Grafana UI

Logged-in users can change and save any of these dashboards in Grafana
(`allowUiUpdates: true` in `docker/grafana/provisioning/dashboards/default.yml`);
anonymous visitors only view. A saved edit stays until the JSON file itself
changes. Re-running the generator with a changed map (or a git pull that
updates a dashboard file) replaces it with the file's version. Re-running with
the same map writes an identical file and keeps the edit. To keep a change for
good, put it in the generator or the map, or use "Save as" for a copy that is
not provisioned.

## What the dashboard shows

- Module × channel grid: rate over the last 10 s of the time range. 0 Hz during
  a run is red, no counts with no run going is "-", a disabled channel with no
  counts is grey "off", blue gets darker per decade. "A run is going" = the last
  `run_event` at the end of the time range is a start; Tune Up does not count.
- Digitizer clock (PLL), one thin row per board (`pll.locked`: 1 locked green,
  0 not locked red, -1 no answer dark red; TODO 85). A board without its PLL
  lock takes no valid data, and the Operator stops a run when that lasts 10 s.
  Boards whose firmware has no PLL status (AMax) have no row.
- Event rate per module (`system_rate.event_rate`), plus a list of the enabled
  channels with no counts in the last minute of a run.
- One time-series panel per group: half width up to 16 channels, full width
  above. Exactly three neighbouring groups numbered the same way (`dE_Ring1`,
  `dE_Ring2`, `dE_Ring3`) share one row, in number order, whatever their order in
  the map.
- Group `-` in the map = nobody needs to watch this channel (the pulser inputs):
  no panel, never listed as silent, grey "off" in the grid unless it counts.
- Run start/stop markers from `run_event` (filtered to this experiment, without
  Tune Up, which runs as run 0).

`channel_rate.rate` and `run_event` are written by the Operator (TODO 74 P2/P5).
On an older Operator the per-channel panels report "No field named rate", and
the run markers show an error until the first Start/Stop is written.

## Folders

One Grafana folder per subdirectory of `docker/grafana/provisioning/dashboards/json/`:

| Folder | Contents |
|---|---|
| `Common/` | `delila_overview.json` — the generic DAQ overview (any experiment) |
| `<experiment_name>/` | that experiment's committed dashboards (e.g. `N_Wall/`: the SY5527 and MHV-4 HV dashboards) plus its generated `rates_<slug>.json` |

Static dashboards are committed; only `rates_*.json` is generated. A dashboard
file placed directly in `json/` would land in Grafana's "General" folder, so
don't (a test checks this). The Fission2026 dashboards were removed on
2026-10-22 (the experiment is over; they remain in git history).

## Tests

```sh
python3 -m unittest discover tools/grafana
```
