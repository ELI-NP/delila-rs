# `eb_config.json` templates

Starting points for common L1 / L2 setups. Pick the closest match,
copy into your work directory, edit channel numbers / tags / etc. to
match your detector.

All templates declare `"$schema": "../../schemas/eb_config.schema.json"`
on the first line so editors with JSON Schema support (VSCode, IntelliJ,
NeoVim+LSP, …) will autocomplete and validate as you type.

After editing, sanity-check with:

```bash
event_builder validate-config my_eb_config.json
```

(also resolves L1 cross-references, L2 graph cycles, and the missing-accept
case — things a pure JSON Schema can't catch).

| File | When to start from this |
|---|---|
| `single_trigger.json` | One specific channel is the only trigger source; no physics cuts |
| `si_telescope.json` | ELIFANT-style Si ΔE-E coincidence: `Counter(E_Sector) + Counter(dE_Sector) + Flag(>0)×2 + Accept(AND)` |
| `multiplicity_2.json` | ≥ 2 distinct HPGe channels must fire within a window before an event is built (singles suppression) |
| `hpge_with_ac_veto.json` | HPGe trigger + Compton-suppression: drop events where an AC ring fired in coincidence |

Companion templates for `chSettings.json` and `timeSettings.json` live
elsewhere — see `docs/offline_event_builder_manual.md` § 4 for the
matching structure.
