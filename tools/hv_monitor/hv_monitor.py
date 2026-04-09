#!/usr/bin/env python3
"""CAEN N1408 HV Monitor — reads VMON/IMON via USB serial, writes to InfluxDB."""

import argparse
import logging
import time

import requests
import serial
import yaml

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
log = logging.getLogger("hv_monitor")


def load_config(path: str) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def open_serial(port: str) -> serial.Serial:
    ser = serial.Serial(
        port=port,
        baudrate=9600,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        xonxoff=True,
        timeout=1.0,
    )
    log.info("Serial port opened: %s", port)
    return ser


def read_param(ser: serial.Serial, board: int, param: str) -> list[str] | None:
    """Send MON command for all 4 channels and return list of 4 value strings."""
    ser.reset_input_buffer()
    cmd = f"$BD:{board:02d},CMD:MON,CH:4,PAR:{param}\r\n"
    ser.write(cmd.encode("ascii"))

    response = ser.readline().decode("ascii", errors="replace").strip()

    if "CMD:OK" not in response:
        log.warning("Bad response for %s: %s", param, response)
        return None

    try:
        val_str = response.split("VAL:")[1]
        values = val_str.split(";")
        if len(values) != 4:
            log.warning("Expected 4 values for %s, got %d: %s", param, len(values), val_str)
            return None
        return values
    except (IndexError, ValueError) as e:
        log.warning("Parse error for %s: %s (%s)", param, response, e)
        return None


def poll_and_write(ser: serial.Serial, board: int, influx_url: str):
    """Read all parameters and write to InfluxDB."""
    vmons = read_param(ser, board, "VMON")
    imons = read_param(ser, board, "IMON")
    vsets = read_param(ser, board, "VSET")
    stats = read_param(ser, board, "STAT")

    if not all([vmons, imons, vsets, stats]):
        log.warning("Incomplete readout, skipping this cycle")
        return

    lines = []
    for ch in range(4):
        vmon = float(vmons[ch])
        imon = float(imons[ch])
        vset = float(vsets[ch])
        stat = int(float(stats[ch]))
        line = (
            f"hv_monitor,board={board},channel={ch} "
            f"vmon={vmon},imon={imon},vset={vset},status={stat}i"
        )
        lines.append(line)

    payload = "\n".join(lines)

    try:
        resp = requests.post(influx_url, data=payload, timeout=2.0)
        if not resp.ok:
            log.warning("InfluxDB write failed: %s %s", resp.status_code, resp.text[:200])
    except requests.RequestException as e:
        log.warning("InfluxDB connection error: %s", e)


def main():
    parser = argparse.ArgumentParser(description="CAEN N1408 HV Monitor for InfluxDB/Grafana")
    parser.add_argument("--config", default="config.yaml", help="Path to YAML config file")
    args = parser.parse_args()

    config = load_config(args.config)
    port = config["serial_port"]
    board = config.get("board_id", 0)
    interval = config.get("poll_interval_sec", 2.0)
    influx_url = config["influx_url"]

    log.info("Starting HV monitor: port=%s board=%d interval=%.1fs", port, board, interval)
    log.info("InfluxDB endpoint: %s", influx_url)

    ser = None
    while True:
        try:
            if ser is None:
                ser = open_serial(port)

            poll_and_write(ser, board, influx_url)

        except serial.SerialException as e:
            log.error("Serial error: %s — reconnecting in 5s", e)
            if ser is not None:
                try:
                    ser.close()
                except Exception:
                    pass
                ser = None
            time.sleep(5)
            continue
        except KeyboardInterrupt:
            log.info("Shutting down")
            break

        time.sleep(interval)

    if ser is not None:
        ser.close()


if __name__ == "__main__":
    main()
