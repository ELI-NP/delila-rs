#!/usr/bin/env python3
"""Mesytec MHV-4 HV monitor (READ-ONLY): USB serial -> InfluxDB line protocol.

Spec: TODO/75_mesytec_hv_monitor.md
  §2   datasheet MHV-4_V40_02 p.6/9 "Remote control via USB" (9600 8N1, echo, read commands)
  §2.1 replies captured from the real unit (FW V4.2, pf-qaq, 2026-10-22)
  §2.2 no ON/OFF state is kept in software: every field comes from the current cycle's reads

Safety: every byte written to the serial port goes through send_command(), which accepts only the
21 read commands in ALLOWED_COMMANDS (RU/RUL/RI/RIL/RP c, c = 0-3, and RRA). The set commands
(ON, OFF, SU, SUL, SIL, SP, AS, STC, STO, STS, SRA) cannot be sent. RUP replies ERROR! on V4.2 and
RT/RTC are unused (no temperature sensors), so they are not allowed either.

Usage:
    python3 -u mhv4_monitor.py --config mhv4_config.yaml
"""

from __future__ import annotations

import argparse
import logging
import math
import re
import signal
import time
from dataclasses import dataclass, field

import requests
import yaml

log = logging.getLogger("mhv4_monitor")

# --- protocol (TODO 75 §2.1) -------------------------------------------------------------------

CHANNELS = (0, 1, 2, 3)
PROMPT = b"MHV-4>"
ERROR_LINE = "ERROR!"

# Per-channel read commands, in the order they are sent, and the InfluxDB field each one fills.
CHANNEL_QUERIES = (
    ("RU", "vmon"),
    ("RUL", "vlimit"),
    ("RI", "imon"),
    ("RIL", "ilimit"),
    ("RP", "polarity"),
)
RAMP_QUERY = "RRA"

# The ONLY strings send_command() will write (plus the terminating CR). Explicit set, no regex:
# nothing like "RU 0\rON 0" or "RU 0 " can slip through.
ALLOWED_COMMANDS = frozenset(
    [f"{prefix} {c}" for prefix, _ in CHANNEL_QUERIES for c in CHANNELS] + [RAMP_QUERY]
)

# Expected value line for each command, from the real replies (TODO 75 §2.1):
#   RU 0  -> "U0: +0.0"              RUL 0 -> "Ulim 0: +27.50"       RI 0 -> "I 0: -0.004 uA"
#   RIL 0 -> "Ilim 0: 20.000 uA"     RP 0  -> "Polarity 0: positive" RRA  -> "MHV ramp speed: 5 V/s"
# label: text before the last ':' compared case- and whitespace-insensitively ({c} = channel).
# units: the unit token accepted after the number ("" = none). None = text value (polarity).
# Only the units actually seen on the device are accepted; anything else is rejected, not converted.
REPLY_FORMAT = {
    "RU": ("u{c}", ("",)),
    "RUL": ("ulim{c}", ("",)),
    "RI": ("i{c}", ("uA",)),
    "RIL": ("ilim{c}", ("uA",)),
    "RP": ("polarity{c}", None),
    "RRA": ("mhvrampspeed", ("V/s",)),
}
POLARITIES = ("positive", "negative")

# A signed decimal number, then an optional unit token. Digit counts are not assumed
# ("+27.50" and "+110.2" both occur on the same unit).
_NUMBER_RE = re.compile(r"([+-]?(?:\d+(?:\.\d*)?|\.\d+))\s*(\S*)")

READ_SLICE_S = 0.2  # pyserial per-read timeout; bounds how long one read() call can block
WRITE_TIMEOUT_S = 1.0
MAX_CONSECUTIVE_TIMEOUTS = 3  # a silent device aborts the cycle instead of 21 x reply timeout
RETRY_DELAY_S = 5.0  # serial port reopen interval
INFLUX_TIMEOUT_S = 2.0
WARN_REPEAT_INTERVAL_S = 300.0  # identical warnings are logged at most once per this interval


class ForbiddenCommandError(ValueError):
    """Raised before any byte is written when a command is not in ALLOWED_COMMANDS."""


class ReplyError(Exception):
    """A reply that cannot be turned into a value. kind: timeout | device-error | malformed."""

    def __init__(self, kind: str, detail: str):
        super().__init__(f"{kind}: {detail}")
        self.kind = kind


class DeviceSilentError(Exception):
    """Several commands in a row got no reply: treat like a lost port (close, retry)."""


def send_command(port, cmd: str) -> None:
    """The only function in this module that writes to the serial port."""
    if not isinstance(cmd, str) or cmd not in ALLOWED_COMMANDS:
        raise ForbiddenCommandError(f"refusing to send {cmd!r}: not a whitelisted read command")
    port.reset_input_buffer()  # drop anything stale so the reply we read belongs to this command
    port.write(cmd.encode("ascii") + b"\r")


def split_reply_lines(segment: bytes) -> list[str]:
    """Split a reply on the device's LF/CR line ends; drop blank lines."""
    text = segment.decode("ascii", errors="replace")
    return [ln.strip() for ln in re.split(r"[\r\n]+", text) if ln.strip()]


def read_reply(port, cmd: str, timeout_s: float, clock=time.monotonic) -> list[str]:
    """Read until a prompt-terminated segment that starts with the echo of `cmd`.

    Returns the lines between the echo and the prompt. A segment that does not start with the
    echo (the late tail of an earlier, timed-out reply) is discarded and reading continues.
    Raises ReplyError("timeout") if no matching segment arrives within timeout_s.
    """
    buf = bytearray()
    stale = 0
    deadline = clock() + timeout_s
    while True:
        idx = buf.find(PROMPT)
        while idx >= 0:
            lines = split_reply_lines(bytes(buf[:idx]))
            del buf[: idx + len(PROMPT)]
            if lines and lines[0] == cmd:
                return lines[1:]
            stale += 1
            idx = buf.find(PROMPT)
        if clock() >= deadline:
            raise ReplyError(
                "timeout",
                f"no complete reply within {timeout_s:.1f} s "
                f"(partial={bytes(buf)!r}, stale segments={stale})",
            )
        buf += port.read(max(1, port.in_waiting))


def _normalize_label(label: str) -> str:
    return "".join(label.split()).lower()


def parse_value(cmd: str, body: list[str]) -> float | str:
    """Turn the lines between echo and prompt into a value: float, or str for polarity."""
    prefix, _, channel = cmd.partition(" ")
    label_template, units = REPLY_FORMAT[prefix]
    if ERROR_LINE in body:
        raise ReplyError("device-error", f"device replied {ERROR_LINE}")
    if len(body) != 1:
        raise ReplyError("malformed", f"expected one value line, got {body!r}")
    line = body[0]
    label, sep, rest = line.rpartition(":")
    if not sep:
        raise ReplyError("malformed", f"no ':' in {line!r}")
    if _normalize_label(label) != label_template.format(c=channel):
        raise ReplyError("malformed", f"label of {line!r} does not belong to {cmd!r}")
    rest = rest.strip()
    if units is None:
        if rest in POLARITIES:
            return rest
        raise ReplyError("malformed", f"unknown polarity in {line!r}")
    m = _NUMBER_RE.fullmatch(rest)
    if m is None:
        raise ReplyError("malformed", f"no number after ':' in {line!r}")
    if m.group(2) not in units:
        raise ReplyError("malformed", f"unit {m.group(2)!r} in {line!r}, expected {units!r}")
    return float(m.group(1))


def query(port, cmd: str, timeout_s: float, clock=time.monotonic) -> float | str:
    send_command(port, cmd)
    return parse_value(cmd, read_reply(port, cmd, timeout_s, clock))


# --- one poll cycle -----------------------------------------------------------------------------


@dataclass
class ChannelReading:
    channel: int
    vmon: float | None = None  # V
    vlimit: float | None = None  # V
    imon: float | None = None  # uA
    ilimit: float | None = None  # uA
    polarity: str | None = None


@dataclass
class CycleReading:
    channels: list[ChannelReading] = field(default_factory=list)
    ramp_vps: float | None = None


def poll_cycle(port, timeout_s: float, warn, clock=time.monotonic) -> CycleReading:
    """Read every channel once. A failed read leaves its field None (never a guessed value).

    Builds a fresh CycleReading every call: nothing is carried over from earlier cycles.
    """
    consecutive_timeouts = 0

    def read(cmd: str):
        nonlocal consecutive_timeouts
        try:
            value = query(port, cmd, timeout_s, clock)
        except ReplyError as e:
            warn(f"reply:{cmd}:{e.kind}", f"{cmd}: {e} -- field omitted this cycle")
            if e.kind == "timeout":
                consecutive_timeouts += 1
                if consecutive_timeouts >= MAX_CONSECUTIVE_TIMEOUTS:
                    raise DeviceSilentError(
                        f"{consecutive_timeouts} commands in a row without a reply"
                    ) from e
            return None
        consecutive_timeouts = 0
        return value

    reading = CycleReading(channels=[ChannelReading(c) for c in CHANNELS])
    for ch in reading.channels:
        for prefix, field_name in CHANNEL_QUERIES:
            setattr(ch, field_name, read(f"{prefix} {ch.channel}"))
    reading.ramp_vps = read(RAMP_QUERY)
    return reading


# --- InfluxDB line protocol ---------------------------------------------------------------------


def escape_tag(value: str) -> str:
    """Line-protocol escaping for tag values: comma, equals sign, space."""
    return value.replace(",", r"\,").replace("=", r"\=").replace(" ", r"\ ")


def escape_field_string(value: str) -> str:
    """Line-protocol escaping inside a double-quoted string field: backslash, double quote."""
    return value.replace("\\", "\\\\").replace('"', '\\"')


def fmt_float(value: float) -> str:
    """Float literal for line protocol: always has a '.', never an exponent, never nan/inf."""
    value = float(value)
    if not math.isfinite(value):
        raise ValueError(f"non-finite value {value!r}")
    text = repr(value)
    if "e" in text or "E" in text:
        text = f"{value:.12f}".rstrip("0")
        if text.endswith("."):
            text += "0"
    return text


def hv_present(vmon: float, threshold_v: float) -> bool:
    """HV output judged from the measured voltage only (TODO 75 §2.2.3). Not the ON/OFF state."""
    return abs(vmon) > threshold_v


def format_lines(device: str, reading: CycleReading, threshold_v: float) -> list[str]:
    """Line-protocol lines for one cycle. Fields whose read failed are omitted."""
    tag_device = escape_tag(device)
    lines = []
    for ch in reading.channels:
        fields = []
        for name in ("vmon", "vlimit", "imon", "ilimit"):
            value = getattr(ch, name)
            if value is not None:
                fields.append(f"{name}={fmt_float(value)}")
        if ch.polarity is not None:
            fields.append(f'polarity="{escape_field_string(ch.polarity)}"')
        if ch.vmon is not None:
            fields.append(f"hv_present={'true' if hv_present(ch.vmon, threshold_v) else 'false'}")
        if fields:
            lines.append(f"mhv4,device={tag_device},channel={ch.channel} {','.join(fields)}")
    if reading.ramp_vps is not None:
        lines.append(f"mhv4_ramp,device={tag_device} speed_vps={fmt_float(reading.ramp_vps)}")
    return lines


def write_influx(url: str, lines: list[str], post, warn) -> bool:
    payload = "\n".join(lines).encode("utf-8")
    try:
        resp = post(
            url,
            data=payload,
            headers={"Content-Type": "text/plain; charset=utf-8"},
            timeout=INFLUX_TIMEOUT_S,
        )
    except requests.RequestException as e:
        warn(f"influx:{type(e).__name__}", f"InfluxDB write failed: {e}")
        return False
    if not resp.ok:
        warn(
            f"influx:http{resp.status_code}",
            f"InfluxDB write failed: HTTP {resp.status_code} {resp.text[:200]}",
        )
        return False
    return True


# --- logging, stop flag ------------------------------------------------------------------------


class WarnLimiter:
    """Log a message the first time its key is seen, then at most once per interval_s.

    The next message after a quiet period reports how many repeats were suppressed.
    """

    def __init__(self, logger, interval_s: float = WARN_REPEAT_INTERVAL_S, clock=time.monotonic):
        self._log = logger
        self._interval = interval_s
        self._clock = clock
        self._state: dict[str, list] = {}  # key -> [last_logged_at, suppressed_count]

    def __call__(self, key: str, msg: str, level=logging.WARNING, exc_info=False) -> bool:
        now = self._clock()
        state = self._state.get(key)
        if state is not None and now - state[0] < self._interval:
            state[1] += 1
            return False
        if state is not None and state[1]:
            msg = f"{msg} (repeated {state[1]}x in the last {now - state[0]:.0f} s)"
        self._log.log(level, msg, exc_info=exc_info)
        self._state[key] = [now, 0]
        return True


class StopFlag:
    """Set from a signal handler. Plain bool, no locks, so a signal can never deadlock it."""

    def __init__(self):
        self._set = False
        self.reason = ""

    def set(self, reason: str = "") -> None:
        self._set = True
        self.reason = reason

    def is_set(self) -> bool:
        return self._set

    def wait(self, timeout_s: float) -> bool:
        end = time.monotonic() + timeout_s
        while not self._set:
            left = end - time.monotonic()
            if left <= 0:
                return False
            time.sleep(min(0.2, left))
        return True


# --- config -------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Config:
    serial_port: str
    device_name: str
    influx_url: str
    poll_interval_sec: float = 2.0
    hv_present_threshold_v: float = 1.0
    reply_timeout_sec: float = 1.0


_REQUIRED_KEYS = ("serial_port", "device_name", "influx_url")
_NUMBER_KEYS = ("poll_interval_sec", "hv_present_threshold_v", "reply_timeout_sec")


def config_from_dict(raw) -> Config:
    """Validate a parsed YAML mapping. Unknown keys are an error (a typo must not fall back
    silently to a default)."""
    if not isinstance(raw, dict):
        raise ValueError("config must be a YAML mapping")
    unknown = sorted(set(raw) - set(_REQUIRED_KEYS) - set(_NUMBER_KEYS))
    if unknown:
        raise ValueError(f"unknown config key(s): {', '.join(unknown)}")
    for key in _REQUIRED_KEYS:
        if not isinstance(raw.get(key), str) or not raw[key].strip():
            raise ValueError(f"config key {key!r} must be a non-empty string")
    if any(ch in raw["device_name"] for ch in '\\"\n\r\t'):
        raise ValueError("device_name must not contain backslash, quote, or control characters")
    numbers = {}
    for key in _NUMBER_KEYS:
        if key not in raw:
            continue
        value = raw[key]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError(f"config key {key!r} must be a number")
        value = float(value)
        if not math.isfinite(value) or value < 0 or (value == 0 and key != "hv_present_threshold_v"):
            raise ValueError(f"config key {key!r} out of range: {value}")
        numbers[key] = value
    return Config(
        serial_port=raw["serial_port"],
        device_name=raw["device_name"],
        influx_url=raw["influx_url"],
        **numbers,
    )


def load_config(path: str) -> Config:
    with open(path) as f:
        return config_from_dict(yaml.safe_load(f))


# --- serial port + main loop --------------------------------------------------------------------


def open_serial_port(path: str):
    # Lazy import: pyserial is installed on the DAQ host; the unit tests never open a port.
    import serial

    return serial.Serial(
        port=path,
        baudrate=9600,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        xonxoff=False,
        rtscts=False,
        dsrdtr=False,
        timeout=READ_SLICE_S,
        write_timeout=WRITE_TIMEOUT_S,
        exclusive=True,
    )


def _close_quietly(port) -> None:
    if port is None:
        return
    try:
        port.close()
    except Exception:  # noqa: BLE001 -- the port is being discarded anyway
        pass


def run(
    cfg: Config,
    stop: StopFlag,
    open_port=open_serial_port,
    post=None,
    clock=time.monotonic,
    retry_delay_s: float = RETRY_DELAY_S,
) -> None:
    """Poll until `stop` is set. Serial trouble -> close, wait retry_delay_s, reopen."""
    if post is None:
        post = requests.post
    warn = WarnLimiter(log, clock=clock)
    port = None
    try:
        while not stop.is_set():
            cycle_start = clock()
            try:
                if port is None:
                    port = open_port(cfg.serial_port)
                    warn("serial:opened", f"Serial port opened: {cfg.serial_port}", logging.INFO)
                reading = poll_cycle(port, cfg.reply_timeout_sec, warn, clock)
            except ForbiddenCommandError:
                raise  # a bug in this file, never retried
            except Exception as e:  # noqa: BLE001 -- SerialException/OSError/termios.error/...
                expected = isinstance(e, (OSError, DeviceSilentError))
                warn(
                    f"serial:{type(e).__name__}:{e}",
                    f"Serial port problem ({type(e).__name__}: {e}) -- "
                    f"closing, retry in {retry_delay_s:.0f} s",
                    logging.ERROR,
                    exc_info=not expected,
                )
                _close_quietly(port)
                port = None
                stop.wait(retry_delay_s)
                continue

            lines = format_lines(cfg.device_name, reading, cfg.hv_present_threshold_v)
            if lines:
                write_influx(cfg.influx_url, lines, post, warn)
            else:
                warn("cycle:empty", "No valid reply in this cycle; nothing written")

            elapsed = clock() - cycle_start
            if elapsed > cfg.poll_interval_sec:
                warn(
                    "cycle:slow",
                    f"Poll cycle took {elapsed:.2f} s > poll_interval_sec "
                    f"{cfg.poll_interval_sec:.2f} s",
                )
            stop.wait(max(0.0, cfg.poll_interval_sec - elapsed))
    finally:
        _close_quietly(port)


def main(argv=None) -> int:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
    parser = argparse.ArgumentParser(description="Mesytec MHV-4 HV monitor (read-only) for InfluxDB")
    parser.add_argument("--config", default="mhv4_config.yaml", help="Path to YAML config file")
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

    log.info(
        "Starting MHV-4 monitor (read-only): port=%s device=%s interval=%.1fs "
        "hv_present_threshold=%.2f V reply_timeout=%.1fs",
        cfg.serial_port,
        cfg.device_name,
        cfg.poll_interval_sec,
        cfg.hv_present_threshold_v,
        cfg.reply_timeout_sec,
    )
    log.info("InfluxDB endpoint: %s", cfg.influx_url)
    log.info("Allowed commands: %s", " | ".join(sorted(ALLOWED_COMMANDS)))
    run(cfg, stop)
    log.info("Stopped (%s)", stop.reason or "loop ended")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
