#!/usr/bin/env python3
"""2D plots from `cfd_scan --dump EVENTS.csv` (TODO 90): per ThGEM, the FW
setting (top row) against the best setting of the scan (bottom row).

  python3 -I plot_events.py EVENTS.csv [--out events.png] [--thgem N ...]

Columns: XY image (x = t_XR - t_XL, y = t_YU - t_YD), the time sum
S_X = t_XL + t_XR - 2 t_T against x, and S_Y = t_YU + t_YD - 2 t_T against y.
A delay line makes S_X / S_Y independent of the position: a horizontal band
whose thickness is the timing resolution. A slope or a curve means the sum
depends on where the particle hit (dispersion along the line, or walk).

Needs numpy + matplotlib. License: BSD-3-Clause (same as delila-rs).
"""
import argparse
import csv
import math

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402


def sigma68(v):
    v = np.sort(np.asarray(v))
    if len(v) < 2:
        return math.nan
    lo, hi = np.quantile(v, [0.16, 0.84])
    return 0.5 * (hi - lo)


def load(path):
    data = {}
    with open(path) as f:
        for row in csv.DictReader(f):
            t = [float(row[k]) if row[k] else math.nan for k in ("t_T", "t_XL", "t_XR", "t_YU", "t_YD")]
            data.setdefault(int(row["thgem"]), {}).setdefault(row["set"], []).append(t)
    return {n: {s: np.array(v) for s, v in sets.items()} for n, sets in data.items()}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("csv")
    ap.add_argument("--out", default="events.png")
    ap.add_argument("--thgem", type=int, nargs="*")
    a = ap.parse_args()
    data = load(a.csv)
    thgems = sorted(n for n in data if not a.thgem or n in a.thgem)
    sets = ["fw", "best"]
    fig, axes = plt.subplots(2 * len(thgems), 3, figsize=(15, 4.2 * 2 * len(thgems)), squeeze=False)
    for i, n in enumerate(thgems):
        for j, s in enumerate(sets):
            ax = axes[2 * i + j]
            t = data[n].get(s)
            if t is None:
                for x in ax:
                    x.axis("off")
                continue
            T, XL, XR, YU, YD = t.T
            x, y = XR - XL, YU - YD
            sx, sy = XL + XR - 2 * T, YU + YD - 2 * T
            okxy = ~np.isnan(x) & ~np.isnan(y)
            okx, oky = ~np.isnan(sx), ~np.isnan(sy)
            ax[0].plot(x[okxy], y[okxy], ".", ms=3)
            ax[0].set_title(f"D{n} {s}: XY, {okxy.sum()} events with four arms")
            ax[0].set_xlabel("x = t_XR - t_XL [ns]")
            ax[0].set_ylabel("y = t_YU - t_YD [ns]")
            ax[1].plot(x[okx], sx[okx], ".", ms=3)
            ax[1].set_title(f"D{n} {s}: S_X vs x, sigma68 {sigma68(sx[okx]):.2f} ns ({okx.sum()})")
            ax[1].set_xlabel("x [ns]")
            ax[1].set_ylabel("S_X [ns]")
            ax[2].plot(y[oky], sy[oky], ".", ms=3)
            ax[2].set_title(f"D{n} {s}: S_Y vs y, sigma68 {sigma68(sy[oky]):.2f} ns ({oky.sum()})")
            ax[2].set_xlabel("y [ns]")
            ax[2].set_ylabel("S_Y [ns]")
    fig.tight_layout()
    fig.savefig(a.out, dpi=70)
    print(f"wrote {a.out}")


if __name__ == "__main__":
    main()
