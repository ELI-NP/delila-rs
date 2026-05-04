#!/usr/bin/env python3
"""Plot PSD2 throughput sweep results.

Inputs (CSV columns produced by throughput_sweep.sh):
    samples, target_rate_hz, n_channels, duration_s,
    events_total, bytes_total, events_per_sec, bytes_per_sec,
    trigger_loss, bytes_per_event, achieved_per_ch_hz

Usage:
    plot_throughput.py results/throughput_1ch.csv results/throughput_32ch.csv
        -o results/throughput_plots.png
"""
import argparse
import sys
from pathlib import Path
import csv
from collections import defaultdict

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


def load(path: Path):
    rows = []
    with path.open() as f:
        for r in csv.DictReader(f):
            rows.append(
                dict(
                    samples=int(r["samples"]),
                    target_rate_hz=float(r["target_rate_hz"]),
                    n_channels=int(r["n_channels"]),
                    events_total=int(r["events_total"]),
                    events_per_sec=float(r["events_per_sec"]),
                    bytes_per_sec=float(r["bytes_per_sec"]),
                    bytes_per_event=float(r["bytes_per_event"]),
                    trigger_loss=int(r["trigger_loss"]),
                    achieved_per_ch_hz=float(r["achieved_per_ch_hz"]),
                )
            )
    return rows


def by_samples(rows):
    g = defaultdict(list)
    for r in rows:
        g[r["samples"]].append(r)
    for k in g:
        g[k].sort(key=lambda r: r["target_rate_hz"])
    return g


SAMPLE_COLORS = {400: "tab:blue", 600: "tab:orange", 800: "tab:green", 1000: "tab:red"}


def predicted_bytes_per_event(samples: int) -> int:
    # PSD2 64-bit words: 2 header + 2 wave-header + N/2 wave-data
    return 8 * (4 + samples // 2)


def filter_outliers(rows):
    """Drop points where bytes/event deviates >40% from prediction
    (caused by leftover buffered data leaking into the warmup window)."""
    out = []
    for r in rows:
        pred = predicted_bytes_per_event(r["samples"])
        if r["bytes_per_event"] <= 0 or r["events_total"] <= 0:
            continue
        ratio = r["bytes_per_event"] / pred
        if ratio < 0.6 or ratio > 1.4:
            # huge mismatch — probably warmup pollution from previous run
            continue
        out.append(r)
    return out


def plot_panel(ax, rows, title, network_cap_bytes_per_s):
    g = by_samples(rows)
    n_channels = rows[0]["n_channels"] if rows else 1
    for s in sorted(g.keys()):
        xs = [r["target_rate_hz"] for r in g[s]]
        ys = [r["achieved_per_ch_hz"] for r in g[s]]
        color = SAMPLE_COLORS.get(s, None)
        ax.plot(xs, ys, "o-", color=color, label=f"{s} samples")
    # ideal line: achieved == target
    if rows:
        xs = sorted({r["target_rate_hz"] for r in rows})
        ax.plot(xs, xs, "k--", lw=1, alpha=0.5, label="ideal (achieved = target)")
    # network-saturation prediction lines: per-ch rate at which 1Gb saturates
    for s, color in SAMPLE_COLORS.items():
        bpe = predicted_bytes_per_event(s)
        sat_total = network_cap_bytes_per_s / bpe
        sat_per_ch = sat_total / n_channels
        ax.axhline(sat_per_ch, color=color, lw=1, ls=":", alpha=0.6)
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("Target TestPulse rate per ch [Hz]")
    ax.set_ylabel("Achieved rate per ch [Hz]")
    ax.set_title(title)
    ax.grid(True, which="both", alpha=0.3)
    ax.legend(fontsize=8)


def plot_bandwidth(ax, rows, title, network_cap_bytes_per_s):
    g = by_samples(rows)
    for s in sorted(g.keys()):
        xs = [r["target_rate_hz"] for r in g[s]]
        ys = [r["bytes_per_sec"] / 1e6 for r in g[s]]  # MB/s
        color = SAMPLE_COLORS.get(s)
        ax.plot(xs, ys, "o-", color=color, label=f"{s} samples")
    ax.axhline(network_cap_bytes_per_s / 1e6, color="k", ls="--", lw=1, alpha=0.6, label="1 Gbit/s line rate")
    ax.set_xscale("log")
    ax.set_xlabel("Target TestPulse rate per ch [Hz]")
    ax.set_ylabel("Reader bytes_read [MB/s]")
    ax.set_title(title)
    ax.grid(True, which="both", alpha=0.3)
    ax.legend(fontsize=8)


def plot_bpe(ax, rows, title):
    g = by_samples(rows)
    for s in sorted(g.keys()):
        xs = [r["target_rate_hz"] for r in g[s]]
        ys = [r["bytes_per_event"] for r in g[s]]
        color = SAMPLE_COLORS.get(s)
        ax.plot(xs, ys, "o-", color=color, label=f"{s} samples (got)")
        # predicted line
        ax.axhline(predicted_bytes_per_event(s), color=color, lw=1, ls=":", alpha=0.6)
    ax.set_xscale("log")
    ax.set_xlabel("Target TestPulse rate per ch [Hz]")
    ax.set_ylabel("bytes / event")
    ax.set_title(title)
    ax.grid(True, which="both", alpha=0.3)
    ax.legend(fontsize=8)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("csvs", nargs="+", type=Path)
    ap.add_argument("-o", "--out", type=Path, default=Path("throughput_plots.png"))
    ap.add_argument(
        "--cap",
        type=float,
        default=110e6,
        help="Network cap [B/s] for reference lines (default 110 MB/s ≈ 880 Mbps)",
    )
    args = ap.parse_args()

    datasets = [(p, filter_outliers(load(p))) for p in args.csvs]
    n = len(datasets)
    fig, axes = plt.subplots(3, n, figsize=(7 * n, 14), squeeze=False)
    for col, (path, rows) in enumerate(datasets):
        if not rows:
            print(f"[warn] empty: {path}", file=sys.stderr)
            continue
        nch = rows[0]["n_channels"]
        plot_panel(
            axes[0][col],
            rows,
            f"{path.stem}: achieved vs target ({nch}ch)",
            args.cap,
        )
        plot_bandwidth(
            axes[1][col],
            rows,
            f"{path.stem}: reader bandwidth ({nch}ch)",
            args.cap,
        )
        plot_bpe(axes[2][col], rows, f"{path.stem}: bytes/event ({nch}ch)")

    fig.suptitle("PSD2 @ 172.18.4.56 throughput — TestPulse trigger, 1 Gbit network", fontsize=14)
    fig.tight_layout(rect=[0, 0, 1, 0.97])
    fig.savefig(args.out, dpi=130)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
