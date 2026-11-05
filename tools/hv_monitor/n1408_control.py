#!/usr/bin/env python3
"""CAEN N1408 (Si bias): monitor + per-channel ON/OFF and settings over USB, with a small web page.

Spec: TODO/82_n1408_si_hv_control.md
  Protocol: UM7352 N1408 User Manual rev.2 p.29-32 (9600 8N1, ASCII; STATUS bits p.30-31).
  §2 behaviour measured on the real unit (FW 1.00, SN 1592, pf-qaq, 2026-11-05).

What the web can do, per channel: ON, OFF, and set VSET, ISET, TRIP, RUP and RDW. MAXV and the trip
mode stay on the panel, so the limit is not one click away from the voltage it limits. Every byte
written to the serial port goes through send_command(), which accepts nothing else. During a
run only OFF works (user, to avoid confusion); Tune Up is not a run. The final say stays with the
unit: REMOTE on the panel and the channel switch at HV_EN. LOCAL on the panel locks the software out
(LOC:ERR).

The unit itself takes VSET above MAXV, TRIP = 1000 (infinite over-current) and ramps up to 100 V/s,
all measured, so those limits are enforced here (ramps: ramp_max_v_per_s in the config). Out-of-range
values are refused, never clamped.

The passphrase prevents accidents, it is NOT security: plain HTTP, plain text on the host, one shared
phrase (user's decision, as in tpcdaq-rs). No passphrase file = read-only. Nothing about the channel
state is kept in software: every field shown comes from the unit's own readback.

Usage:
    python3 -u n1408_control.py --config n1408_control_config.yaml
"""

from __future__ import annotations

import argparse
import collections
import functools
import json
import logging
import math
import os
import queue
import re
import secrets
import signal
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import requests
import yaml

from mhv4_monitor import (
    StopFlag,
    WarnLimiter,
    _close_quietly,
    escape_field_string,
    escape_tag,
    fmt_float,
    write_influx,
)

log = logging.getLogger("n1408_control")

# --- protocol ------------------------------------------------------------------------------------

CHANNELS = (0, 1, 2, 3)
ALL_CHANNELS = 4  # "CH:4" = every channel; monitor commands only, see allowed_commands()

# Read every cycle: (protocol parameter, reading field, parser).
_INT_RE = re.compile(r"\d+")
_NUMBER_RE = re.compile(r"[+-]?\d+(\.\d+)?")


def _parse_int(text: str) -> int:
    if not _INT_RE.fullmatch(text):
        raise ValueError(text)
    return int(text)


def _parse_number(text: str) -> float:
    if not _NUMBER_RE.fullmatch(text):
        raise ValueError(text)
    return float(text)


def _parse_choice(*choices):
    def parse(text: str) -> str:
        if text not in choices:
            raise ValueError(text)
        return text

    return parse


CHANNEL_QUERIES = (
    ("STAT", "status", _parse_int),
    ("VMON", "vmon", _parse_number),
    ("IMON", "imon", _parse_number),
    ("VSET", "vset", _parse_number),
    ("ISET", "iset", _parse_number),
    ("MAXV", "maxv", _parse_number),
    ("RUP", "rup", _parse_number),
    ("RDW", "rdw", _parse_number),
    ("TRIP", "trip", _parse_number),
    ("PDWN", "pdwn", _parse_choice("KILL", "RAMP")),
    ("POL", "polarity", _parse_choice("+", "-")),
)
BOARD_QUERIES = (("BDCTR", "control"), ("BDILK", "interlock"), ("BDALARM", "alarm"))
IDENTITY_QUERIES = (
    ("BDNAME", "name"),
    ("BDNCH", "channels"),
    ("BDFREL", "firmware"),
    ("BDSNUM", "serial"),
    ("BDILKM", "interlock_mode"),
)

# UM7352 p.30-31.
STATUS_BITS = ("ON", "RUP", "RDW", "OVC", "OVV", "UNV", "MAXV", "TRIP", "OVP", "OVT", "DIS", "KILL",
               "ILK", "NOCAL")
BIT_ON, BIT_RUP, BIT_RDW, BIT_UNV = 1 << 0, 1 << 1, 1 << 2, 1 << 5
BIT_TRIP, BIT_DIS, BIT_KILL, BIT_ILK = 1 << 7, 1 << 10, 1 << 11, 1 << 12
BIT_NOCAL = 1 << 13
UNDER_VOLTAGE_V = 2.5  # the unit's own UNV margin (p.30)

# Bits worth a line in the audit log when they change. Ramp bits come and go in every ON and OFF.
_EVENT_MASK = sum(1 << STATUS_BITS.index(n) for n in
                  ("ON", "OVC", "MAXV", "TRIP", "OVP", "OVT", "DIS", "KILL", "ILK", "NOCAL"))


def _mon_board(board: int, par: str) -> str:
    return f"$BD:{board:02d},CMD:MON,PAR:{par}"


def _mon_channel(board: int, ch: int, par: str) -> str:
    return f"$BD:{board:02d},CMD:MON,CH:{ch},PAR:{par}"


def _set_power(board: int, ch: int, action: str) -> str:
    return f"$BD:{board:02d},CMD:SET,CH:{ch},PAR:{action.upper()}"


@functools.lru_cache(maxsize=None)
def allowed_commands(board: int) -> frozenset:
    """Every fixed string send_command() will write: the monitor commands and ON/OFF per channel.
    Explicit set: nothing like a second command after a line break or a CH:4 ON can slip through."""
    cmds = {_mon_board(board, p) for p, _ in BOARD_QUERIES + IDENTITY_QUERIES}
    cmds |= {_mon_channel(board, ALL_CHANNELS, p) for p, _, _ in CHANNEL_QUERIES}
    for ch in CHANNELS:
        cmds |= {_mon_channel(board, ch, p)
                 for p in ("STAT", "VMON", "IMON", "VSET", "ISET", "TRIP", "RUP", "RDW", "MAXV",
                           "IMAX")}
        cmds |= {_set_power(board, ch, a) for a in ("on", "off")}
    return frozenset(cmds)


@dataclass(frozen=True)
class Setting:
    """What the web may change. decimals = what the unit keeps; tolerance = half of its step, for
    comparing a readback (it steps VSET in 0.02 V: 80.01 comes back as 080.02, TODO 82 §2.1)."""

    decimals: int
    tolerance: float


SETTINGS = {
    "VSET": Setting(2, 0.011),
    "ISET": Setting(3, 0.0006),
    "TRIP": Setting(1, 0.051),
    "RUP": Setting(0, 0.5),
    "RDW": Setting(0, 0.5),
}
TRIP_MAX_S = 999.9  # 1000 means infinite over-current (UM7352 p.14); the unit would accept it
RAMP_MIN_V_PER_S = 1.0  # the unit's own minimum (RUPMIN, RDWMIN); its maximum is 100
RAMP_MAX_V_PER_S = 100.0


def _set_value(board: int, ch: int, par: str, value: float) -> str:
    return (f"$BD:{board:02d},CMD:SET,CH:{ch},PAR:{par},"
            f"VAL:{value:.{SETTINGS[par].decimals}f}")


def is_allowed(board: int, cmd: str) -> bool:
    """A fixed command, or one setting of one channel with a plain number. No sign, no exponent,
    no line break, no CH:4; MAXV, RUP, RDW, PDWN and the rest are not in the pattern."""
    if cmd in allowed_commands(board):
        return True
    params = "|".join(SETTINGS)
    return re.fullmatch(rf"\$BD:{board:02d},CMD:SET,CH:[0-3],PAR:({params}),VAL:[0-9]{{1,4}}(\.[0-9]{{1,3}})?",
                        cmd) is not None


class ForbiddenCommandError(ValueError):
    """A command outside allowed_commands(): a bug in this file, never retried."""


class ReplyError(Exception):
    """The unit answered, but not with a usable value. `code` = its error field if it gave one."""

    def __init__(self, code: str, text: str):
        super().__init__(f"{code}: {text!r}")
        self.code = code


class DeviceSilentError(Exception):
    """No complete reply line: the unit is gone, and a late reply must not be taken for the next."""


class IdentityError(Exception):
    """The unit on the port is not the configured one."""


def send_command(port, board: int, cmd: str) -> None:
    """The only place that writes to the serial port."""
    if not is_allowed(board, cmd):
        raise ForbiddenCommandError(f"refused to send {cmd!r}")
    port.reset_input_buffer()
    port.write((cmd + "\r\n").encode("ascii"))


def query(port, board: int, cmd: str) -> str | None:
    """Send one command and return its VAL (None for a plain CMD:OK).

    Real replies (TODO 82 §2): #BD:00,CMD:OK,VAL:N1408  #BD:00,CMD:OK  #BD:00,LOC:ERR
    """
    send_command(port, board, cmd)
    raw = port.readline()
    if not raw.endswith(b"\n"):
        raise DeviceSilentError(f"no complete reply to {cmd} (got {raw!r})")
    line = raw.decode("ascii", "replace").strip()
    prefix = f"#BD:{board:02d},"
    if not line.startswith(prefix):
        raise ReplyError("malformed", line)
    body = line[len(prefix):]
    if body == "CMD:OK":
        return None
    if body.startswith("CMD:OK,VAL:"):
        return body[len("CMD:OK,VAL:"):]
    if body in ("CMD:ERR", "CH:ERR", "PAR:ERR", "VAL:ERR", "LOC:ERR"):
        raise ReplyError(body, line)
    raise ReplyError("malformed", line)


def status_flags(status: int) -> list[str]:
    return [name for i, name in enumerate(STATUS_BITS) if status >> i & 1]


def channel_state(status: int | None, vmon: float | None, vset: float | None) -> str:
    """One word for the channel, from this cycle's readback only.

    The ON bit alone is not enough: after SET OFF it stays set, without RDW, until VMON is about
    0.15 V (3-4 s on the real unit, TODO 82 §2), and it also ends a ramp up a little before VSET.
    So ON is only claimed with VMON near VSET. With a real under-voltage the unit sets UNV.
    """
    if status is None:
        return "UNKNOWN"
    if status & BIT_KILL:
        return "KILL"
    if status & BIT_ILK:
        return "INTERLOCK"
    if status & BIT_TRIP:
        return "TRIP"
    if status & BIT_ON:
        if status & BIT_RDW:
            return "RAMP_DOWN"
        if status & BIT_RUP:
            return "RAMP_UP"
        if status & BIT_UNV:
            return "ON"
        if vmon is None or vset is None or vmon < vset - UNDER_VOLTAGE_V:
            return "SETTLING"
        return "ON"
    if status & BIT_DIS:
        return "DISABLED"
    return "OFF"


def status_changes(previous: list | None, current: list) -> list[dict]:
    """What changed between two cycles, per channel. Says what happened, not who did it."""
    if previous is None:
        return []
    events = []
    for ch, (old, new) in enumerate(zip(previous, current)):
        if old is None or new is None or (old ^ new) & _EVENT_MASK == 0:
            continue
        events.append({
            "channel": ch,
            "status_before": old,
            "status_after": new,
            "gone": status_flags(old & ~new & _EVENT_MASK),
            "new": status_flags(new & ~old & _EVENT_MASK),
        })
    return events


# --- reading -------------------------------------------------------------------------------------


@dataclass
class Reading:
    board: dict = field(default_factory=dict)
    channels: list = field(default_factory=list)


def _no_warn(*_args, **_kwargs) -> bool:
    return True


def poll_cycle(port, board: int, warn=_no_warn) -> Reading:
    """One full read. A bad reply leaves that field None (unknown, never 0); silence raises."""
    reading = Reading(channels=[{} for _ in CHANNELS])
    for par, name in BOARD_QUERIES:
        try:
            reading.board[name] = query(port, board, _mon_board(board, par))
        except ReplyError as e:
            warn(f"reply:{par}", f"Bad reply to {par}: {e}")
            reading.board[name] = None
    for par, name, parse in CHANNEL_QUERIES:
        values = [None] * len(CHANNELS)
        try:
            text = query(port, board, _mon_channel(board, ALL_CHANNELS, par))
            parts = (text or "").split(";")
            if len(parts) != len(CHANNELS):
                raise ValueError(text)
            values = [parse(p) for p in parts]
        except (ReplyError, ValueError) as e:
            warn(f"reply:{par}", f"Bad reply to {par}: {e}")
        for ch, value in zip(reading.channels, values):
            ch[name] = value
    return reading


def identify(port, cfg: "Config") -> dict:
    """Read the board identity; refuse a unit other than the configured one. A serial port path can
    change hands (the N1408's USB chip has no serial number), and this program can switch HV."""
    info = {}
    for par, name in IDENTITY_QUERIES:
        try:
            info[name] = query(port, cfg.board, _mon_board(cfg.board, par))
        except ReplyError as e:
            raise IdentityError(f"cannot read {par}: {e}") from e
    if info["name"] != "N1408" or info["serial"] != cfg.expected_serial:
        raise IdentityError(
            f"port {cfg.serial_port} has {info['name']} SN {info['serial']}, "
            f"expected N1408 SN {cfg.expected_serial}"
        )
    return info


def format_lines(device: str, reading: Reading) -> list[str]:
    """Line-protocol lines for one cycle. Fields whose read failed are omitted."""
    lines = []
    for ch, values in enumerate(reading.channels):
        fields = [f"{name}={fmt_float(values[name])}"
                  for name in ("vmon", "imon", "vset", "iset", "maxv", "rup", "rdw", "trip")
                  if values.get(name) is not None]
        status = values.get("status")
        if status is not None:
            fields.append(f"status={status}i")
            fields.append(f"output_on={'true' if status & BIT_ON else 'false'}")
        if fields:
            lines.append(f"n1408,device={escape_tag(device)},channel={ch} {','.join(fields)}")
    return lines


# --- ON / OFF ------------------------------------------------------------------------------------


def execute_command(port, cfg: "Config", action: str, channel: int, clock=time.monotonic,
                    sleep=time.sleep) -> dict:
    """ON or OFF of one channel, judged by the readback.

    result: done | already (nothing sent) | refused (nothing sent) | failed.
    The unit answers CMD:OK to SET ON even with the channel switch at OFF and does nothing
    (TODO 82 §2), so CMD:OK proves nothing: what counts is STAT afterwards.
    """
    result = {"action": action, "channel": channel, "ok": False, "result": "refused",
              "status_before": None, "status_after": None, "vset": None, "vmon": None}

    def finish(outcome: str, message: str) -> dict:
        result["result"] = outcome
        result["message"] = message
        result["ok"] = outcome in ("done", "already")
        return result

    if action not in ("on", "off"):
        return finish("refused", f"unknown action {action!r}")
    ch_cfg = cfg.channels.get(channel)
    if ch_cfg is None:
        return finish("refused", f"no channel {channel}")
    if not ch_cfg.control:
        return finish("refused", f"channel {channel} ({ch_cfg.name}) is not enabled for control "
                                 "in the config")

    board = cfg.board
    stat_cmd = _mon_channel(board, channel, "STAT")
    try:
        control = query(port, board, _mon_board(board, "BDCTR"))
        before = _parse_int(query(port, board, stat_cmd) or "")
        result["status_before"] = before
        result["vset"] = _parse_number(query(port, board, _mon_channel(board, channel, "VSET")) or "")
        result["vmon"] = _parse_number(query(port, board, _mon_channel(board, channel, "VMON")) or "")
    except (ReplyError, ValueError) as e:
        return finish("refused", f"cannot read the channel before the command: {e}")

    if control != "REMOTE":
        return finish("refused", f"the panel is {control}: set CONTROL to REMOTE on the N1408")
    if action == "on":
        try:
            interlock = query(port, board, _mon_board(board, "BDILK"))
        except ReplyError as e:
            return finish("refused", f"cannot read the interlock: {e}")
        if interlock != "NO":
            return finish("refused", f"board interlock is {interlock}")
        if before & BIT_DIS:
            return finish("refused", "the channel switch is at OFF: set it to HV_EN on the panel")
        for bit, why in ((BIT_KILL, "the channel is in KILL"), (BIT_ILK, "the channel is in "
                         "INTERLOCK"), (BIT_NOCAL, "calibration error")):
            if before & bit:
                return finish("refused", why)
        if before & BIT_ON and not before & BIT_RDW:
            result["status_after"] = before
            return finish("already", "already ON")
    elif not before & BIT_ON:
        result["status_after"] = before
        return finish("already", "already OFF")

    try:
        query(port, board, _set_power(board, channel, action))
    except ReplyError as e:
        return finish("failed", f"the unit refused SET {action.upper()}: {e.code}")

    def confirmed(status: int) -> bool:
        if action == "on":
            return bool(status & BIT_ON)
        return bool(status & BIT_RDW) or not status & BIT_ON

    deadline = clock() + cfg.readback_timeout_sec
    while True:
        sleep(0.3)
        try:
            status = _parse_int(query(port, board, stat_cmd) or "")
        except (ReplyError, ValueError) as e:
            log.warning("Bad STAT after SET %s ch%d: %s", action.upper(), channel, e)
            status = None
        if status is not None:
            result["status_after"] = status
            if confirmed(status):
                return finish("done", f"{action.upper()} confirmed by STAT "
                                      f"({'|'.join(status_flags(status)) or 'no flags'})")
        if clock() >= deadline:
            return finish("failed", f"the unit replied CMD:OK but STAT is still "
                                    f"{result['status_after']} after "
                                    f"{cfg.readback_timeout_sec:.0f} s")


def execute_setting(port, cfg: "Config", channel: int, param, value, expected) -> dict:
    """Change one setting of one channel, judged by the readback.

    `expected` is the value the operator saw: if the unit has another one now, someone else changed
    it and nothing is sent. Limits come from the unit's own readback and refuse, never clamp.
    """
    number = isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    result = {"action": "set", "channel": channel, "param": str(param),
              "requested": value if number else str(value), "ok": False, "result": "refused",
              "value_before": None, "value_after": None, "status_before": None}

    def finish(outcome: str, message: str) -> dict:
        result["result"] = outcome
        result["message"] = message
        result["ok"] = outcome in ("done", "already")
        return result

    setting = SETTINGS.get(param)
    if setting is None:
        return finish("refused", f"{param} cannot be changed from here, only {', '.join(SETTINGS)}")
    ch_cfg = cfg.channels.get(channel)
    if ch_cfg is None:
        return finish("refused", f"no channel {channel}")
    if not ch_cfg.control:
        return finish("refused", f"channel {channel} ({ch_cfg.name}) is not enabled for control "
                                 "in the config")
    if not number:
        return finish("refused", f"{value!r} is not a number")
    if not (isinstance(expected, (int, float)) and not isinstance(expected, bool)
            and math.isfinite(expected)):
        return finish("refused", "the value you saw is missing: read the channel first")
    if value < 0:
        return finish("refused", f"{value:g} is negative")
    scaled = value * 10 ** setting.decimals
    if abs(scaled - round(scaled)) > 1e-6:
        if setting.decimals == 0:
            return finish("refused", f"{param} must be a whole number: the unit would cut {value:g}")
        return finish("refused", f"{param} keeps {setting.decimals} digits after the point")

    board = cfg.board

    def read(par: str) -> float:
        return _parse_number(query(port, board, _mon_channel(board, channel, par)) or "")

    try:
        control = query(port, board, _mon_board(board, "BDCTR"))
        if control != "REMOTE":
            return finish("refused", f"the panel is {control}: set CONTROL to REMOTE on the N1408")
        status = _parse_int(query(port, board, _mon_channel(board, channel, "STAT")) or "")
        result["status_before"] = status
        before = read(param)
        result["value_before"] = before
        if abs(before - expected) > setting.tolerance:
            return finish("refused", f"{param} is {before:g} now, you saw {expected:g}: someone "
                                     "changed it. Check it and try again")
        if param == "VSET":
            maxv = read("MAXV")
            if value > maxv:
                return finish("refused", f"VSET {value:g} V is above MAXV {maxv:g} V. MAXV is set "
                                         "on the panel (LOCAL)")
        elif param == "ISET":
            imax = read("IMAX")
            if value > imax:
                return finish("refused", f"ISET {value:g} µA is above the unit's {imax:g} µA")
            if status & BIT_ON:
                imon = abs(read("IMON"))
                if value <= imon:
                    return finish("refused", f"ISET {value:g} µA is not above the present current "
                                             f"{imon:g} µA: the channel would trip")
        elif param == "TRIP":
            if value > TRIP_MAX_S:
                return finish("refused", f"TRIP above {TRIP_MAX_S} s is refused: 1000 s means an "
                                         "over-current never trips")
        elif not RAMP_MIN_V_PER_S <= value <= cfg.ramp_max_v_per_s:
            return finish("refused", f"{param} {value:g} V/s is outside {RAMP_MIN_V_PER_S:g} to "
                                     f"{cfg.ramp_max_v_per_s:g} V/s (ramp_max_v_per_s)")
    except (ReplyError, ValueError) as e:
        return finish("refused", f"cannot read the channel before the command: {e}")

    if abs(value - before) <= setting.tolerance:
        result["value_after"] = before
        return finish("already", f"{param} is already {before:g}")

    try:
        query(port, board, _set_value(board, channel, param, value))
    except ReplyError as e:
        return finish("failed", f"the unit refused {param} {value:g}: {e.code}")
    try:
        after = read(param)
    except (ReplyError, ValueError) as e:
        return finish("failed", f"sent {param} {value:g} but cannot read it back: {e}")
    result["value_after"] = after
    if abs(after - value) > setting.tolerance:
        return finish("failed", f"the unit replied CMD:OK but {param} reads {after:g}, not {value:g}")
    return finish("done", f"{param} {before:g} -> {after:g}")


# --- the DAQ: only asked whether a run is going ----------------------------------------------------

OPERATOR_TIMEOUT_S = 1.0


def daq_state(get, url: str) -> dict:
    """From the Operator's /api/status. Tune Up is not a run: the Si is biased and tuned in it.
    An Operator that does not answer is reported, not guessed: no DAQ running is the usual reason,
    and HV work must not depend on it."""
    state = {"reachable": False, "running": False, "run": None, "state": None, "tuneup": False}
    if not url:
        return state
    try:
        reply = get(url + "/api/status", timeout=OPERATOR_TIMEOUT_S)
        body = reply.json() if reply.ok else None
    except (requests.RequestException, ValueError):
        return state
    if not isinstance(body, dict) or "system_state" not in body:
        return state
    run_info = body.get("run_info") if isinstance(body.get("run_info"), dict) else {}
    state["reachable"] = True
    state["state"] = body["system_state"]
    state["tuneup"] = body.get("tuneup_mode") is True
    state["run"] = run_info.get("run_number")
    run_open = run_info.get("status") == "running"
    state["running"] = (run_open or state["state"] == "Running") and not state["tuneup"]
    return state


# --- operator session ----------------------------------------------------------------------------


class ControlSession:
    """One holder at a time. Acquiring always takes over and names the previous holder (as in
    tpcdaq-rs): an accident guard, not access control. Only the HTTP threads touch this."""

    def __init__(self, idle_timeout_s: float, clock=time.monotonic):
        self._idle = idle_timeout_s
        self._clock = clock
        self._lock = threading.Lock()
        self._token = None
        self._operator = None
        self._last_used = 0.0

    def _expire(self) -> None:
        if self._token is not None and self._clock() - self._last_used > self._idle:
            self._token = None
            self._operator = None

    def acquire(self, operator: str) -> tuple[str, str | None]:
        with self._lock:
            self._expire()
            preempted = self._operator
            self._token = secrets.token_hex(16)
            self._operator = operator
            self._last_used = self._clock()
            return self._token, preempted

    def authorize(self, token) -> str | None:
        with self._lock:
            self._expire()
            if self._token is None or token != self._token:
                return None
            self._last_used = self._clock()
            return self._operator

    def release(self, token) -> bool:
        with self._lock:
            self._expire()
            if self._token is None or token != self._token:
                return False
            self._token = None
            self._operator = None
            return True

    def holder(self) -> str | None:
        with self._lock:
            self._expire()
            return self._operator


def read_passphrase(path: str) -> str | None:
    """Read at every acquire, so creating or changing the file needs no restart. None = read-only."""
    try:
        with open(path, encoding="utf-8") as f:
            return f.read().strip() or None
    except OSError:
        return None


# --- audit log -----------------------------------------------------------------------------------


class AuditLog:
    """Append-only JSON lines: every command and status change, whatever its result."""

    def __init__(self, path: str, warn):
        self._path = path
        self._warn = warn
        self._lock = threading.Lock()
        self.recent = collections.deque(maxlen=30)

    def record(self, row: dict) -> None:
        row = {"time": time.strftime("%Y-%m-%dT%H:%M:%S%z"), **row}
        line = json.dumps(row, ensure_ascii=False)
        with self._lock:
            self.recent.append(row)
            if not self._path:
                return
            try:
                os.makedirs(os.path.dirname(self._path), exist_ok=True)
                with open(self._path, "a", encoding="utf-8") as f:
                    f.write(line + "\n")
            except OSError as e:
                self._warn("audit", f"Cannot write the audit log {self._path}: {e}", logging.ERROR)


# --- config --------------------------------------------------------------------------------------


@dataclass(frozen=True)
class ChannelConfig:
    name: str
    control: bool = False  # ON/OFF allowed from the web page; off until someone says so


@dataclass(frozen=True)
class Config:
    serial_port: str
    expected_serial: str
    device_name: str
    influx_url: str
    passphrase_file: str
    audit_log: str
    channels: dict
    board: int = 0
    poll_interval_sec: float = 2.0
    reply_timeout_sec: float = 1.0
    readback_timeout_sec: float = 5.0
    retry_delay_sec: float = 5.0
    control_idle_timeout_sec: float = 600.0
    http_host: str = "0.0.0.0"
    http_port: int = 8095
    operator_url: str = "http://localhost:9091"
    ramp_max_v_per_s: float = 10.0


_REQUIRED_KEYS = ("serial_port", "expected_serial", "device_name", "influx_url", "passphrase_file",
                  "audit_log", "channels")
_NUMBER_KEYS = ("poll_interval_sec", "reply_timeout_sec", "readback_timeout_sec", "retry_delay_sec",
                "control_idle_timeout_sec", "ramp_max_v_per_s")


def config_from_dict(raw) -> Config:
    if not isinstance(raw, dict):
        raise ValueError("config must be a mapping")
    missing = [k for k in _REQUIRED_KEYS if k not in raw]
    if missing:
        raise ValueError(f"missing config key(s): {', '.join(missing)}")
    raw_channels = raw["channels"]
    if not isinstance(raw_channels, dict) or set(raw_channels) != set(CHANNELS):
        raise ValueError("channels must list exactly 0, 1, 2 and 3")
    channels = {}
    for ch in CHANNELS:
        entry = raw_channels[ch] or {}
        channels[ch] = ChannelConfig(str(entry.get("name", f"ch{ch}")), entry.get("control") is True)
    values = {k: raw[k] for k in ("serial_port", "device_name", "influx_url", "passphrase_file",
                                  "audit_log", "board", "http_host", "http_port", "operator_url")
              if k in raw}
    values.update({k: float(raw[k]) for k in _NUMBER_KEYS if k in raw})
    if not RAMP_MIN_V_PER_S <= values.get("ramp_max_v_per_s", 10.0) <= RAMP_MAX_V_PER_S:
        raise ValueError(f"ramp_max_v_per_s must be {RAMP_MIN_V_PER_S:g} to {RAMP_MAX_V_PER_S:g}")
    return Config(expected_serial=str(raw["expected_serial"]), channels=channels, **values)


def load_config(path: str) -> Config:
    with open(path, encoding="utf-8") as f:
        return config_from_dict(yaml.safe_load(f))


# --- serial owner: polls, and runs the ON/OFF commands one at a time ------------------------------


def open_serial_port(cfg: Config):
    # Lazy import: pyserial is installed on the DAQ host; the unit tests never open a port.
    import serial

    return serial.Serial(
        port=cfg.serial_port,
        baudrate=9600,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        xonxoff=True,
        timeout=cfg.reply_timeout_sec,
        write_timeout=1.0,
        exclusive=True,
    )


@dataclass
class Command:
    action: str  # "on", "off", "all_off", "set"
    channel: int | None
    operator: str
    client: str
    param: object = None  # "set": as received, execute_setting() checks them
    value: object = None
    expected: object = None
    daq_reachable: bool | None = None  # None = not asked (OFF works whatever the DAQ does)
    reply: queue.Queue = field(default_factory=lambda: queue.Queue(maxsize=1))


class Worker(threading.Thread):
    """The only thread that touches the serial port. HTTP threads send it Commands and read
    `status`, a dict replaced whole on every change."""

    def __init__(self, cfg: Config, open_port, post, audit: AuditLog, warn):
        super().__init__(name="n1408-serial", daemon=True)
        self._cfg = cfg
        self._open_port = open_port
        self._post = post
        self._audit = audit
        self._warn = warn
        self._halt = threading.Event()
        self._port = None
        self._info = None
        self._previous = None
        self._next_poll = 0.0
        self._retry_at = 0.0
        self.commands: queue.Queue = queue.Queue()
        self.status = self._offline_status("not connected yet")

    def stop(self) -> None:
        self._halt.set()

    def _channel_rows(self, reading: Reading | None) -> list[dict]:
        rows = []
        for ch in CHANNELS:
            values = reading.channels[ch] if reading else {}
            status = values.get("status")
            rows.append({
                "ch": ch,
                "name": self._cfg.channels[ch].name,
                "control_allowed": self._cfg.channels[ch].control,
                "state": channel_state(status, values.get("vmon"), values.get("vset")),
                "flags": status_flags(status) if status is not None else [],
                **{name: values.get(name) for _, name, _ in CHANNEL_QUERIES},
            })
        return rows

    def _offline_status(self, error: str) -> dict:
        return {"connected": False, "error": error, "updated_at": None, "board": {},
                "channels": self._channel_rows(None)}

    def run(self) -> None:
        try:
            while not self._halt.is_set():
                now = time.monotonic()
                if self._port is None and now >= self._retry_at:
                    self._connect()
                    now = time.monotonic()
                due = self._next_poll if self._port is not None else self._retry_at
                try:
                    cmd = self.commands.get(timeout=min(max(due - now, 0.0), 0.2) or 0.001)
                except queue.Empty:
                    cmd = None
                if cmd is not None:
                    self._handle(cmd)
                elif self._port is not None and time.monotonic() >= self._next_poll:
                    self._poll()
        finally:
            _close_quietly(self._port)

    def _connect(self) -> None:
        try:
            self._port = self._open_port(self._cfg)
            self._info = identify(self._port, self._cfg)
        except Exception as e:  # noqa: BLE001 -- SerialException/OSError/IdentityError/silence/...
            self._lost(e)
            return
        self._previous = None
        self._next_poll = 0.0
        self._warn("serial:opened", f"Connected: {self._info}", logging.INFO)

    def _lost(self, error: Exception) -> None:
        self._warn(f"serial:{type(error).__name__}:{error}",
                   f"Serial problem ({type(error).__name__}: {error}) -- retry in "
                   f"{self._cfg.retry_delay_sec:.0f} s", logging.ERROR)
        _close_quietly(self._port)
        self._port = None
        self._retry_at = time.monotonic() + self._cfg.retry_delay_sec
        self.status = self._offline_status(f"{type(error).__name__}: {error}")

    def _poll(self) -> None:
        start = time.monotonic()
        try:
            reading = poll_cycle(self._port, self._cfg.board, self._warn)
        except ForbiddenCommandError:
            raise  # a bug in this file, never retried
        except Exception as e:  # noqa: BLE001 -- see _connect
            self._lost(e)
            return
        self.status = {
            "connected": True,
            "error": None,
            "updated_at": time.time(),
            "board": {**self._info, **reading.board},
            "channels": self._channel_rows(reading),
        }
        current = [c["status"] for c in reading.channels]
        for event in status_changes(self._previous, current):
            self._record({"action": "status_change", "operator": None, "client": None,
                          "result": "seen", **event})
        self._previous = current
        lines = format_lines(self._cfg.device_name, reading)
        if lines:
            write_influx(self._cfg.influx_url, lines, self._post, self._warn)
        self._next_poll = start + max(self._cfg.poll_interval_sec, time.monotonic() - start + 0.05)

    def _record(self, row: dict) -> None:
        self._audit.record(row)
        tags = f"device={escape_tag(self._cfg.device_name)},action={escape_tag(row['action'])}"
        if row.get("channel") is not None:
            tags += f",channel={row['channel']}"
        detail = row.get("message") or (
            f"new: {row.get('new')}, gone: {row.get('gone')}" if "new" in row else "")
        fields = (f'operator="{escape_field_string(row.get("operator") or "")}",'
                  f'result="{escape_field_string(row["result"])}",'
                  f'message="{escape_field_string(detail)}"')
        write_influx(self._cfg.influx_url, [f"n1408_event,{tags} {fields}"], self._post, self._warn)

    def _handle(self, cmd: Command) -> None:
        if self._port is None:
            cmd.reply.put({"ok": False, "result": "offline",
                           "message": f"the N1408 is not connected ({self.status['error']})"})
            return
        channels = (
            [c for c in CHANNELS if self._cfg.channels[c].control]
            if cmd.action == "all_off" else [cmd.channel]
        )
        action = "off" if cmd.action == "all_off" else cmd.action
        who = {"operator": cmd.operator, "client": cmd.client, "daq_reachable": cmd.daq_reachable}
        results = []
        for ch in channels:
            try:
                if action == "set":
                    result = execute_setting(self._port, self._cfg, ch, cmd.param, cmd.value,
                                             cmd.expected)
                else:
                    result = execute_command(self._port, self._cfg, action, ch)
            except ForbiddenCommandError:
                raise  # a bug in this file, never retried
            except Exception as e:  # noqa: BLE001 -- see _connect
                result = {"action": action, "channel": ch, "ok": False, "result": "failed",
                          "message": f"lost the unit during the command, the outcome is unknown: "
                                     f"{type(e).__name__}: {e}"}
                self._record({**result, **who})
                results.append(result)
                self._lost(e)
                break
            self._record({**result, **who})
            results.append(result)
            if self._previous is not None and result.get("status_after") is not None \
                    and ch in CHANNELS:
                self._previous[ch] = result["status_after"]
        self._next_poll = 0.0
        if cmd.action == "all_off":
            cmd.reply.put({"ok": all(r["ok"] for r in results), "results": results,
                           "result": "done" if all(r["ok"] for r in results) else "failed"})
        else:
            cmd.reply.put(results[0])


# --- HTTP ----------------------------------------------------------------------------------------

_STATUS_FOR_RESULT = {"done": 200, "already": 200, "refused": 409, "failed": 502, "offline": 503}
_CHANNEL_PATH = re.compile(r"/api/channel/(\d+)/(on|off|set)")
_MAX_BODY = 64 * 1024
_PAGE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "n1408_control.html")


class Handler(BaseHTTPRequestHandler):
    server_version = "n1408_control"

    def log_message(self, fmt, *args):  # the audit log has what matters
        log.debug("%s %s", self.address_string(), fmt % args)

    def _send(self, code: int, body: bytes, content_type: str) -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _json(self, code: int, obj: dict) -> None:
        self._send(code, json.dumps(obj, ensure_ascii=False).encode("utf-8"), "application/json")

    def _body(self) -> dict:
        length = int(self.headers.get("Content-Length") or 0)
        if not 0 < length <= _MAX_BODY:
            return {}
        try:
            body = json.loads(self.rfile.read(length))
        except ValueError:
            return {}
        return body if isinstance(body, dict) else {}

    def do_GET(self):
        app = self.server.app
        if self.path == "/api/status":
            self._json(200, app.status())
        elif self.path == "/":
            try:
                with open(_PAGE, "rb") as f:
                    self._send(200, f.read(), "text/html; charset=utf-8")
            except OSError as e:
                self._json(500, {"message": f"cannot read the page: {e}"})
        else:
            self._json(404, {"message": "not found"})

    def do_POST(self):
        app = self.server.app
        body = self._body()
        client = self.client_address[0]
        if self.path == "/api/control/acquire":
            self._json(*app.acquire(body.get("operator"), body.get("passphrase"), client))
        elif self.path == "/api/control/release":
            released = app.session.release(body.get("token"))
            self._json(200, {"released": released})
        elif self.path == "/api/all_off":
            self._json(*app.command("all_off", None, body.get("token"), client))
        else:
            match = _CHANNEL_PATH.fullmatch(self.path)
            if not match:
                self._json(404, {"message": "not found"})
                return
            self._json(*app.command(match[2], int(match[1]), body.get("token"), client,
                                    param=body.get("param"), value=body.get("value"),
                                    expected=body.get("expected")))


class Server(ThreadingHTTPServer):
    daemon_threads = True


class App:
    """Worker + HTTP server."""

    COMMAND_TIMEOUT_S = 60.0

    def __init__(self, cfg: Config, open_port=open_serial_port, post=None, get=None):
        self.cfg = cfg
        self._get = get or requests.get
        warn = WarnLimiter(log)
        self.audit = AuditLog(cfg.audit_log, warn)
        self.session = ControlSession(cfg.control_idle_timeout_sec)
        self.worker = Worker(cfg, open_port, post or requests.post, self.audit, warn)
        self._server = Server((cfg.http_host, cfg.http_port), Handler)
        self._server.app = self
        self.http_port = self._server.server_address[1]
        self._http_thread = threading.Thread(target=self._server.serve_forever, name="n1408-http",
                                             daemon=True)

    def start(self) -> None:
        self.worker.start()
        self._http_thread.start()

    def stop(self) -> None:
        self.worker.stop()
        self.worker.join(timeout=5.0)
        self._server.shutdown()
        self._server.server_close()

    def status(self) -> dict:
        status = dict(self.worker.status)
        updated = status.get("updated_at")
        status["age_s"] = None if updated is None else round(time.time() - updated, 1)
        status["control_enabled"] = read_passphrase(self.cfg.passphrase_file) is not None
        status["holder"] = self.session.holder()
        status["daq"] = daq_state(self._get, self.cfg.operator_url)
        status["idle_timeout_s"] = self.cfg.control_idle_timeout_sec
        status["ramp_max_v_per_s"] = self.cfg.ramp_max_v_per_s
        status["events"] = list(self.audit.recent)
        return status

    def acquire(self, operator, passphrase, client: str) -> tuple[int, dict]:
        expected = read_passphrase(self.cfg.passphrase_file)
        if expected is None:
            return 503, {"message": "control is off: no passphrase file on the host "
                                    f"({self.cfg.passphrase_file})"}
        operator = operator.strip() if isinstance(operator, str) else ""
        if not operator:
            return 400, {"message": "write your name"}
        if passphrase != expected:
            self.audit.record({"action": "acquire", "operator": operator, "client": client,
                               "result": "refused", "message": "wrong passphrase"})
            return 403, {"message": "wrong passphrase"}
        token, preempted = self.session.acquire(operator)
        self.audit.record({"action": "acquire", "operator": operator, "client": client,
                           "result": "done", "preempted": preempted})
        return 200, {"token": token, "preempted": preempted}

    def command(self, action: str, channel, token, client: str, param=None, value=None,
                expected=None) -> tuple[int, dict]:
        operator = self.session.authorize(token)
        if operator is None:
            return 401, {"ok": False, "result": "refused",
                         "message": "no control: enter your name and the passphrase"}
        daq_reachable = None
        if action in ("on", "set"):
            daq = daq_state(self._get, self.cfg.operator_url)
            daq_reachable = daq["reachable"]
            if daq["running"]:
                run = f"run {daq['run']}" if daq["run"] is not None else "a run"
                message = f"{run} is in progress: only OFF works during a run"
                self.audit.record({"action": action, "channel": channel, "param": param,
                                   "operator": operator, "client": client, "result": "refused",
                                   "message": message})
                return 409, {"ok": False, "result": "refused", "message": message}
        cmd = Command(action, channel, operator, client, param, value, expected, daq_reachable)
        self.worker.commands.put(cmd)
        try:
            result = cmd.reply.get(timeout=self.COMMAND_TIMEOUT_S)
        except queue.Empty:
            return 504, {"ok": False, "result": "failed",
                         "message": "no answer from the serial thread, the outcome is unknown"}
        if daq_reachable is False:
            result = {**result, "message": f"{result.get('message', '')} (DAQ state unknown: the "
                                           "Operator does not answer)"}
        return _STATUS_FOR_RESULT.get(result["result"], 500), result


def main(argv=None) -> int:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
    parser = argparse.ArgumentParser(description="CAEN N1408 monitor + ON/OFF web page")
    parser.add_argument("--config", default="n1408_control_config.yaml", help="YAML config file")
    args = parser.parse_args(argv)
    try:
        cfg = load_config(args.config)
    except (OSError, ValueError, yaml.YAMLError) as e:
        log.error("Cannot load config %s: %s", args.config, e)
        return 2

    stop = StopFlag()

    def on_signal(signum, _frame):
        stop.set(signal.Signals(signum).name)

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)

    app = App(cfg)
    app.start()
    allowed = [f"{c}:{cfg.channels[c].name}" for c in CHANNELS if cfg.channels[c].control]
    log.info("Starting N1408 control: port=%s SN=%s http=%s:%d control channels=%s passphrase "
             "file=%s (%s)", cfg.serial_port, cfg.expected_serial, cfg.http_host, app.http_port,
             allowed or "none", cfg.passphrase_file,
             "present" if read_passphrase(cfg.passphrase_file) else "absent: read-only")
    try:
        while not stop.is_set():
            stop.wait(0.5)
            if not app.worker.is_alive():
                log.error("The serial thread died; exiting so systemd restarts this")
                return 1
    finally:
        log.info("Stopping (%s)", stop.reason or "exit")
        app.stop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
