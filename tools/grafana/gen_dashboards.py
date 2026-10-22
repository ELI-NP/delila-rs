#!/usr/bin/env python3
"""Generate the per-experiment Grafana rate dashboard (TODO 74 P4).

Inputs: the DAQ TOML (experiment name, boards) and a channel map
(``module channel name group`` per line, whitespace separated, TODO 74 §3.2).

InfluxDB only knows (module, channel): the Operator never writes names.
Names are put in at query time (a CASE in each panel's SQL), so renaming a
channel = edit the map, re-run this script, and Grafana reloads the file
within ~10 s - the new name applies to past data as well.

    python3 tools/grafana/gen_dashboards.py --config config/config_thgem_pfqaq.toml

Python 3.11+ standard library only. See tools/grafana/README.md.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
# Grafana's file provider (docker/grafana/provisioning/dashboards/default.yml)
# turns each subdirectory of json/ into a folder; the dashboard is written to
# json/<experiment_name>/rates_<slug>.json, which is gitignored (host-local output).
DEFAULT_OUT = REPO_ROOT / "docker/grafana/provisioning/dashboards/json"
# experiment_name becomes a directory (and Grafana folder) name.
FOLDER_RE = re.compile(r"^[A-Za-z0-9_.+\-]+$")
# The provisioned InfluxDB datasource (same uid as every dashboard in json/).
DEFAULT_DS_UID = "P951FEA4DE68E13C5"

# name/group end up inside SQL string literals: no quotes, no spaces.
NAME_RE = re.compile(r"^[A-Za-z0-9_.+\-/]+$")
INT_RE = re.compile(r"^[0-9]+$")

# Every scan of a measurement is bounded in time: InfluxDB 3 Core refuses
# queries that touch more than 432 parquet files (~3.5 days on pf-qaq).
RANGE = "time >= $__timeFrom AND time <= $__timeTo"
GRID_WINDOW = "10 seconds"  # "current" rate = mean over the last 10 s of the range
SILENT_WINDOW = "1 minute"  # silent channel = no count during the last minute

# Cell colours of the module x channel grid (Hz, absolute thresholds):
# 0 Hz is red, a disabled channel without counts grey ("off"), counting
# channels get darker per decade.
GRID_THRESHOLDS = [
    {"color": "red", "value": None},
    {"color": "super-light-blue", "value": 1e-9},
    {"color": "light-blue", "value": 10},
    {"color": "blue", "value": 100},
    {"color": "semi-dark-blue", "value": 1000},
    {"color": "dark-blue", "value": 10000},
    {"color": "purple", "value": 100000},
]
OFF = -1  # grid value of a disabled channel with no counts


class GenError(Exception):
    """A user-facing error (bad TOML, bad map, missing file)."""


@dataclass(frozen=True)
class Board:
    module: int  # module number of the channel map: module_id, else id
    tag: str  # InfluxDB `module` tag: the Operator writes the source id
    name: str
    num_channels: int
    disabled: frozenset[int] = frozenset()

    @property
    def label(self) -> str:
        return f"M{self.module} {self.name}" if self.name else f"M{self.module}"


@dataclass(frozen=True)
class Entry:
    """One line of the channel map."""

    module: int
    channel: int
    name: str
    group: str
    line: int


@dataclass(frozen=True)
class Channel:
    board: Board
    channel: int
    name: str
    group: str

    @property
    def enabled(self) -> bool:
        return self.channel not in self.board.disabled


# --------------------------------------------------------------------------
# Inputs
# --------------------------------------------------------------------------


def _resolve_config_file(rel: str, toml_path: Path) -> Path:
    """Digitizer JSON paths in the TOML are relative to the repo root (the
    DAQ runs from there); also accept the TOML's parent's parent and parent."""
    p = Path(rel)
    if p.is_absolute():
        candidates = [p]
    else:
        candidates = []
        for base in (REPO_ROOT, toml_path.resolve().parent.parent,
                     toml_path.parent.parent, toml_path.parent):
            c = base / p
            if c not in candidates:
                candidates.append(c)
    for c in candidates:
        if c.is_file():
            return c
    tried = ", ".join(str(c) for c in candidates)
    raise GenError(f"digitizer config '{rel}' not found (tried {tried})")


def _is_false(value) -> bool:
    if isinstance(value, bool):
        return not value
    if isinstance(value, (int, float)):
        return value == 0
    return str(value).strip().lower() in ("false", "0", "no", "off")


def _disabled_channels(dig: dict, num_channels: int) -> frozenset[int]:
    defaults = dig.get("channel_defaults") or {}
    overrides = dig.get("channel_overrides") or {}
    if not isinstance(overrides, dict):
        overrides = {}
    default_en = defaults.get("enabled", True) if isinstance(defaults, dict) else True
    off = set()
    for ch in range(num_channels):
        ov = overrides.get(str(ch)) or {}
        if _is_false(ov.get("enabled", default_en) if isinstance(ov, dict) else default_en):
            off.add(ch)
    return frozenset(off)


def load_config(toml_path: Path) -> tuple[str, list[Board]]:
    """Experiment name and boards ([[network.sources]]) of a DAQ TOML."""
    try:
        with open(toml_path, "rb") as f:
            cfg = tomllib.load(f)
    except FileNotFoundError:
        raise GenError(f"{toml_path}: no such file") from None
    except tomllib.TOMLDecodeError as e:
        raise GenError(f"{toml_path}: {e}") from None

    exp = (cfg.get("operator") or {}).get("experiment_name")
    if not exp:
        raise GenError(f"{toml_path}: [operator] experiment_name is missing")
    sources = (cfg.get("network") or {}).get("sources") or []
    if not sources:
        raise GenError(f"{toml_path}: no [[network.sources]]")

    boards: list[Board] = []
    for src in sources:
        if "id" not in src:
            raise GenError(f"{toml_path}: a [[network.sources]] entry has no id")
        sid = int(src["id"])
        name = str(src.get("name", ""))
        what = f"{toml_path}: source id={sid} ({name or 'no name'})"
        rel = src.get("config_file")
        if not rel:
            raise GenError(f"{what} has no config_file (the channel count comes from its digitizer JSON)")
        try:
            jpath = _resolve_config_file(rel, toml_path)
        except GenError as e:
            raise GenError(f"{what}: {e}") from None
        try:
            dig = json.loads(jpath.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            raise GenError(f"{what}: cannot read {jpath}: {e}") from None
        if not isinstance(dig, dict):
            raise GenError(f"{what}: {jpath} is not a digitizer config object")
        nch = dig.get("num_channels")
        if nch is None:
            nch = 32  # same default as the DAQ (src/config/digitizer.rs)
            print(f"note: {jpath} has no num_channels; using the DAQ default 32", file=sys.stderr)
        if not isinstance(nch, int) or nch <= 0:
            raise GenError(f"{what}: {jpath}: num_channels must be a positive integer, got {nch!r}")
        boards.append(Board(
            module=int(src.get("module_id", sid)),
            tag=str(sid),
            name=name,
            num_channels=nch,
            disabled=_disabled_channels(dig, nch),
        ))

    for i, a in enumerate(boards):
        for b in boards[:i]:
            if a.module == b.module:
                raise GenError(f"{toml_path}: two sources have module {a.module} ('{b.name}', '{a.name}')")
    return exp, boards


def parse_map(text: str, boards: list[Board], source: str = "<map>") -> list[Entry]:
    """Parse a channel map (TODO 74 §3.2). All errors are reported at once,
    each with its line number."""
    by_module = {b.module: b for b in boards}
    entries: list[Entry] = []
    seen: dict[tuple[int, int], int] = {}
    names: dict[str, int] = {}
    errors: list[str] = []

    for lineno, raw in enumerate(text.splitlines(), start=1):
        cols = raw.split("#", 1)[0].split()
        if not cols:
            continue
        where = f"{source}:{lineno}"
        if len(cols) < 4:
            errors.append(f"{where}: expected 'module channel name group', got {len(cols)} column(s)")
            continue
        mod_s, ch_s, name, group = cols[:4]  # columns 5+ belong to the analysis
        bad = False
        for label, tok in (("module", mod_s), ("channel", ch_s)):
            if not INT_RE.match(tok):
                errors.append(f"{where}: {label} '{tok}' is not a non-negative integer")
                bad = True
        for label, tok in (("name", name), ("group", group)):
            if not NAME_RE.match(tok):
                errors.append(f"{where}: {label} '{tok}' may only contain A-Z a-z 0-9 _ . + - /")
                bad = True
        if bad:
            continue
        module, channel = int(mod_s), int(ch_s)
        board = by_module.get(module)
        if board is None:
            known = ", ".join(str(m) for m in sorted(by_module))
            errors.append(f"{where}: module {module} is not in the DAQ config (modules: {known})")
            continue
        if channel >= board.num_channels:
            errors.append(f"{where}: channel {channel} out of range: module {module} "
                          f"({board.name}) has {board.num_channels} channels (0-{board.num_channels - 1})")
            continue
        key = (module, channel)
        if key in seen:
            errors.append(f"{where}: module {module} channel {channel} already listed on line {seen[key]}")
            continue
        if name in names:
            errors.append(f"{where}: name '{name}' already used on line {names[name]} (names label the series)")
            continue
        seen[key] = lineno
        names[name] = lineno
        entries.append(Entry(module, channel, name, group, lineno))

    # A user name must not be the default name M{m}/Ch{c} of an unlisted channel.
    for e in entries:
        m = re.fullmatch(r"M([0-9]+)/Ch([0-9]+)", e.name)
        if not m:
            continue
        other = (int(m[1]), int(m[2]))
        board = by_module.get(other[0])
        if other not in seen and board is not None and other[1] < board.num_channels:
            errors.append(f"{source}:{e.line}: name '{e.name}' is the default name of module "
                          f"{other[0]} channel {other[1]}, which is not in the map")

    if errors:
        raise GenError("\n".join(errors))
    return entries


def build_channels(boards: list[Board], entries: list[Entry]) -> list[Channel]:
    """Every channel of every board; unlisted ones are M{m}/Ch{c} in group M{m}."""
    listed = {(e.module, e.channel): e for e in entries}
    out = []
    for b in boards:
        for ch in range(b.num_channels):
            e = listed.get((b.module, ch))
            if e:
                out.append(Channel(b, ch, e.name, e.group))
            else:
                out.append(Channel(b, ch, f"M{b.module}/Ch{ch}", f"M{b.module}"))
    return out


def group_channels(channels: list[Channel], entries: list[Entry]) -> dict[str, list[Channel]]:
    """Groups in map order (first appearance), then the per-board defaults.
    A channel that is neither in the map nor enabled is unused: no panel."""
    listed = {(e.module, e.channel) for e in entries}
    groups: dict[str, list[Channel]] = {e.group: [] for e in entries}
    for c in channels:
        if c.enabled or (c.board.module, c.channel) in listed:
            groups.setdefault(c.group, []).append(c)
    return {g: cs for g, cs in groups.items() if cs}


def slug(text: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:34].strip("-")
    if not s:
        raise GenError(f"experiment name '{text}' has no letters or digits")
    return s


# --------------------------------------------------------------------------
# SQL (InfluxDB 3 / DataFusion). Tags are strings: module = '0'.
# --------------------------------------------------------------------------


def lit(s: str) -> str:
    return "'" + s.replace("'", "''") + "'"


def last(window: str) -> str:
    """The last `window` of the dashboard's time range."""
    return f"time > $__timeTo - interval '{window}' AND time <= $__timeTo"


def name_case(channels: list[Channel]) -> str:
    whens = "\n".join(
        f"    WHEN module = {lit(c.board.tag)} AND channel = '{c.channel}' THEN {lit(c.name)}"
        for c in channels
    )
    return f"  CASE\n{whens}\n    ELSE concat('M', module, '/Ch', channel)\n  END"


def channel_filter(channels: list[Channel]) -> str:
    by_tag: dict[str, list[int]] = {}
    for c in channels:
        by_tag.setdefault(c.board.tag, []).append(c.channel)
    parts = [
        f"(module = {lit(tag)} AND channel IN ({', '.join(lit(str(ch)) for ch in chs)}))"
        for tag, chs in by_tag.items()
    ]
    return "(" + "\n       OR ".join(parts) + ")"


def group_rate_sql(channels: list[Channel]) -> str:
    return (
        "SELECT $__dateBin(time) AS time, AVG(rate) AS rate,\n"
        f"{name_case(channels)} AS name\n"
        "FROM channel_rate\n"
        f"WHERE {RANGE}\n"
        f"  AND {channel_filter(channels)}\n"
        "GROUP BY 1, module, channel\n"
        "ORDER BY 1"
    )


def board_case(boards: list[Board]) -> str:
    whens = " ".join(f"WHEN {lit(b.tag)} THEN {lit(b.label)}" for b in boards)
    return f"CASE module {whens} ELSE concat('M', module) END"


def module_rate_sql(boards: list[Board]) -> str:
    tags = ", ".join(lit(b.tag) for b in boards)
    return (
        "SELECT $__dateBin(time) AS time, AVG(event_rate) AS event_rate,\n"
        f"  {board_case(boards)} AS board\n"
        "FROM system_rate\n"
        f"WHERE {RANGE}\n"
        f"  AND module IN ({tags})\n"
        "GROUP BY 1, module\n"
        "ORDER BY 1"
    )


def grid_sql(boards: list[Board], channels: list[Channel]) -> str:
    """One row per board, one column per channel: mean rate over the last
    GRID_WINDOW of the range. 0 = no counts, -1 = disabled and no counts,
    NULL = the board has no such channel."""
    nmax = max(b.num_channels for b in boards)
    cols = ",\n".join(f"  MAX(CASE WHEN c.ch = '{i}' THEN c.v END) AS \"{i}\"" for i in range(nmax))
    board_rows = ", ".join(f"({i}, {lit(b.tag)}, {lit(b.label)})" for i, b in enumerate(boards))
    ch_rows = ",\n      ".join(
        f"({lit(c.board.tag)}, '{c.channel}', {'true' if c.enabled else 'false'})" for c in channels
    )
    return (
        "SELECT b.board AS \"Board\",\n"
        f"{cols}\n"
        f"FROM (VALUES {board_rows}) AS b(pos, tag, board)\n"
        "JOIN (\n"
        f"  SELECT m.tag, m.ch, CASE WHEN r.rate > 0 THEN r.rate WHEN m.en THEN 0 ELSE {OFF} END AS v\n"
        "  FROM (VALUES\n"
        f"      {ch_rows}\n"
        "  ) AS m(tag, ch, en)\n"
        "  LEFT JOIN (\n"
        "    SELECT module, channel, AVG(rate) AS rate FROM channel_rate\n"
        f"    WHERE {last(GRID_WINDOW)}\n"
        "    GROUP BY module, channel\n"
        "  ) AS r ON r.module = m.tag AND r.channel = m.ch\n"
        ") AS c ON c.tag = b.tag\n"
        "GROUP BY b.pos, b.board\n"
        "ORDER BY b.pos"
    )


def silent_sql(channels: list[Channel]) -> str:
    """Enabled channels without a count during the last SILENT_WINDOW
    (also those the Operator wrote no row for: it skips count = 0)."""
    rows = [
        f"({lit(c.board.tag)}, '{c.channel}', {c.board.module}, {c.channel}, {lit(c.name)}, {lit(c.group)})"
        for c in channels if c.enabled
    ]
    keep = "COALESCE(r.rate, 0) = 0"
    if not rows:  # every channel disabled: keep the SQL valid, list nothing
        rows, keep = ["('', '', -1, -1, '', '')"], "false"
    return (
        "SELECT m.name AS \"Name\", m.grp AS \"Group\", m.module AS \"Module\", m.channel AS \"Channel\"\n"
        "FROM (VALUES\n"
        "    " + ",\n    ".join(rows) + "\n"
        ") AS m(tag, ch, module, channel, name, grp)\n"
        "LEFT JOIN (\n"
        "  SELECT module, channel, MAX(rate) AS rate FROM channel_rate\n"
        f"  WHERE {last(SILENT_WINDOW)}\n"
        "  GROUP BY module, channel\n"
        ") AS r ON r.module = m.tag AND r.channel = m.ch\n"
        f"WHERE {keep}\n"
        "ORDER BY m.module, m.channel"
    )


def run_event_sql(event: str, experiment: str | None) -> str:
    """Annotation query on run_event (written by the Operator at Start/Stop)."""
    if experiment is None:
        text = "concat(exp, ' run ', CAST(run AS VARCHAR), ' ', event)"
        exp_filter = ""
    else:
        text = "concat('Run ', CAST(run AS VARCHAR), ' ', event)"
        exp_filter = f" AND exp = {lit(experiment)}"
    return (
        f"SELECT time, {text} AS text\n"
        "FROM run_event\n"
        f"WHERE {RANGE}{exp_filter} AND event = {lit(event)}\n"
        "ORDER BY time"
    )


# --------------------------------------------------------------------------
# Grafana JSON
# --------------------------------------------------------------------------


def ds_ref(uid: str) -> dict:
    return {"type": "influxdb", "uid": uid}


def target(sql: str, fmt: str, uid: str, ref: str = "A") -> dict:
    return {"refId": ref, "datasource": ds_ref(uid), "rawSql": sql, "format": fmt}


def run_annotations(uid: str, experiment: str | None) -> list[dict]:
    """Run start/stop markers. `query` repeats the SQL on purpose: Grafana's
    InfluxDB plugin turns an annotation target WITHOUT `query` into a legacy
    InfluxQL annotation (prepareAnnotation), which never runs in SQL mode."""
    out = [{
        "builtIn": 1,
        "datasource": {"type": "grafana", "uid": "-- Grafana --"},
        "enable": True,
        "hide": True,
        "iconColor": "rgba(0, 211, 255, 1)",
        "name": "Annotations & Alerts",
        "type": "dashboard",
    }]
    for event, colour in (("start", "green"), ("stop", "red")):
        sql = run_event_sql(event, experiment)
        out.append({
            "datasource": ds_ref(uid),
            "enable": True,
            "hide": False,
            "iconColor": colour,
            "name": f"Run {event}",
            "target": {"refId": "Anno", "datasource": ds_ref(uid), "rawSql": sql, "query": sql, "format": "table"},
        })
    return out


def timeseries_panel(pid: int, title: str, pos: dict, targets: list[dict], uid: str, unit: str,
                     display: str, calcs=("mean", "lastNotNull"), description: str = "") -> dict:
    return {
        "id": pid,
        "type": "timeseries",
        "title": title,
        "description": description,
        "gridPos": pos,
        "datasource": ds_ref(uid),
        "fieldConfig": {
            "defaults": {
                "unit": unit,
                "displayName": display,
                "custom": {"lineWidth": 1, "fillOpacity": 0, "showPoints": "never", "spanNulls": False},
            },
            "overrides": [],
        },
        "options": {
            "tooltip": {"mode": "multi", "sort": "desc"},
            "legend": {"showLegend": True, "displayMode": "table", "placement": "right", "calcs": list(calcs)},
        },
        "targets": targets,
    }


def grid_panel(pid: int, pos: dict, boards: list[Board], channels: list[Channel], uid: str) -> dict:
    return {
        "id": pid,
        "type": "table",
        "title": f"Rate per channel (Hz, mean of the last {GRID_WINDOW} of the time range)",
        "description": "Red = 0 Hz (no counts). Grey 'off' = disabled in the digitizer JSON and no counts. "
                       "Blue shades per decade (10, 100, 1k, 10k Hz), purple >= 100 kHz. "
                       "Columns are channel numbers; names are in the panels below.",
        "gridPos": pos,
        "datasource": ds_ref(uid),
        "fieldConfig": {
            "defaults": {
                "unit": "short",
                "color": {"mode": "thresholds"},
                "thresholds": {"mode": "absolute", "steps": GRID_THRESHOLDS},
                "mappings": [
                    {"type": "value", "options": {str(OFF): {"text": "off", "color": "#808080", "index": 0}}},
                    {"type": "special", "options": {"match": "null", "result": {"text": "", "color": "transparent", "index": 1}}},
                ],
                "custom": {"align": "center", "minWidth": 40,
                           "cellOptions": {"type": "color-background", "mode": "basic"}},
            },
            "overrides": [{
                "matcher": {"id": "byName", "options": "Board"},
                "properties": [
                    {"id": "custom.cellOptions", "value": {"type": "auto"}},
                    {"id": "custom.align", "value": "left"},
                    {"id": "custom.width", "value": 180},
                ],
            }],
        },
        "options": {"showHeader": True, "cellHeight": "sm", "footer": {"show": False}},
        "targets": [target(grid_sql(boards, channels), "table", uid)],
    }


def silent_panel(pid: int, pos: dict, channels: list[Channel], uid: str) -> dict:
    return {
        "id": pid,
        "type": "table",
        "title": f"Silent channels (0 Hz, last {SILENT_WINDOW})",
        "description": "Enabled channels without a single count during the last minute of the time range. "
                       "Channels disabled in the digitizer JSON are not listed.",
        "gridPos": pos,
        "datasource": ds_ref(uid),
        "fieldConfig": {
            "defaults": {"noValue": "none - every enabled channel counts", "custom": {"align": "left"}},
            "overrides": [{
                "matcher": {"id": "byName", "options": "Name"},
                "properties": [
                    {"id": "color", "value": {"mode": "fixed", "fixedColor": "red"}},
                    {"id": "custom.cellOptions", "value": {"type": "color-text"}},
                ],
            }] + [{
                "matcher": {"id": "byName", "options": col},
                "properties": [{"id": "custom.width", "value": width}],
            } for col, width in (("Name", 100), ("Group", 90), ("Module", 70), ("Channel", 70))],
        },
        "options": {"showHeader": True, "cellHeight": "sm", "footer": {"show": False}},
        "targets": [target(silent_sql(channels), "table", uid)],
    }


def rates_dashboard(experiment: str, boards: list[Board], channels: list[Channel],
                    groups: dict[str, list[Channel]], uid: str = DEFAULT_DS_UID,
                    sources: str = "") -> dict:
    panels: list[dict] = []
    pid = 0

    def next_id() -> int:
        nonlocal pid
        pid += 1
        return pid

    # Title + header + horizontal scrollbar + one ~34 px row per board (grid unit = 38 px).
    grid_h = max(5, -(-(120 + 34 * len(boards)) // 38))
    panels.append(grid_panel(next_id(), {"h": grid_h, "w": 24, "x": 0, "y": 0}, boards, channels, uid))
    y = grid_h
    panels.append(timeseries_panel(
        next_id(), "Event rate per module (Hz)", {"h": 9, "w": 16, "x": 0, "y": y},
        [target(module_rate_sql(boards), "time_series", uid)], uid, "Hz", "${__field.labels.board}",
        calcs=("mean", "max", "lastNotNull")))
    panels.append(silent_panel(next_id(), {"h": 9, "w": 8, "x": 16, "y": y}, channels, uid))
    y += 9
    panels.append({"id": next_id(), "type": "row", "title": "Channel rates by group", "collapsed": False,
                   "gridPos": {"h": 1, "w": 24, "x": 0, "y": y}, "panels": []})
    y += 1

    # Up to 16 channels: half width, two side by side. More: full width,
    # taller above 32 so the legend table stays readable.
    x = row_h = 0
    for name, chs in groups.items():
        w = 24 if len(chs) > 16 else 12
        h = 12 if len(chs) > 32 else 9
        if x + w > 24:
            x, y, row_h = 0, y + row_h, 0
        panels.append(timeseries_panel(
            next_id(), f"{name} ({len(chs)} ch)", {"h": h, "w": w, "x": x, "y": y},
            [target(group_rate_sql(chs), "time_series", uid)], uid, "Hz", "${__field.labels.name}"))
        x, row_h = x + w, max(row_h, h)
        if x >= 24:
            x, y, row_h = 0, y + row_h, 0

    return {
        "uid": f"rates-{slug(experiment)}",
        "title": f"Rates — {experiment}",
        "description": f"Generated by tools/grafana/gen_dashboards.py{sources}. Edits saved in Grafana are "
                       "replaced when the generator is re-run with a changed channel map; names belong in the map.",
        "tags": ["delila", "rates"],
        "editable": True,
        "graphTooltip": 1,
        "fiscalYearStartMonth": 0,
        "liveNow": False,
        "refresh": "10s",
        "schemaVersion": 39,
        "time": {"from": "now-30m", "to": "now"},
        "timepicker": {},
        "timezone": "",
        "templating": {"list": []},
        "annotations": {"list": run_annotations(uid, experiment)},
        "links": [{"title": "DAQ", "type": "dashboards", "tags": ["daq"], "asDropdown": False,
                   "includeVars": False, "keepTime": True, "targetBlank": False, "icon": "external link"}],
        "panels": panels,
    }


def write_json_atomic(obj: dict, path: Path) -> None:
    """Grafana re-reads the directory every ~10 s: never let it see half a file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".json.part", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(obj, f, indent=2, ensure_ascii=False)
            f.write("\n")
        os.chmod(tmp, 0o644)
        os.replace(tmp, path)
    except BaseException:
        Path(tmp).unlink(missing_ok=True)
        raise


def _rel(p: Path) -> str:
    try:
        return str(p.resolve().relative_to(REPO_ROOT))
    except ValueError:
        return str(p)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", required=True, type=Path, help="DAQ TOML (e.g. config/config_thgem_pfqaq.toml)")
    ap.add_argument("--map", type=Path,
                    help="channel map (default: config/channel_map_<experiment_name>.txt; "
                         "if that does not exist every channel gets its default name)")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help="base directory; the dashboard goes to <out>/<experiment_name>/ "
                         f"(default: {_rel(DEFAULT_OUT)})")
    ap.add_argument("--datasource-uid", default=DEFAULT_DS_UID, help="InfluxDB datasource uid (default: %(default)s)")
    args = ap.parse_args(argv)

    try:
        experiment, boards = load_config(args.config)
        map_path = args.map or REPO_ROOT / "config" / f"channel_map_{experiment}.txt"
        if map_path.is_file():
            entries = parse_map(map_path.read_text(encoding="utf-8"), boards, _rel(map_path))
            sources = f" from {_rel(args.config)} and {_rel(map_path)}"
        elif args.map:
            raise GenError(f"{args.map}: no such file")
        else:
            print(f"note: {_rel(map_path)} does not exist: every channel gets its default name M<m>/Ch<c>",
                  file=sys.stderr)
            entries = []
            sources = f" from {_rel(args.config)} (no channel map)"
        channels = build_channels(boards, entries)
        groups = group_channels(channels, entries)
        dash = rates_dashboard(experiment, boards, channels, groups, args.datasource_uid, sources)
        if not FOLDER_RE.match(experiment):
            raise GenError(f"experiment_name {experiment!r} cannot be a folder name "
                           "(letters, digits, _ . + - only)")
        out = args.out / experiment / f"rates_{slug(experiment)}.json"
        write_json_atomic(dash, out)
    except GenError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    print(f"wrote {out} (uid {dash['uid']})")
    print(f"  {len(boards)} board(s), {len(channels)} channels, {len(entries)} named, "
          f"{sum(not c.enabled for c in channels)} disabled")
    print("  groups: " + ", ".join(f"{g} ({len(cs)})" for g, cs in groups.items()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
