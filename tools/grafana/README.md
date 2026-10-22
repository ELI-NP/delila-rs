# Grafana rate dashboards (TODO 74)

`gen_dashboards.py` builds the per-experiment dashboard **Rates — &lt;experiment&gt;**
from the DAQ TOML and a channel map. Python 3.11+, standard library only.

## Generate

On the DAQ host, from the repository root:

```sh
python3 tools/grafana/gen_dashboards.py --config config.toml
# options: --map config/channel_map_<experiment>.txt   (this is the default)
#          --out docker/grafana/provisioning/dashboards/json/generated   (default)
#          --datasource-uid P951FEA4DE68E13C5                            (default)
```

- From the TOML it reads `[operator] experiment_name` and every
  `[[network.sources]]`: module = `module_id` (else `id`), board name = `name`,
  channel count = `num_channels` of its digitizer JSON (`config_file`).
  Channels disabled in that JSON and missing from the map are treated as unused.
- Output: `rates_<slug>.json` (uid `rates-<slug>`) in
  `docker/grafana/provisioning/dashboards/json/generated/`. That directory is
  gitignored: generate on each host, never commit. Grafana's file provider scans
  it and loads new or changed files **within ~10 s, no restart**.
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

## What the dashboard shows

- Module × channel grid: rate over the last 10 s of the time range. 0 Hz is red,
  a disabled channel with no counts is grey "off", blue gets darker per decade.
- Event rate per module (`system_rate.event_rate`), plus a list of the enabled
  channels with no counts in the last minute.
- One time-series panel per group.
- Run start/stop markers from `run_event` (filtered to this experiment).

`channel_rate.rate` and `run_event` are written by the Operator (TODO 74 P2/P5).
On an older Operator the per-channel panels report "No field named rate", and
the run markers show an error until the first Start/Stop is written.

The generic `delila_overview.json` (any experiment) and `fission2026_beam.json`
in `docker/grafana/provisioning/dashboards/json/` are static files and are not
generated.

## Tests

```sh
python3 -m unittest discover tools/grafana
```
