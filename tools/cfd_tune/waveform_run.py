#!/usr/bin/env python3
"""Take one run with single-trace waveforms on some DIG1 boards, then restore
their settings exactly (TODO 90 P0/P3 data for cfd_scan).

  python3 -I waveform_run.py --modules 1,2,3 --seconds 60 [--record-ns 1000]
         [--pre-trigger-ns 500] [--patch PATCH.json] [--exp-name DAQ_Test]
         [--comment TEXT] [--operator http://localhost:9091]

- Refuses unless the DAQ is Idle or Configured, no run, no Tune Up.
- Backs up each board's config (GET /api/digitizers/{id}) to a JSON file first.
- Sets waveforms_enabled = True, vtrace_probe_1 = VPROBE_NONE (single trace: the
  input at full rate; dual trace halves it), record_length = --record-ns.
  The record must stay below 1792 samples (3584 ns on a 730, 7168 ns on a 725):
  above that the FW keeps only one of two simultaneous events in a channel couple
  (UM4380 / UM5678).
- --pre-trigger-ns sets channel_defaults.pre_trigger_ns (and drops per-channel
  pre_trigger_ns overrides) for the run: slow ThGEM pulses need a clean baseline
  before the pulse (pf-qaq run 17: 256 ns was not enough).
- --no-waveforms turns waveforms OFF instead (threshold tests).
- --patch applies channel_overrides keys for the run only, in cfd_scan's patch
  format {"<id>": {"channel_overrides": {"<ch>": {"trigger_threshold": 400}}}}
  (e.g. thresholds above the noise, amp_check).
- Starts a run (next run number of the Operator), stops it after --seconds, and
  ALWAYS restores the backups (PUT + save), then checks they read back identical.

Only the Operator REST API is used (CLAUDE.md). Standard library only.
License: BSD-3-Clause (same as delila-rs).
"""
import argparse
import json
import sys
import time
import urllib.request

COUPLE_ARBITER_SAMPLES = 1792


def req(base, method, path, body=None, timeout=120):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(base + path, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=timeout) as f:
        return json.loads(f.read().decode() or "null")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--operator", default="http://localhost:9091")
    ap.add_argument("--modules", required=True, help="comma-separated digitizer ids, e.g. 1,2,3")
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--record-ns", type=int, default=1000)
    ap.add_argument("--pre-trigger-ns", type=int, default=None)
    ap.add_argument("--patch", default=None, help="channel_overrides applied for this run only")
    ap.add_argument("--no-waveforms", action="store_true",
                    help="waveforms OFF for this run (threshold tests: only the patch and the run)")
    ap.add_argument("--exp-name", default=None, help="default: the Operator's experiment_name")
    ap.add_argument("--comment", default="cfd_scan waveforms (single trace)")
    ap.add_argument("--backup", default=None, help="backup file (default /tmp/waveform_run_backup_<time>.json)")
    a = ap.parse_args()
    api = a.operator.rstrip("/") + "/api"
    mods = [int(x) for x in a.modules.split(",") if x.strip()]
    patch = json.load(open(a.patch)) if a.patch else {}
    unknown = [m for m in patch if int(m) not in mods]
    if unknown:
        sys.exit(f"refused: the patch names digitizer(s) {unknown} that are not in --modules")

    st = req(api, "GET", "/status")
    if st.get("run_info") or st.get("tuneup_mode") or st.get("system_state") not in ("Idle", "Configured"):
        sys.exit(f"refused: state {st.get('system_state')}, run {bool(st.get('run_info'))}, "
                 f"Tune Up {st.get('tuneup_mode')} — needs Idle/Configured, no run, no Tune Up")
    backup = {m: req(api, "GET", f"/digitizers/{m}") for m in mods}
    for m, c in backup.items():
        fw = c.get("firmware", "")
        if not fw.endswith("1"):
            sys.exit(f"refused: digitizer {m} firmware {fw} is not DIG1 (PSD1/PHA1)")
        ns = 4.0 if "1725" in (c.get("model") or "") else 2.0
        if not a.no_waveforms and a.record_ns / ns >= COUPLE_ARBITER_SAMPLES:
            sys.exit(f"refused: {a.record_ns} ns = {a.record_ns / ns:.0f} samples on digitizer {m} "
                     f">= {COUPLE_ARBITER_SAMPLES} (couple arbiter: simultaneous events lost)")
    bfile = a.backup or time.strftime("/tmp/waveform_run_backup_%Y%m%d_%H%M%S.json")
    with open(bfile, "w") as f:
        json.dump({str(k): v for k, v in backup.items()}, f, indent=1)
    print(f"backup -> {bfile}")

    run = st["next_run_number"]
    exp = a.exp_name or st.get("experiment_name") or ""
    try:
        for m in mods:
            c = json.loads(json.dumps(backup[m]))
            if a.no_waveforms:
                c["board"]["waveforms_enabled"] = False
            else:
                c["board"]["waveforms_enabled"] = True
                c["board"]["vtrace_probe_1"] = "VPROBE_NONE"
                c["board"]["record_length"] = a.record_ns
            if a.pre_trigger_ns is not None:
                c.setdefault("channel_defaults", {})["pre_trigger_ns"] = a.pre_trigger_ns
                for o in c.get("channel_overrides", {}).values():
                    o.pop("pre_trigger_ns", None)
            for ch, kv in patch.get(str(m), {}).get("channel_overrides", {}).items():
                c.setdefault("channel_overrides", {}).setdefault(str(ch), {}).update(kv)
            req(api, "PUT", f"/digitizers/{m}", c)
            req(api, "POST", f"/digitizers/{m}/save")
        r = req(api, "POST", "/run/start", {"run_number": run, "comment": a.comment, "exp_name": exp})
        print(f"start run {run} ({exp}): {r.get('success')} {r.get('message')}")
        if r.get("success"):
            while True:
                s = req(api, "GET", "/status")
                if not s.get("run_info"):
                    print("run ended by itself (auto-stop?) — check the Operator")
                    break
                if int(s["run_info"].get("elapsed_secs") or 0) >= a.seconds:
                    r = req(api, "POST", "/stop")
                    print(f"stop: {r.get('success')} {r.get('message')}")
                    break
                time.sleep(1)
    finally:
        for m in mods:
            req(api, "PUT", f"/digitizers/{m}", backup[m])
            req(api, "POST", f"/digitizers/{m}/save")
        same = all(req(api, "GET", f"/digitizers/{m}") == backup[m] for m in mods)
        print(f"restored; identical to backup: {same}")
        if not same:
            sys.exit(f"WARNING: a restored config differs from {bfile} — check it")


if __name__ == "__main__":
    main()
