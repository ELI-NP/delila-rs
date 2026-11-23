#!/usr/bin/env python3
"""Apply a cfd_scan patch (channel_overrides per digitizer) through the
Operator REST API (TODO 90 P4).

  python3 -I apply_patch.py patch.json                 # show what would change
  python3 -I apply_patch.py patch.json --apply         # PUT + save, with a backup
  python3 -I apply_patch.py --self-test

Patch format (cfd_scan --out):
  {"<digitizer id>": {"channel_overrides": {"<ch>": {"cfd_delay_ns": 20, ...}}}}

Only the keys in the patch are changed; every other setting of the channel and
the board is left as it is. Without --apply nothing is written. With --apply the
current configs are first saved to a backup file, then PUT + save per digitizer,
and the result is read back. Refuses while a run or Tune Up is active.

Standard library only. License: BSD-3-Clause (same as delila-rs).
"""
import argparse
import copy
import json
import sys
import time
import urllib.request


def req(base, method, path, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(base + path, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=timeout) as f:
        return json.loads(f.read().decode() or "null")


def effective(cfg, ch, key):
    o = cfg.get("channel_overrides", {}).get(str(ch), {})
    return o.get(key, cfg.get("channel_defaults", {}).get(key))


def merge(cfg, module_patch):
    """New config with the patch's channel keys written as overrides; returns
    (new_config, [(channel, key, before, after), ...]) for the keys that change."""
    new = copy.deepcopy(cfg)
    ovr = new.setdefault("channel_overrides", {})
    changes = []
    for ch, kv in module_patch.get("channel_overrides", {}).items():
        for k, v in kv.items():
            before = effective(cfg, ch, k)
            if before != v:
                changes.append((int(ch), k, before, v))
            ovr.setdefault(str(ch), {})[k] = v
    return new, sorted(changes)


def self_test():
    cfg = {"channel_defaults": {"cfd_delay_ns": 20, "cfd_fraction": "CFD_FRACTLIST_50", "polarity": "P"},
           "channel_overrides": {"3": {"cfd_delay_ns": 30}}}
    new, ch = merge(cfg, {"channel_overrides": {"3": {"cfd_delay_ns": 40}, "5": {"cfd_fraction": "CFD_FRACTLIST_75"},
                                                "6": {"cfd_delay_ns": 20}}})
    assert new["channel_overrides"]["3"] == {"cfd_delay_ns": 40}
    assert new["channel_overrides"]["5"] == {"cfd_fraction": "CFD_FRACTLIST_75"}
    assert new["channel_defaults"] == cfg["channel_defaults"]
    assert cfg["channel_overrides"] == {"3": {"cfd_delay_ns": 30}}, "input must not be modified"
    assert ch == [(3, "cfd_delay_ns", 30, 40), (5, "cfd_fraction", "CFD_FRACTLIST_50", "CFD_FRACTLIST_75")], ch
    assert effective(new, 6, "cfd_delay_ns") == 20
    print("self-test OK")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("patch", nargs="?")
    ap.add_argument("--operator", default="http://localhost:9091")
    ap.add_argument("--apply", action="store_true", help="write (default: only show the changes)")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        self_test()
        return
    if not a.patch:
        ap.error("patch file required")
    api = a.operator.rstrip("/") + "/api"
    patch = json.load(open(a.patch))

    st = req(api, "GET", "/status")
    if a.apply and (st.get("run_info") or st.get("tuneup_mode")):
        sys.exit("refused: a run or Tune Up is active")
    current = {m: req(api, "GET", f"/digitizers/{m}") for m in patch}
    plans = {}
    for m, mp in patch.items():
        new, changes = merge(current[m], mp)
        plans[m] = new
        print(f"digitizer {m} ({current[m].get('name', '')}): {len(changes)} change(s)")
        for ch, k, b, v in changes:
            print(f"   ch{ch:<2} {k}: {b} -> {v}")
    if not a.apply:
        print("dry run: nothing written (add --apply)")
        return
    bfile = time.strftime("/tmp/apply_patch_backup_%Y%m%d_%H%M%S.json")
    with open(bfile, "w") as f:
        json.dump(current, f, indent=1)
    print(f"backup -> {bfile}")
    for m, new in plans.items():
        req(api, "PUT", f"/digitizers/{m}", new)
        req(api, "POST", f"/digitizers/{m}/save")
        ok = req(api, "GET", f"/digitizers/{m}") == new
        print(f"digitizer {m}: saved, reads back as written: {ok}")
    print("applied; the next Configure / run start pushes it to the boards")


if __name__ == "__main__":
    main()
