#!/usr/bin/env python3
"""Stress-test PHA2 Configure→Arm→Start transitions to find the silent-zero-events bug.

For each iteration we:
  1. edit the on-disk JSON to set a random rate
  2. /api/stop, /api/reset, /api/configure, /api/arm, /api/start
  3. wait 2s, snapshot reader metrics
  4. /api/stop, /api/reset
  5. log iteration outcome (PASS / FAIL=events=0 / ANOMALY=events too few)

Usage:
    scripts/pha2_transition_stress.py --iters 50 --out stress.csv
"""
import argparse
import csv
import json
import random
import time
import urllib.error
import urllib.request
from pathlib import Path

OPERATOR = "http://localhost:9090"
JSON_PATH = Path("config/digitizers/pha2_thrput.json")
RATES = [1000, 2000, 5000, 10000, 20000, 50000, 100000]
RUN_TIME_S = 2.0
EXPECTED_FRAC = 0.5  # accept >=50% of expected events (very permissive)


def http_post(path, body=None):
    data = json.dumps(body or {}).encode()
    req = urllib.request.Request(
        OPERATOR + path,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        return {"_error": f"{e.code}", "_body": e.read().decode(errors="replace")}


def http_get(path):
    req = urllib.request.Request(OPERATOR + path)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        return {"_error": f"{e.code}"}


def reader_metrics():
    snap = http_get("/api/status")
    for c in snap.get("components", []):
        if c.get("role") == "source":
            m = c.get("metrics") or {}
            return {
                "events": int(m.get("events_processed", 0) or 0),
                "bytes": int(m.get("bytes_transferred", 0) or 0),
                "loss": int(m.get("trigger_loss_count", 0) or 0),
            }
    return None


def wait_state(target, timeout=15):
    t0 = time.time()
    last = None
    while time.time() - t0 < timeout:
        s = http_get("/api/status")
        last = s.get("system_state")
        if last == target:
            return True
        time.sleep(0.2)
    return False


def edit_json(period_ns, default_enabled="False"):
    cfg = json.loads(JSON_PATH.read_text())
    cfg["board"]["test_pulse_period"] = period_ns
    cfg["board"]["record_length"] = 400
    cfg["channel_defaults"]["enabled"] = default_enabled
    JSON_PATH.write_text(json.dumps(cfg, indent=2) + "\n")


def run_one(iter_idx, rate_hz):
    period = 1_000_000_000 // rate_hz
    edit_json(period)

    http_post("/api/stop")
    http_post("/api/reset")
    wait_state("Idle", 10)

    cfg_resp = http_post("/api/configure", {"run_number": 90000 + iter_idx, "comment": f"stress_{iter_idx}", "exp_name": "stress"})
    if not wait_state("Configured", 30):
        return {"status": "CONFIG_TIMEOUT", "rate": rate_hz, "events": -1, "loss": -1}

    arm_resp = http_post("/api/arm")
    if not wait_state("Armed", 15):
        http_post("/api/stop")
        return {"status": "ARM_TIMEOUT", "rate": rate_hz, "events": -1, "loss": -1}

    start_resp = http_post("/api/start", {"run_number": 90000 + iter_idx, "comment": "stress"})
    if not wait_state("Running", 10):
        http_post("/api/stop")
        return {"status": "START_TIMEOUT", "rate": rate_hz, "events": -1, "loss": -1}

    # Wait long enough to be statistically meaningful even at 1kHz
    time.sleep(RUN_TIME_S)
    m = reader_metrics()
    http_post("/api/stop")
    wait_state("Configured", 10)

    if m is None:
        return {"status": "NO_METRICS", "rate": rate_hz, "events": -1, "loss": -1}

    expected = rate_hz * RUN_TIME_S
    pct = m["events"] / expected if expected > 0 else 0.0
    if m["events"] == 0:
        status = "FAIL_ZERO"
    elif pct < EXPECTED_FRAC:
        status = "FAIL_LOW"
    else:
        status = "PASS"

    return {
        "status": status,
        "rate": rate_hz,
        "events": m["events"],
        "bytes": m["bytes"],
        "loss": m["loss"],
        "expected": int(expected),
        "pct": round(pct * 100, 1),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iters", type=int, default=50)
    ap.add_argument("--out", default="stress.csv")
    ap.add_argument("--seed", type=int, default=None)
    args = ap.parse_args()

    if args.seed is not None:
        random.seed(args.seed)

    rows = []
    fail_count = 0
    fail_zero_rates = []

    fields = ["iter", "rate", "status", "events", "expected", "pct", "bytes", "loss"]
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    with open(args.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        f.flush()

        for i in range(args.iters):
            rate = random.choice(RATES)
            t0 = time.time()
            row = run_one(i, rate)
            row["iter"] = i
            elapsed = time.time() - t0
            print(f"[{i:3d}/{args.iters}] rate={rate:6d}Hz elapsed={elapsed:4.1f}s status={row['status']:10s} events={row.get('events',-1):8d} pct={row.get('pct',0):5.1f}%")
            rows.append(row)
            w.writerow({k: row.get(k, "") for k in fields})
            f.flush()
            if row["status"].startswith("FAIL"):
                fail_count += 1
                if row["status"] == "FAIL_ZERO":
                    fail_zero_rates.append(rate)

    total = len(rows)
    print()
    print(f"=== Summary ===")
    print(f"  total: {total}")
    print(f"  FAIL_ZERO: {sum(1 for r in rows if r['status'] == 'FAIL_ZERO')}")
    print(f"  FAIL_LOW:  {sum(1 for r in rows if r['status'] == 'FAIL_LOW')}")
    print(f"  PASS:      {sum(1 for r in rows if r['status'] == 'PASS')}")
    if fail_zero_rates:
        from collections import Counter
        print(f"  FAIL_ZERO by rate: {dict(Counter(fail_zero_rates))}")


if __name__ == "__main__":
    main()
