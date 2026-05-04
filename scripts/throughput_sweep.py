#!/usr/bin/env python3
"""Throughput sweep for PSD2 / PHA2 @ 172.18.4.56.

Pre-req: operator+reader+merger+monitor already running with the matching
throughput TOML (e.g. config/config_psd2_thrput.toml or config/config_pha2_thrput.toml).

Each iteration is tagged STUCK / SAT / OK:
  STUCK  delivery < `--stuck-floor-eps` events/s (default 100/s) — looks
         like a wedged FW state, retried up to `--max-retries` times
         after `--stuck-cool-down` seconds idle (default 60 s).
  SAT    delivery < 90 % of target but well above the stuck floor —
         genuine bandwidth saturation (1 Gb/s ethernet ceiling at
         ~108 MB/s for samples=1000), recorded as healthy and NOT retried.
  OK     delivery within 10 % of target.

A short `--cool-down` (default 5 s) is also applied between healthy
iterations to keep the FW happy across rate transitions.

Usage:
    # PSD2 (default JSON path)
    scripts/throughput_sweep.py --mode 1ch  --out throughput_psd2_1ch.csv
    scripts/throughput_sweep.py --mode 32ch --out throughput_psd2_32ch.csv
    # PHA2
    scripts/throughput_sweep.py --json-path config/digitizers/pha2_thrput.json \\
        --mode 1ch --out throughput_pha2_1ch.csv
"""
import argparse
import csv
import json
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

OPERATOR = "http://localhost:9090"
SAMPLES_LIST = [400, 600, 800, 1000]
RATES_BY_MODE = {
    "1ch":  [1000, 2000, 5000, 10_000, 20_000, 30_000, 50_000, 70_000, 100_000],
    "32ch": [100, 200, 500, 1000, 1500, 2000],
}
DEFAULT_ENABLED_BY_MODE = {"1ch": "False", "32ch": "True"}
NCH_BY_MODE = {"1ch": 1, "32ch": 32}


def http_post(path: str, body=None) -> dict:
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
        body = e.read().decode(errors="replace")
        raise RuntimeError(f"POST {path} → {e.code}: {body}") from e


def http_get(path: str) -> dict:
    req = urllib.request.Request(OPERATOR + path)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read().decode())


def wait_state(target: str, timeout_s: float = 30) -> bool:
    deadline = time.time() + timeout_s
    last = None
    while time.time() < deadline:
        try:
            s = http_get("/api/status")
            last = s.get("system_state")
            if last == target:
                return True
        except Exception:
            pass
        time.sleep(0.4)
    print(f"    [warn] wait_state {target} timeout (last={last})", file=sys.stderr)
    return False


def reader_metrics(snap: dict) -> dict:
    for c in snap.get("components", []):
        if c.get("role") == "source":
            m = c.get("metrics") or {}
            return {
                "events": int(m.get("events_processed", 0) or 0),
                "bytes": int(m.get("bytes_transferred", 0) or 0),
                "loss": int(m.get("trigger_loss_count", 0) or 0),
            }
    return {"events": 0, "bytes": 0, "loss": 0}


def edit_json(json_path: Path, samples: int, period_ns: int, default_enabled: str):
    cfg = json.loads(json_path.read_text())
    cfg["board"]["record_length"] = samples
    cfg["board"]["test_pulse_period"] = period_ns
    cfg["channel_defaults"]["enabled"] = default_enabled
    json_path.write_text(json.dumps(cfg, indent=2) + "\n")


def safe_post(path: str, body=None) -> bool:
    try:
        http_post(path, body)
        return True
    except Exception as e:
        print(f"    [warn] POST {path} failed: {e}", file=sys.stderr)
        return False


def measure_one(json_path: Path, samples: int, rate_hz: int, mode: str, duration: float, warmup: float, run_seq: int, stuck_floor_eps: float):
    """Run one Configure→Arm→Start→Stop cycle. Returns measurement dict or
    None on infrastructure failure. The returned `stuck` flag is True only
    when delivery looks catastrophically wedged (events_per_sec below
    `stuck_floor_eps`); genuine FW/network saturation reaches the
    bandwidth ceiling but still delivers many events, and is reported as
    `saturation=True / stuck=False` so retries don't burn cool-down time
    chasing a physical limit."""
    period_ns = int(1_000_000_000 // rate_hz)
    print(f"\n=== mode={mode} samples={samples} rate={rate_hz} Hz period={period_ns} ns")

    edit_json(json_path, samples, period_ns, DEFAULT_ENABLED_BY_MODE[mode])

    safe_post("/api/stop")
    safe_post("/api/reset")
    wait_state("Idle", 10)

    print("  configure")
    if not safe_post("/api/configure", {"run_number": run_seq, "comment": "thrput", "exp_name": "thrput"}):
        return None
    if not wait_state("Configured", 30):
        return None

    print("  arm")
    if not safe_post("/api/arm"):
        safe_post("/api/stop")
        return None
    if not wait_state("Armed", 20):
        safe_post("/api/stop")
        return None

    print("  start")
    if not safe_post("/api/start", {"run_number": run_seq, "comment": "thrput"}):
        safe_post("/api/stop")
        return None
    if not wait_state("Running", 10):
        safe_post("/api/stop")
        return None

    time.sleep(warmup)
    s1 = http_get("/api/status")
    time.sleep(duration)
    s2 = http_get("/api/status")

    safe_post("/api/stop")
    wait_state("Configured", 15)
    safe_post("/api/reset")
    wait_state("Idle", 10)

    m1, m2 = reader_metrics(s1), reader_metrics(s2)
    de = m2["events"] - m1["events"]
    db = m2["bytes"]  - m1["bytes"]
    dl = m2["loss"]   - m1["loss"]
    eps = de / duration
    bps = db / duration
    bpe = (db / de) if de > 0 else 0.0
    nch = NCH_BY_MODE[mode]
    ach = eps / nch
    expected = rate_hz * nch * duration
    pct = (de / expected * 100.0) if expected > 0 else 0.0
    # Stuck = catastrophic delivery failure. Distinguishing from genuine
    # bandwidth saturation: a saturated 1 Gb/s link still delivers tens
    # of thousands of events/s; a stuck FW state delivers ~0.
    stuck = eps < stuck_floor_eps
    # Saturation flag is informational: events flowing but well under
    # target rate (likely network or per-event-bytes ceiling).
    saturated = (not stuck) and pct < 90.0
    if stuck:
        health_tag = "STUCK"
    elif saturated:
        health_tag = "SAT"
    else:
        health_tag = "OK"
    print(f"  result events={de} bytes={db} loss={dl}  -> {eps:.1f} ev/s, {bps/1e6:.2f} MB/s, {bpe:.1f} B/ev, ach/ch={ach:.1f} Hz  [{health_tag} {pct:.0f}% of target]")
    return {
        "samples": samples, "target_rate_hz": rate_hz, "n_channels": nch,
        "duration_s": duration,
        "events_total": de, "bytes_total": db,
        "events_per_sec": round(eps, 1), "bytes_per_sec": round(bps, 1),
        "trigger_loss": dl, "bytes_per_event": round(bpe, 1),
        "achieved_per_ch_hz": round(ach, 1),
        "expected_total": int(expected), "pct_of_target": round(pct, 1),
        "healthy": not stuck,
        "saturated": saturated,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["1ch", "32ch"], required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--json-path", default="config/digitizers/psd2_thrput.json",
                    help="Path to the per-firmware thrput JSON the script edits in-place")
    ap.add_argument("--duration", type=float, default=15.0)
    ap.add_argument("--warmup", type=float, default=3.0)
    ap.add_argument("--cool-down", type=float, default=5.0,
                    help="Seconds to idle between healthy iterations (default 5)")
    ap.add_argument("--stuck-cool-down", type=float, default=60.0,
                    help="Seconds to idle when an iteration looks stuck before retrying (default 60)")
    ap.add_argument("--max-retries", type=int, default=1,
                    help="Per-iteration retries when STUCK is detected (default 1)")
    ap.add_argument("--stuck-floor-eps", type=float, default=100.0,
                    help="Events/s below this counts as a wedged FW state "
                         "and triggers a retry (default 100). Leave well "
                         "below the lowest sweep rate; bandwidth-saturated "
                         "iterations stay above this floor and are recorded "
                         "as SAT, not STUCK.")
    args = ap.parse_args()

    json_path = Path(args.json_path)
    if not json_path.is_file():
        ap.error(f"--json-path {json_path} not found")

    rates = RATES_BY_MODE[args.mode]
    rows = []
    fields = [
        "samples", "target_rate_hz", "n_channels", "duration_s",
        "events_total", "bytes_total",
        "events_per_sec", "bytes_per_sec",
        "trigger_loss", "bytes_per_event", "achieved_per_ch_hz",
        "expected_total", "pct_of_target", "healthy", "saturated", "attempt",
    ]
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    stuck_count = 0
    with open(args.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        f.flush()
        run_seq = int(time.time())
        for samples in SAMPLES_LIST:
            for rate_hz in rates:
                row = None
                attempt = 0
                while attempt <= args.max_retries:
                    run_seq += 1
                    attempt += 1
                    row = measure_one(json_path, samples, rate_hz, args.mode,
                                      args.duration, args.warmup, run_seq,
                                      args.stuck_floor_eps)
                    if row is None:
                        # infrastructure failure (timeout etc.) — bail this iter
                        break
                    if row["healthy"]:
                        break
                    if attempt > args.max_retries:
                        print(f"  [stuck] giving up after {attempt} attempts, "
                              f"recording last result anyway")
                        break
                    print(f"  [stuck] attempt {attempt} unhealthy "
                          f"({row['pct_of_target']}% of target), "
                          f"sleeping {args.stuck_cool_down}s for FW recovery...")
                    stuck_count += 1
                    time.sleep(args.stuck_cool_down)
                if row is None:
                    continue
                row["attempt"] = attempt
                rows.append(row)
                w.writerow({k: row.get(k, "") for k in fields})
                f.flush()
                # Cool-down between iterations (FW seems to need brief idle
                # to fully settle Configure-cycle state).
                if args.cool_down > 0:
                    time.sleep(args.cool_down)
    healthy_n = sum(1 for r in rows if r.get("healthy"))
    saturated_n = sum(1 for r in rows if r.get("saturated"))
    print(f"\nDone. Wrote {len(rows)} rows ({healthy_n} healthy "
          f"[{saturated_n} bandwidth-saturated], "
          f"{len(rows) - healthy_n} stuck after retries, "
          f"{stuck_count} retries triggered) to {args.out}")


if __name__ == "__main__":
    main()
