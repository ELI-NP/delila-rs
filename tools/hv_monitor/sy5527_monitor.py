#!/usr/bin/env python3
"""SY5527 HV Monitor — reads all slots via CAENHVWrapper, writes to InfluxDB."""

import argparse
import ctypes
import logging
import time

import requests
import yaml

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
log = logging.getLogger("sy5527_monitor")

# CAENHVWrapper constants
CAENHV_OK = 0
SY5527 = 3
LINKTYPE_TCPIP = 0


def load_config(path: str) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


class SY5527Connection:
    """Minimal CAENHVWrapper connection for monitoring (read-only)."""

    def __init__(self, host: str, username: str, password: str,
                 lib_path: str = "/usr/lib64/libcaenhvwrapper.so"):
        self._lib = ctypes.CDLL(lib_path)
        self._handle = ctypes.c_int(-1)
        self._host = host.encode()
        self._username = username.encode()
        self._password = password.encode()
        self._connected = False

    def connect(self):
        if self._connected:
            return
        result = self._lib.CAENHV_InitSystem(
            SY5527, LINKTYPE_TCPIP,
            self._host, self._username, self._password,
            ctypes.byref(self._handle),
        )
        if result != CAENHV_OK:
            raise RuntimeError(f"CAENHV_InitSystem failed: {result}")
        self._connected = True
        log.info("Connected to SY5527 (handle=%d)", self._handle.value)

    def disconnect(self):
        if not self._connected:
            return
        self._lib.CAENHV_DeinitSystem(self._handle.value)
        self._connected = False
        log.info("Disconnected from SY5527")

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, *args):
        self.disconnect()

    def read_slot(self, slot: int, num_channels: int = 12) -> list[dict] | None:
        """Read VMon, VSet, IMon, Status, Pw, and channel names for a slot.

        Returns None if the slot is empty/inaccessible.
        """
        n = num_channels
        h = self._handle.value
        ch_arr = (ctypes.c_ushort * n)(*range(n))

        # Probe: if VMon fails, slot is empty
        vmon_arr = (ctypes.c_float * n)()
        result = self._lib.CAENHV_GetChParam(
            h, ctypes.c_ushort(slot), b"VMon",
            ctypes.c_ushort(n), ch_arr,
            ctypes.cast(vmon_arr, ctypes.c_void_p),
        )
        if result != CAENHV_OK:
            return None

        vset_arr = (ctypes.c_float * n)()
        imon_arr = (ctypes.c_float * n)()
        status_arr = (ctypes.c_uint * n)()
        pw_arr = (ctypes.c_uint * n)()

        self._lib.CAENHV_GetChParam(h, ctypes.c_ushort(slot), b"VSet", ctypes.c_ushort(n), ch_arr, ctypes.cast(vset_arr, ctypes.c_void_p))
        self._lib.CAENHV_GetChParam(h, ctypes.c_ushort(slot), b"IMon", ctypes.c_ushort(n), ch_arr, ctypes.cast(imon_arr, ctypes.c_void_p))
        self._lib.CAENHV_GetChParam(h, ctypes.c_ushort(slot), b"Status", ctypes.c_ushort(n), ch_arr, ctypes.cast(status_arr, ctypes.c_void_p))
        self._lib.CAENHV_GetChParam(h, ctypes.c_ushort(slot), b"Pw", ctypes.c_ushort(n), ch_arr, ctypes.cast(pw_arr, ctypes.c_void_p))

        # Channel names
        NameType = ctypes.c_char * 12
        names_arr = (NameType * n)()
        self._lib.CAENHV_GetChName(h, ctypes.c_ushort(slot), ctypes.c_ushort(n), ch_arr, ctypes.cast(names_arr, ctypes.c_void_p))

        channels = []
        for i in range(n):
            name = names_arr[i].value.decode().strip("\x00").strip()
            channels.append({
                "slot": slot,
                "channel": i,
                "name": name,
                "vmon": vmon_arr[i],
                "vset": vset_arr[i],
                "imon": imon_arr[i],
                "status": status_arr[i],
                "pw": pw_arr[i],
            })
        return channels


def build_line_protocol(all_channels: list[dict]) -> str:
    """Build InfluxDB line protocol string from channel data."""
    lines = []
    for ch in all_channels:
        # Escape spaces in tag values
        name = ch["name"].replace(" ", "\\ ") if ch["name"] else "unnamed"
        line = (
            f'sy5527_hv,slot={ch["slot"]},channel={ch["channel"]},name={name} '
            f'vmon={ch["vmon"]},vset={ch["vset"]},'
            f'imon={ch["imon"]},status={ch["status"]}i,pw={ch["pw"]}i'
        )
        lines.append(line)
    return "\n".join(lines)


def poll_and_write(conn: SY5527Connection, slots: list[int],
                   channels_per_slot: int, influx_url: str):
    """Read all configured slots and write to InfluxDB."""
    all_channels = []
    for slot in slots:
        data = conn.read_slot(slot, channels_per_slot)
        if data is not None:
            all_channels.extend(data)

    if not all_channels:
        log.warning("No data read from any slot")
        return

    payload = build_line_protocol(all_channels)

    try:
        resp = requests.post(influx_url, data=payload, timeout=5.0)
        if not resp.ok:
            log.warning("InfluxDB write failed: %s %s", resp.status_code, resp.text[:200])
    except requests.RequestException as e:
        log.warning("InfluxDB connection error: %s", e)


def main():
    parser = argparse.ArgumentParser(
        description="SY5527 HV Monitor for InfluxDB/Grafana")
    parser.add_argument("--config", default="sy5527_config.yaml",
                        help="Path to YAML config file")
    args = parser.parse_args()

    config = load_config(args.config)
    host = config["hv_host"]
    username = config.get("hv_username", "admin")
    password = config.get("hv_password", "")
    lib_path = config.get("lib_path", "/usr/lib64/libcaenhvwrapper.so")
    slots = config.get("slots", [0, 1, 4, 5, 7, 8, 10, 11, 13, 14])
    channels_per_slot = config.get("channels_per_slot", 12)
    interval = config.get("poll_interval_sec", 5.0)
    influx_url = config["influx_url"]

    log.info("Starting SY5527 monitor: host=%s slots=%s interval=%.1fs",
             host, slots, interval)
    log.info("InfluxDB endpoint: %s", influx_url)

    conn = None
    while True:
        try:
            if conn is None:
                conn = SY5527Connection(host, username, password, lib_path)
                conn.connect()

            poll_and_write(conn, slots, channels_per_slot, influx_url)

        except RuntimeError as e:
            log.error("Connection error: %s — reconnecting in 10s", e)
            if conn is not None:
                try:
                    conn.disconnect()
                except Exception:
                    pass
                conn = None
            time.sleep(10)
            continue
        except KeyboardInterrupt:
            log.info("Shutting down")
            break

        time.sleep(interval)

    if conn is not None:
        conn.disconnect()


if __name__ == "__main__":
    main()
