#!/usr/bin/env python3
"""PMT Gain Matcher — Automated HV adjustment for photopeak alignment.

Usage:
    python3 gain_matcher.py status --config gain_config.yaml
    python3 gain_matcher.py scan --measure-time 10
    python3 gain_matcher.py match --config gain_config.yaml [--dry-run] [--yes]

SAFETY: Use --dry-run to simulate without changing HV.
"""

import argparse
import datetime
import logging
import sys
import time

import numpy as np
import yaml

from config import GainMatcherConfig, load_config
from daq_client import DAQClient, DAQError
from fitter import FitResult, find_peaks_auto, fit_peak
from hv_control import HVController, CAENHVError

logger = logging.getLogger(__name__)


def cmd_status(args):
    """Show HV crate status — all slots and channels."""
    cfg = load_config(args.config)

    print(f"Connecting to SY5527 at {cfg.hv_host}...")
    with HVController(cfg.hv_host, cfg.hv_username, cfg.hv_password) as hv:
        # Probe each slot (0-15) to find populated ones
        # GetCrateMap has ctypes compatibility issues, so we probe directly
        print("\nProbing slots...")
        print("-" * 80)

        for slot in range(16):
            # Try reading VMon for ch0 to detect if slot is populated
            try:
                test = hv._get_float_param(slot, "VMon", [0])
            except CAENHVError:
                continue  # empty slot

            # Detect number of channels by probing
            n_ch = _probe_slot_channels(hv, slot)
            if n_ch == 0:
                continue

            print(f"\nSlot {slot}: {n_ch} channels")
            print(f"{'Ch':>4} {'Name':>12} {'VSet':>8} {'VMon':>8} "
                  f"{'IMon':>8} {'SVMax':>8} {'Pw':>4} {'Status':>8}")
            print("-" * 68)

            channels = list(range(n_ch))
            params = hv.get_channel_params(slot, channels)
            for p in params:
                pw_str = "ON" if p.pw else "OFF"
                print(f"{p.channel:4d} {p.name:>12s} {p.v_set:8.1f} "
                      f"{p.v_mon:8.1f} {p.i_mon:8.2f} {p.sv_max:8.0f} "
                      f"{pw_str:>4s} 0x{p.status:04X}")


def _probe_slot_channels(hv, slot: int) -> int:
    """Probe how many channels a slot has by trying common sizes."""
    for n in [48, 24, 12, 8, 4]:
        try:
            hv._get_float_param(slot, "VMon", list(range(n)))
            return n
        except CAENHVError:
            continue
    return 0


def cmd_scan(args):
    """Scan all channels: acquire histograms and auto-detect peaks."""
    cfg = load_config(args.config) if hasattr(args, 'config') and args.config else None
    daq = DAQClient(
        operator_url=cfg.operator_url if cfg else args.operator_url,
        monitor_url=cfg.monitor_url if cfg else args.monitor_url,
    )

    # Check DAQ is running
    try:
        if not daq.is_running():
            print("WARNING: DAQ is not in Running state.")
            if not _confirm("Continue anyway?"):
                return
    except DAQError as e:
        print(f"ERROR: Cannot reach DAQ: {e}")
        return

    # Clear and accumulate
    print("Clearing histograms...")
    daq.clear_histograms()
    print(f"Accumulating data for {args.measure_time}s...")
    time.sleep(args.measure_time)

    # Get all histograms
    print("Fetching histograms...")
    summary = daq.get_all_histograms()
    ch_list = summary.get("channels", [])

    print(f"\n{'Module':>6} | {'Ch':>4} | {'Counts':>10} | "
          f"{'Peak ADC':>10} | Suggested Region")
    print("-" * 65)

    results = []
    for ch_info in ch_list:
        m = ch_info["module_id"]
        c = ch_info["channel_id"]
        total = ch_info.get("total_counts", 0)

        if total < 100:
            print(f"{m:6d} | {c:4d} | {total:10d} | {'none':>10s} | (skip)")
            results.append({"module": m, "channel": c, "counts": total,
                            "peak": None, "region": None})
            continue

        hist = daq.get_histogram(m, c)
        peaks = find_peaks_auto(hist["bins"])

        if not peaks:
            print(f"{m:6d} | {c:4d} | {total:10d} | {'none':>10s} | (no peak)")
            results.append({"module": m, "channel": c, "counts": total,
                            "peak": None, "region": None})
            continue

        peak = peaks[0]  # strongest peak
        margin = 50
        region = [max(0, peak - margin), min(65535, peak + margin)]
        print(f"{m:6d} | {c:4d} | {total:10d} | {peak:10d} | {region}")
        results.append({"module": m, "channel": c, "counts": total,
                        "peak": int(peak), "region": region})

    # Save results
    ts = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
    out_path = args.output or f"scan_result_{ts}.yaml"
    with open(out_path, "w") as f:
        yaml.dump({"timestamp": ts, "measure_time": args.measure_time,
                    "results": results}, f, default_flow_style=False)
    print(f"\nSaved to: {out_path}")


def cmd_match(args):
    """Run gain matching iteration loop."""
    cfg = load_config(args.config)
    if args.max_iterations is not None:
        cfg.max_iterations = args.max_iterations

    daq = DAQClient(cfg.operator_url, cfg.monitor_url)
    active = [ch for ch in cfg.channels if not ch.skip]
    print(f"Gain matching: {len(active)} active channels, "
          f"max {cfg.max_iterations} iterations")

    # Check DAQ
    try:
        if not daq.is_running():
            print("WARNING: DAQ is not in Running state.")
            if not args.yes and not _confirm("Continue anyway?"):
                return
    except DAQError as e:
        print(f"ERROR: Cannot reach DAQ: {e}")
        return

    # Connect HV
    print(f"Connecting to SY5527 at {cfg.hv_host}...")
    with HVController(cfg.hv_host, cfg.hv_username, cfg.hv_password) as hv:
        # Safety confirmation
        if not args.dry_run and not args.yes:
            print("\n*** WARNING: This will CHANGE HV settings! ***")
            if not _confirm("Proceed with HV adjustment?"):
                return

        final_iteration = 0
        all_results = {}

        for iteration in range(1, cfg.max_iterations + 1):
            final_iteration = iteration
            print(f"\n{'='*60}")
            print(f"=== Iteration {iteration}/{cfg.max_iterations} ===")
            print(f"{'='*60}")

            # 1. Clear + accumulate
            daq.clear_histograms()
            print(f"Accumulating data for {cfg.measure_time}s...")
            time.sleep(cfg.measure_time)

            # 2. Fit each channel
            print(f"\n{'Ch':>12s} | {'Peak':>8s} | {'Target':>8s} | "
                  f"{'Delta':>8s} | {'V_old':>8s} | {'V_new':>8s} | Status")
            print("-" * 78)

            adjustments = []
            all_converged = True
            fit_failures = 0

            for ch in active:
                try:
                    hist = daq.get_histogram(ch.dig_module, ch.dig_ch)
                except DAQError as e:
                    print(f"{ch.name:>12s} | {'---':>8s} | {ch.target_position:8d} | "
                          f"{'---':>8s} | {'---':>8s} | {'---':>8s} | DAQ ERROR")
                    fit_failures += 1
                    all_converged = False  # Fix #1: failed channels are not converged
                    continue

                fit = fit_peak(hist["bins"], ch.peak_region, cfg.min_counts)
                all_results[ch.name] = fit

                if not fit.success:
                    print(f"{ch.name:>12s} | {'---':>8s} | {ch.target_position:8d} | "
                          f"{'---':>8s} | {'---':>8s} | {'---':>8s} | "
                          f"FIT FAIL: {fit.message}")
                    fit_failures += 1
                    all_converged = False  # Fix #1: failed channels are not converged
                    continue

                # Fix #3: guard against zero/negative peak position
                if fit.peak_position <= 1.0:
                    print(f"{ch.name:>12s} | {fit.peak_position:8.1f} | "
                          f"{ch.target_position:8d} | {'---':>8s} | "
                          f"{'---':>8s} | {'---':>8s} | PEAK TOO LOW")
                    fit_failures += 1
                    all_converged = False
                    continue

                delta = fit.peak_position - ch.target_position
                tolerance = ch.target_position * cfg.tolerance_percent / 100.0

                if abs(delta) <= tolerance:
                    print(f"{ch.name:>12s} | {fit.peak_position:8.1f} | "
                          f"{ch.target_position:8d} | {delta:+8.1f} | "
                          f"{'---':>8s} | {'---':>8s} | CONVERGED")
                    continue

                all_converged = False

                # Get current HV
                hv_info = hv.get_channel_params(ch.hv_slot, [ch.hv_ch])[0]
                v_current = hv_info.v_set

                if v_current < 1.0:
                    print(f"{ch.name:>12s} | {fit.peak_position:8.1f} | "
                          f"{ch.target_position:8d} | {delta:+8.1f} | "
                          f"{v_current:8.1f} | {'---':>8s} | HV OFF/ZERO")
                    continue

                # V_new = V_current * (target / current)^(1/alpha)
                ratio = ch.target_position / fit.peak_position
                v_new = v_current * (ratio ** (1.0 / cfg.pmt_alpha))

                # Step limit
                dv = v_new - v_current
                if abs(dv) > cfg.hv_step_limit:
                    v_new = v_current + cfg.hv_step_limit * (1 if dv > 0 else -1)

                # Safety: non-negative
                v_new = max(0.0, v_new)

                status = "DRY-RUN" if args.dry_run else "ADJUSTING"
                print(f"{ch.name:>12s} | {fit.peak_position:8.1f} | "
                      f"{ch.target_position:8d} | {delta:+8.1f} | "
                      f"{v_current:8.1f} | {v_new:8.1f} | {status}")

                adjustments.append((ch, v_current, v_new))

            # Check termination
            if fit_failures == len(active):
                print("\nERROR: All channels failed to fit. Aborting.")
                break

            if all_converged:
                print("\n*** All channels converged! ***")
                break

            # 3. Apply HV changes
            if not args.dry_run and adjustments:
                for ch, v_old, v_new in adjustments:
                    hv.set_voltage(ch.hv_slot, ch.hv_ch, v_new)

                print(f"\nWaiting for HV ramp ({len(adjustments)} channels)...")
                slots = set(ch.hv_slot for ch, _, _ in adjustments)
                for slot in slots:
                    chs = [ch.hv_ch for ch, _, _ in adjustments
                           if ch.hv_slot == slot]
                    if not hv.wait_ramp(slot, chs):
                        print(f"WARNING: Ramp timeout on slot {slot}")

                # Fix #4: PMT settling time after voltage change
                if cfg.settling_time > 0:
                    print(f"Waiting {cfg.settling_time}s for PMT settling...")
                    time.sleep(cfg.settling_time)

        # Save final results
        _save_results(cfg, all_results, active, final_iteration,
                      all_converged, args.dry_run)


def _save_results(cfg, results, active, iterations, converged, dry_run):
    """Save matching results to YAML."""
    ts = datetime.datetime.now().strftime("%Y-%m-%dT%H:%M:%S")
    out = {
        "timestamp": ts,
        "iterations": iterations,
        "converged": converged,
        "dry_run": dry_run,
        "channels": [],
    }
    for ch in active:
        fit = results.get(ch.name)
        entry = {
            "name": ch.name,
            "target": ch.target_position,
        }
        if fit and fit.success:
            entry["final_peak"] = round(fit.peak_position, 1)
            delta_pct = (fit.peak_position - ch.target_position) / ch.target_position * 100
            entry["delta_percent"] = round(delta_pct, 2)
        else:
            entry["final_peak"] = None
            entry["delta_percent"] = None
            entry["error"] = fit.message if fit else "no data"
        out["channels"].append(entry)

    out_path = f"result_{datetime.datetime.now().strftime('%Y%m%d_%H%M%S')}.yaml"
    with open(out_path, "w") as f:
        yaml.dump(out, f, default_flow_style=False)
    print(f"\nResults saved to: {out_path}")


def _confirm(prompt: str) -> bool:
    """Ask user for Y/n confirmation."""
    try:
        answer = input(f"{prompt} [y/N] ").strip().lower()
        return answer in ("y", "yes")
    except (EOFError, KeyboardInterrupt):
        return False


def main():
    parser = argparse.ArgumentParser(
        description="PMT Gain Matcher — Automated HV gain matching tool"
    )
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="Enable debug logging")
    subparsers = parser.add_subparsers(dest="command")

    # status
    sp_status = subparsers.add_parser("status", help="Show HV crate status")
    sp_status.add_argument("--config", required=True, help="YAML config file")

    # scan
    sp_scan = subparsers.add_parser("scan", help="Scan channels for peaks")
    sp_scan.add_argument("--config", default=None,
                         help="YAML config file (uses daq URLs from config)")
    sp_scan.add_argument("--measure-time", type=int, default=10,
                         help="Data accumulation time (seconds)")
    sp_scan.add_argument("--output", type=str, default=None,
                         help="Output YAML file path")
    sp_scan.add_argument("--operator-url", default="http://localhost:8080",
                         help="Operator URL (overridden by --config)")
    sp_scan.add_argument("--monitor-url", default="http://localhost:8081",
                         help="Monitor URL (overridden by --config)")

    # match
    sp_match = subparsers.add_parser("match", help="Run gain matching")
    sp_match.add_argument("--config", required=True, help="YAML config file")
    sp_match.add_argument("--max-iterations", type=int, default=None,
                          help="Override max iterations from config")
    sp_match.add_argument("--dry-run", action="store_true",
                          help="Simulate without changing HV")
    sp_match.add_argument("--yes", "-y", action="store_true",
                          help="Skip confirmation prompts")

    args = parser.parse_args()

    # Logging
    level = logging.DEBUG if args.verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s %(levelname)-5s %(name)s: %(message)s",
        datefmt="%H:%M:%S",
    )

    if args.command == "status":
        cmd_status(args)
    elif args.command == "scan":
        cmd_scan(args)
    elif args.command == "match":
        cmd_match(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
