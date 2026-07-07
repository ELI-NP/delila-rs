#!/usr/bin/env bash
# update_amax_fw.sh — regenerate ALL AMax firmware bindings from one RegisterFile.
#
# Why: every AMax FW rebuild shifts the register address map (and sometimes
# adds/drops registers). This used to need a human to hand-pick codegen flags,
# hand-edit a BROADCAST_BASE constant in handle.rs, then rebuild Rust + the
# Angular UI separately. `amax_codegen` now auto-derives the whole layout from
# RegisterFile.json, so this script chains the remaining steps into one command:
#
#   1. amax_codegen  → src/config/amax_generated.rs
#                      src/reader/caen/amax_registers_generated.rs
#                      web/operator-ui/src/app/models/amax-generated.ts
#   2. cargo fmt     (normalise the generated Rust)
#   3. cargo build --release --bins
#   4. npm run build (web/operator-ui → dist/, committed per CLAUDE.md policy)
#
# A new register only becomes type-safe + visible in the UI once it has a
# `tools/amax_viewer/fw_params.json` entry (label/category/type/...). The
# codegen step warns about RegisterFile registers that lack one.
#
# Full manual: docs/amax_fw_update_manual.md
#
# Usage:
#   scripts/update_amax_fw.sh <RegisterFile.json> [--with-viewer] [--no-ui]
#
#   --with-viewer  also regenerate amax_viewer's register_defs.json via gen_defs
#                  (best-effort; needs CAEN libs + a `Name`-bearing RegisterFile).
#   --no-ui        skip the Angular `npm run build` step (Rust-only iteration).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log()  { printf '\033[0;36m[update-amax]\033[0m %s\n' "$*"; }
warn() { printf '\033[0;33m[update-amax] WARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[0;31m[update-amax] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

REGFILE=""
WITH_VIEWER=0
DO_UI=1
for arg in "$@"; do
  case "$arg" in
    --with-viewer) WITH_VIEWER=1 ;;
    --no-ui)       DO_UI=0 ;;
    -h|--help)     tail -n +2 "$0" | grep '^#' | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*)            die "unknown option: $arg" ;;
    *)             [ -z "$REGFILE" ] || die "multiple RegisterFile args"; REGFILE="$arg" ;;
  esac
done

[ -n "$REGFILE" ] || die "usage: scripts/update_amax_fw.sh <RegisterFile.json> [--with-viewer] [--no-ui]"
[ -f "$REGFILE" ] || die "RegisterFile not found: $REGFILE"
command -v cargo >/dev/null || die "cargo not found (source ~/.cargo/env?)"

cd "$REPO"

# 1. Codegen (auto-derives PAGE_BASE/PAGE_STRIDE/BROADCAST_BASE; prints a summary
#    + warns about registers with no fw_params.json metadata).
log "codegen from $REGFILE"
cargo run --quiet --features dev-tools --bin amax_codegen -- "$REGFILE" \
  || die "amax_codegen failed (check the layout summary above)"

# 2. Normalise the generated Rust (codegen emits single-line writes; rustfmt
#    reflows them to match the committed style so diffs stay minimal).
log "cargo fmt"
cargo fmt

# 3. (optional) amax_viewer register_defs.json from the same RegisterFile.
if [ "$WITH_VIEWER" -eq 1 ]; then
  mkdir -p tools/amax_viewer/registers
  STAMP="$(basename "$REGFILE" .json)"
  OUT="tools/amax_viewer/registers/register_${STAMP}.json"
  log "gen_defs → $OUT (best-effort)"
  if (cd tools/amax_viewer && cargo run --quiet --bin gen_defs -- \
        "$REPO/$REGFILE" -p fw_params.json -o "$REPO/$OUT"); then
    log "viewer defs written: $OUT"
  else
    warn "gen_defs failed (needs CAEN libs + a Name-bearing RegisterFile) — skipped"
  fi
fi

# 4. Rust release build (proves the new register set compiles end-to-end).
log "cargo build --release --bins"
cargo build --release --bins

# 5. Angular UI build → committed dist/. Node is NOT installed everywhere (e.g.
#    the AMax dev box `gant` has cargo but no npm), so a missing toolchain
#    degrades gracefully: the generated TS SOURCE was already written by step 1,
#    only the compiled dist/ bundle is skipped and must be rebuilt elsewhere.
UI_BUILT=0
if [ "$DO_UI" -eq 0 ]; then
  log "skipping UI build (--no-ui)"
elif ! command -v npm >/dev/null; then
  warn "npm not found on $(hostname) — Angular dist/ NOT rebuilt."
  warn "The generated TS (amax-generated.ts) WAS updated; commit it, then on a"
  warn "Node-equipped machine rebuild + commit the dist/ bundle:"
  warn "    cd web/operator-ui && npm ci && npm run build"
else
  log "npm run build (web/operator-ui)"
  if (cd web/operator-ui && npm run build); then
    UI_BUILT=1
  else
    warn "npm run build failed — dist/ not refreshed. Fix and rerun the UI build."
  fi
fi

# 6. Report what changed so the developer can review + commit (dist/ included
#    per the Frontend Deployment Policy in CLAUDE.md).
log "done. changed files:"
git -C "$REPO" status --short -- \
  src/config/amax_generated.rs \
  src/reader/caen/amax_registers_generated.rs \
  web/operator-ui/src/app/models/amax-generated.ts \
  web/operator-ui/dist/ \
  tools/amax_viewer/registers/ || true

echo
log "Next: review the diff, then commit the generated Rust + TS sources:"
cat <<'EOF'
  git add src/config/amax_generated.rs \
          src/reader/caen/amax_registers_generated.rs \
          web/operator-ui/src/app/models/amax-generated.ts
EOF
if [ "$UI_BUILT" -eq 1 ]; then
  log "...and the rebuilt UI bundle (same commit):"
  echo "  git add web/operator-ui/dist/"
else
  warn "dist/ was NOT rebuilt here — rebuild + commit it on a Node machine so the"
  warn "operator UI reflects the new registers/tabs (served from the committed dist/)."
fi
