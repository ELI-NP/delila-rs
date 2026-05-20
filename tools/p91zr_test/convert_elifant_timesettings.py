#!/usr/bin/env python3
"""Convert ELIFANT-Event's 4D `timeSettings.json` into the delila-rs SPEC
v0.5.1 tree form (`{version, entries: [{module, channel, ref, offset_ns}]}`).

ELIFANT layout:
    cfg[refMod][refCh][mod][ch].TimeOffset

Our convention: `aligned_ts = raw_ts - offset_ns`. Reference channel maps to
offset 0 (we subtract the self-self entry to normalise; ELIFANT's reference
row carries a non-zero clock baseline that we drop because the tree form
expresses everything relative to the root).

All channels end up as direct children of the root, so the resulting tree
is "flat" (depth 1) — equivalent to ELIFANT's single-reference model.
"""

import argparse
import json
from pathlib import Path


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--input",
        default="/Users/aogaki/WorkSpace/ELIFANT2025/ELIFANT-Event/all_run_p91Zr/timeSettings.json",
    )
    ap.add_argument(
        "--ref-module",
        type=int,
        default=9,
        help="Reference module index used by ELIFANT (settings.json TimeReferenceMod)",
    )
    ap.add_argument(
        "--ref-channel",
        type=int,
        default=0,
    )
    ap.add_argument("--output", default="timeSettings.json")
    args = ap.parse_args()

    with open(args.input) as f:
        elifant = json.load(f)

    rm, rc = args.ref_module, args.ref_channel
    # The slice we want: all (mod, ch) offsets relative to the reference.
    ref_slice = elifant[rm][rc]
    # ELIFANT carries an absolute clock baseline in cfg[rm][rc][rm][rc] —
    # subtract it so that ref-to-ref reads 0.0 in our tree.
    baseline = ref_slice[rm][rc]["TimeOffset"]

    entries = []
    for m_idx, mod_block in enumerate(ref_slice):
        for c_idx, ch_block in enumerate(mod_block):
            off = ch_block["TimeOffset"] - baseline
            # Root: (rm, rc) with offset 0.
            if m_idx == rm and c_idx == rc:
                entries.append({
                    "module": m_idx,
                    "channel": c_idx,
                    "ref": None,
                    "offset_ns": 0.0,
                })
            else:
                entries.append({
                    "module": m_idx,
                    "channel": c_idx,
                    "ref": [rm, rc],
                    "offset_ns": off,
                })

    tree = {
        "version": "1.0",
        "entries": entries,
    }

    out_path = Path(args.output)
    with open(out_path, "w") as f:
        json.dump(tree, f, indent=2)

    print(f"[ok] Wrote {out_path} ({len(entries)} entries, root=({rm}, {rc}))")
    print(f"     Baseline subtracted: {baseline:.3f} ns")
    print(f"     Offset range: {min(e['offset_ns'] for e in entries):.2f} .. "
          f"{max(e['offset_ns'] for e in entries):.2f} ns")


if __name__ == "__main__":
    main()
