#!/usr/bin/env python3
"""CAEN HV mainframe monitor (SY5527 / SY4527 / ...) — read-only, via CAENHVWrapper,
writes to InfluxDB.

Only Get calls are made (InitSystem, GetCrateMap, GetChParam, GetChName, GetError,
Free, DeinitSystem); nothing is ever set on the crate.

Per poll, for every populated slot (from the crate map, optionally limited by the
config's `slots`), one line per channel:

    sy5527_hv,slot=<s>,channel=<c>,name=<crate channel name> \
        vmon=<V>,vset=<V>,imon=<uA>,status=<bits>i,pw=<0|1>i

`vset` is the channel parameter V0Set. (Until 2026-10-22 this script asked for
"VSet", which the boards do not have: the call failed with -5 "Parameter not
present", the result was not checked, and vset was written as 0.) A field whose
read fails is left out of the line, never written as a guess.

Constants are from CAENHVWrapper.h v7.5 (CAEN_SYSTEM_TYPE_t, LINKTYPE_TCPIP,
MAX_CH_NAME). The library is CAEN's and is not in this repository.
"""

from __future__ import annotations

import argparse
import ctypes
import logging
import signal
import time

import requests
import yaml

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
log = logging.getLogger("sy5527_monitor")

CAENHV_OK = 0
LINKTYPE_TCPIP = 0
MAX_CH_NAME = 12
SYSTEM_TYPES = {"SY1527": 0, "SY2527": 1, "SY4527": 2, "SY5527": 3, "R6060": 16}

# (InfluxDB field, CAEN channel parameter, C type of one value)
CHANNEL_PARAMS = (
    ("vmon", b"VMon", ctypes.c_float),
    ("vset", b"V0Set", ctypes.c_float),
    ("imon", b"IMon", ctypes.c_float),
    ("status", b"Status", ctypes.c_uint),
    ("pw", b"Pw", ctypes.c_uint),
)
INT_FIELDS = {"status", "pw"}

RECONNECT_DELAY_S = 10.0
WARN_REPEAT_INTERVAL_S = 300.0


class ConnectionLost(RuntimeError):
    """The crate cannot be reached or no read succeeds: reconnect."""


class WarnLimiter:
    """Log a warning the first time its key is seen, then at most once per interval."""

    def __init__(self, interval_s: float = WARN_REPEAT_INTERVAL_S, clock=time.monotonic):
        self._interval = interval_s
        self._clock = clock
        self._last: dict[str, float] = {}
        self._suppressed: dict[str, int] = {}

    def __call__(self, key: str, msg: str) -> bool:
        now = self._clock()
        last = self._last.get(key)
        if last is not None and now - last < self._interval:
            self._suppressed[key] = self._suppressed.get(key, 0) + 1
            return False
        n = self._suppressed.pop(key, 0)
        log.warning("%s%s", msg, f" (repeated {n} times since last report)" if n else "")
        self._last[key] = now
        return True


def load_config(path: str) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def escape_tag(value: str) -> str:
    """Line-protocol tag value: backslash-escape comma, equals sign and space."""
    return value.replace(",", "\\,").replace("=", "\\=").replace(" ", "\\ ")


def format_field(name: str, value) -> str:
    if name in INT_FIELDS:
        return f"{name}={int(value)}i"
    return f"{name}={round(float(value), 4)}"


def build_lines(slot: int, names: list[str] | None, values: dict[str, list]) -> list[str]:
    """One sy5527_hv line per channel. `values` holds only the fields that were read."""
    n = max((len(v) for v in values.values()), default=0)
    lines = []
    for ch in range(n):
        fields = [format_field(f, values[f][ch]) for f, _, _ in CHANNEL_PARAMS if f in values]
        if not fields:
            continue
        name = (names[ch] if names and ch < len(names) else "").strip() or "unnamed"
        lines.append(f"sy5527_hv,slot={slot},channel={ch},name={escape_tag(name)} " + ",".join(fields))
    return lines


def select_slots(populated: list[tuple[int, int]], configured: list[int] | None,
                 warn=None) -> list[tuple[int, int]]:
    """Populated (slot, nch) pairs, limited to `configured` slots when given."""
    if configured is None:
        return populated
    by_slot = dict(populated)
    for s in configured:
        if s not in by_slot and warn:
            warn(f"empty-slot-{s}", f"Configured slot {s} is empty in the crate map; skipped")
    return [(s, by_slot[s]) for s in configured if s in by_slot]


class HVConnection:
    """Read-only CAENHVWrapper session (ctypes)."""

    def __init__(self, lib_path: str, system_type: str, host: str, username: str, password: str):
        if system_type not in SYSTEM_TYPES:
            raise ValueError(f"unknown system_type {system_type!r}; known: {sorted(SYSTEM_TYPES)}")
        self._lib = lib = ctypes.CDLL(lib_path)
        P = ctypes.POINTER
        lib.CAENHV_InitSystem.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_char_p,
                                          ctypes.c_char_p, ctypes.c_char_p, P(ctypes.c_int)]
        lib.CAENHV_DeinitSystem.argtypes = [ctypes.c_int]
        lib.CAENHV_GetError.argtypes = [ctypes.c_int]
        lib.CAENHV_GetError.restype = ctypes.c_char_p
        lib.CAENHV_GetCrateMap.argtypes = [
            ctypes.c_int, P(ctypes.c_ushort), P(P(ctypes.c_ushort)), P(P(ctypes.c_char)),
            P(P(ctypes.c_char)), P(P(ctypes.c_uint)), P(P(ctypes.c_ubyte)), P(P(ctypes.c_ubyte))]
        lib.CAENHV_GetChParam.argtypes = [ctypes.c_int, ctypes.c_ushort, ctypes.c_char_p,
                                          ctypes.c_ushort, P(ctypes.c_ushort), ctypes.c_void_p]
        lib.CAENHV_GetChName.argtypes = [ctypes.c_int, ctypes.c_ushort, ctypes.c_ushort,
                                         P(ctypes.c_ushort), ctypes.c_void_p]
        lib.CAENHV_Free.argtypes = [ctypes.c_void_p]
        self._system = SYSTEM_TYPES[system_type]
        self._host = host.encode()
        self._username = username.encode()
        self._password = password.encode()
        self._handle = None

    def _error(self) -> str:
        if self._handle is None:
            return ""
        msg = self._lib.CAENHV_GetError(self._handle)
        return msg.decode(errors="replace") if msg else ""

    def connect(self) -> None:
        handle = ctypes.c_int(-1)
        rc = self._lib.CAENHV_InitSystem(self._system, LINKTYPE_TCPIP, self._host,
                                         self._username, self._password, ctypes.byref(handle))
        if rc != CAENHV_OK:
            raise ConnectionLost(f"CAENHV_InitSystem failed: {rc}")
        self._handle = handle.value
        log.info("Connected to %s (handle=%d)", self._host.decode(), self._handle)

    def disconnect(self) -> None:
        if self._handle is None:
            return
        rc = self._lib.CAENHV_DeinitSystem(self._handle)
        log.info("Disconnected (DeinitSystem rc=%d)", rc)
        self._handle = None

    def crate_map(self) -> list[tuple[int, int, str, str]]:
        """(slot, number of channels, model, description) for every populated slot."""
        P = ctypes.POINTER
        nslot = ctypes.c_ushort()
        nch, model, desc = P(ctypes.c_ushort)(), P(ctypes.c_char)(), P(ctypes.c_char)()
        ser, fmin, fmax = P(ctypes.c_uint)(), P(ctypes.c_ubyte)(), P(ctypes.c_ubyte)()
        rc = self._lib.CAENHV_GetCrateMap(self._handle, ctypes.byref(nslot), ctypes.byref(nch),
                                          ctypes.byref(model), ctypes.byref(desc), ctypes.byref(ser),
                                          ctypes.byref(fmin), ctypes.byref(fmax))
        if rc != CAENHV_OK:
            raise ConnectionLost(f"CAENHV_GetCrateMap failed: {rc} {self._error()}")
        try:
            models = self._packed_strings(model, nslot.value)
            descs = self._packed_strings(desc, nslot.value)
            return [(s, nch[s], models[s], descs[s].strip())
                    for s in range(nslot.value) if nch[s]]
        finally:
            for p in (nch, model, desc, ser, fmin, fmax):
                self._lib.CAENHV_Free(p)

    @staticmethod
    def _packed_strings(ptr, n: int) -> list[str]:
        """n NUL-terminated strings stored back to back (GetCrateMap's model/description lists)."""
        out, offset = [], 0
        base = ctypes.addressof(ptr.contents)
        for _ in range(n):
            s = ctypes.string_at(base + offset)
            out.append(s.decode(errors="replace"))
            offset += len(s) + 1
        return out

    def read_slot(self, slot: int, nch: int, warn) -> tuple[list[str] | None, dict[str, list]]:
        """Channel names and the fields that were read; failed reads are warned about and left out."""
        chans = (ctypes.c_ushort * nch)(*range(nch))
        values: dict[str, list] = {}
        for field, param, ctype in CHANNEL_PARAMS:
            arr = (ctype * nch)()
            rc = self._lib.CAENHV_GetChParam(self._handle, slot, param, nch, chans,
                                             ctypes.cast(arr, ctypes.c_void_p))
            if rc == CAENHV_OK:
                values[field] = list(arr)
            else:
                warn(f"param-{slot}-{field}",
                     f"slot {slot}: GetChParam {param.decode()} failed: {rc} {self._error()}")
        names_arr = ((ctypes.c_char * MAX_CH_NAME) * nch)()
        rc = self._lib.CAENHV_GetChName(self._handle, slot, nch, chans,
                                        ctypes.cast(names_arr, ctypes.c_void_p))
        names = None
        if rc == CAENHV_OK:
            names = [names_arr[i].value.decode(errors="replace") for i in range(nch)]
        else:
            warn(f"names-{slot}", f"slot {slot}: GetChName failed: {rc} {self._error()}")
        return names, values


def poll_once(conn, slots: list[tuple[int, int]], warn) -> list[str]:
    """Read every slot; raise ConnectionLost when nothing at all could be read."""
    lines = []
    for slot, nch in slots:
        names, values = conn.read_slot(slot, nch, warn)
        lines.extend(build_lines(slot, names, values))
    if slots and not lines:
        raise ConnectionLost("no channel parameter could be read from any slot")
    return lines


def write_influx(url: str, lines: list[str], warn) -> None:
    try:
        resp = requests.post(url, data="\n".join(lines), timeout=5.0)
        if not resp.ok:
            warn("influx-http", f"InfluxDB write failed: {resp.status_code} {resp.text[:200]}")
    except requests.RequestException as e:
        warn("influx-conn", f"InfluxDB connection error: {e}")


def main() -> None:
    parser = argparse.ArgumentParser(description="CAEN HV mainframe monitor for InfluxDB/Grafana (read-only)")
    parser.add_argument("--config", default="sy5527_config.yaml", help="Path to YAML config file")
    args = parser.parse_args()

    config = load_config(args.config)
    host = config["hv_host"]
    interval = float(config.get("poll_interval_sec", 5.0))
    influx_url = config["influx_url"]
    configured_slots = config.get("slots")  # None = every populated slot
    conn = HVConnection(
        lib_path=config.get("lib_path", "/usr/local/lib/libcaenhvwrapper.so"),
        system_type=config.get("system_type", "SY5527"),
        host=host,
        username=config.get("hv_username", "admin"),
        password=config.get("hv_password", ""),
    )
    warn = WarnLimiter()

    stopping = False

    def on_signal(signum, _frame):
        nonlocal stopping
        stopping = True
        log.info("Received signal %d, stopping", signum)

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)

    log.info("Starting HV monitor (read-only): host=%s system=%s interval=%.1fs slots=%s",
             host, config.get("system_type", "SY5527"), interval,
             configured_slots if configured_slots is not None else "all populated")
    log.info("InfluxDB endpoint: %s", influx_url)

    slots: list[tuple[int, int]] | None = None
    while not stopping:
        try:
            if slots is None:
                conn.connect()
                crate = conn.crate_map()
                for s, n, model, desc in crate:
                    log.info("slot %d: %s, %d ch, %s", s, model, n, desc)
                slots = select_slots([(s, n) for s, n, _, _ in crate], configured_slots, warn)
                if not slots:
                    raise ConnectionLost("no populated slot to read")
            write_influx(influx_url, poll_once(conn, slots, warn), warn)
        except ConnectionLost as e:
            warn("connection", f"{e} — reconnecting in {RECONNECT_DELAY_S:.0f} s")
            conn.disconnect()
            slots = None
            deadline = time.monotonic() + RECONNECT_DELAY_S
            while not stopping and time.monotonic() < deadline:
                time.sleep(0.2)
            continue
        deadline = time.monotonic() + interval
        while not stopping and time.monotonic() < deadline:
            time.sleep(0.2)

    conn.disconnect()
    log.info("Stopped")


if __name__ == "__main__":
    main()
