#!/usr/bin/env python3
"""Convert ELIFANT-Event p91Zr config (chSettings.json + L2Settings.json +
settings.json) to delila-rs SPEC v0.5.1 equivalents (eb_config.json +
chSettings.json slim form).

Run:

    python3 convert_elifant_config.py
        --elifant-dir /Users/aogaki/WorkSpace/ELIFANT2025/ELIFANT-Event/all_run_p91Zr
        --out-dir /Users/aogaki/WorkSpace/delila-rs/tools/p91zr_test

Outputs:
    - chSettings.json  (slim: ID/Module/Channel/DetectorType/Tags/p0..p3)
    - eb_config.json   (timing + L1 or-of-triggers + L2 from L2Settings)

`timeSettings.json` is NOT generated here — the user wants delila-rs's own
EB to measure offsets later via time_calibrator.
"""

import argparse
import json
from pathlib import Path


# Map ELIFANT L2 operator strings to our snake_case CmpOp.
# Both are the literal symbol "==" / "<" / ... so identity works.
CMP_OPS = {"==", "!=", "<", "<=", ">", ">="}
LOGIC_OPS = {"AND", "OR"}


def load_json(path: Path):
    with open(path) as f:
        return json.load(f)


def build_slim_ch_settings(elifant_ch: list) -> list:
    """Drop the fields removed by Phase J (IsEventTrigger/HasAC/AC*/ThresholdADC
    + the geometry fields ELIFANT carried but our SPEC § 4.2 doesn't define).
    Keep: ID, Module, Channel, DetectorType, Tags, p0..p3."""
    slim = []
    for mod in elifant_ch:
        slim_mod = []
        for ch in mod:
            slim_mod.append({
                "ID": ch["ID"],
                "Module": ch["Module"],
                "Channel": ch["Channel"],
                "DetectorType": ch["DetectorType"],
                "Tags": ch.get("Tags", []),
                "p0": ch.get("p0", 0.0),
                "p1": ch.get("p1", 1.0),
                "p2": ch.get("p2", 0.0),
                "p3": ch.get("p3", 0.0),
            })
        slim.append(slim_mod)
    return slim


def build_l1_or_of_triggers(elifant_ch: list, trigger_min_adc: int = 0) -> dict:
    """ELIFANT marks each trigger channel with IsEventTrigger=true. Map this
    to one `channel` op per trigger + a top-level `or` op named `trigger`.

    When `trigger_min_adc > 0`, wrap each `channel` op in an `energy_gate`
    so that hits below the threshold do NOT become trigger anchors
    (SPEC § 5.2 trigger gate — kills the very-low-energy noise floor that
    otherwise produces accidental coincidences).
    """
    definitions = []
    or_inputs = []
    for mod in elifant_ch:
        for ch in mod:
            if not ch.get("IsEventTrigger", False):
                continue
            ch_name = f"trg_M{ch['Module']:02d}_C{ch['Channel']:02d}"
            definitions.append({
                "type": "channel",
                "name": ch_name,
                "module": ch["Module"],
                "channel": ch["Channel"],
            })
            if trigger_min_adc > 0:
                gated_name = f"{ch_name}_gate"
                definitions.append({
                    "type": "energy_gate",
                    "name": gated_name,
                    "source": ch_name,
                    "min_adc": trigger_min_adc,
                    "max_adc": 65535,
                })
                or_inputs.append(gated_name)
            else:
                or_inputs.append(ch_name)

    definitions.append({
        "type": "or",
        "name": "trigger",
        "inputs": or_inputs,
    })
    return {"definitions": definitions, "trigger": "trigger"}


def convert_l2_op(op: dict) -> dict:
    """Translate one ELIFANT L2 op to our snake_case form.

    ELIFANT uses PascalCase keys (`Type`, `Name`, `Tags`, `Monitor`,
    `Operator`, `Value`); we use snake_case + `type` tag.
    """
    t = op["Type"]
    if t == "Counter":
        return {
            "type": "counter",
            "name": op["Name"],
            "tags": op.get("Tags", []),
        }
    if t == "Flag":
        operator = op["Operator"]
        if operator not in CMP_OPS:
            raise ValueError(f"Unknown Flag operator: {operator}")
        return {
            "type": "flag",
            "name": op["Name"],
            "monitor": op["Monitor"],
            "operator": operator,
            "value": int(op["Value"]),
        }
    if t == "Accept":
        operator = op["Operator"]
        if operator not in LOGIC_OPS:
            raise ValueError(f"Unknown Accept operator: {operator}")
        monitor = op["Monitor"]
        if isinstance(monitor, str):
            monitor = [monitor]
        return {
            "type": "accept",
            "name": op["Name"],
            "monitor": monitor,
            "operator": operator,
        }
    raise ValueError(f"Unknown L2 op Type: {t}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--elifant-dir",
        default="/Users/aogaki/WorkSpace/ELIFANT2025/ELIFANT-Event/all_run_p91Zr",
        help="Path to ELIFANT-Event/all_run_p91Zr (containing chSettings.json etc.)",
    )
    ap.add_argument("--out-dir", required=True, help="Where to write the converted files")
    ap.add_argument(
        "--coincidence-window-ns",
        type=float,
        default=None,
        help="Override CoincidenceWindow from settings.json",
    )
    ap.add_argument(
        "--trigger-min-adc",
        type=int,
        default=0,
        help="If > 0, wrap every L1 channel op in an `energy_gate` "
             "with `min_adc` set to this and `max_adc` = 65535. "
             "Acts as a noise floor on trigger candidates.",
    )
    args = ap.parse_args()

    elifant_dir = Path(args.elifant_dir)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    # Load inputs
    settings = load_json(elifant_dir / "settings.json")
    ch_in_path = elifant_dir / settings["ChannelSettings"]
    l2_in_path = elifant_dir / settings["L2Settings"]
    elifant_ch = load_json(ch_in_path)
    elifant_l2 = load_json(l2_in_path)

    # Slim chSettings.json
    slim_ch = build_slim_ch_settings(elifant_ch)
    with open(out_dir / "chSettings.json", "w") as f:
        json.dump(slim_ch, f, indent=2)
    print(f"[ok] chSettings.json -> {out_dir / 'chSettings.json'}")

    # eb_config.json
    coincidence = args.coincidence_window_ns or settings["CoincidenceWindow"]
    l1 = build_l1_or_of_triggers(elifant_ch, trigger_min_adc=args.trigger_min_adc)
    l2 = [convert_l2_op(op) for op in elifant_l2]

    eb_config = {
        "version": "1.0",
        "timing": {
            "coincidence_window_ns": coincidence,
            "buffer_delay_ns": 1.0e9,
            "slice_duration_ns": 1.0e7,
        },
        "channels_file": "chSettings.json",
        "time_offsets_file": "timeSettings.json",
        "l1": l1,
        "l2": l2,
        "output": {
            "events_per_file": 1_000_000,
            "directory": "./eb_output",
            "zmq_pub_endpoint": "tcp://*:5610",
        },
    }
    with open(out_dir / "eb_config.json", "w") as f:
        json.dump(eb_config, f, indent=2)
    print(f"[ok] eb_config.json   -> {out_dir / 'eb_config.json'}")

    # Summary
    n_trig = sum(1 for op in l1["definitions"] if op["type"] == "channel")
    print()
    print("=== Summary ===")
    print(f"  Source: {elifant_dir}")
    print(f"  Modules: {len(elifant_ch)}")
    print(f"  L1 trigger channels: {n_trig}")
    print(f"  L2 ops: {len(l2)}")
    print(f"  Coincidence window: {coincidence} ns")


if __name__ == "__main__":
    main()
