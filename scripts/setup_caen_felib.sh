#!/usr/bin/env bash
# setup_caen_felib.sh — install the CAEN FELib stack into an ISOLATED prefix.
#
# Why: delila-rs talks to digitizers through CAEN FELib + its dig1 (Gen1) /
# dig2 (Gen2) backends. The official dig1 binary package bundles its own copy
# of CAENVME/Comm/Digitizer and wants them in /usr/local/lib, which on a shared
# machine would SHADOW the system CAEN libs that other software (CoMPASS) uses.
# To avoid breaking anyone, we install delila-rs's CAEN stack under a private
# PREFIX (default /opt/delila-caen) and point the build at it via CAEN_PREFIX,
# baking an rpath + using LD_LIBRARY_PATH at runtime. The system /lib + /usr
# CAEN libs are left untouched.
#
# The ONE unavoidable system path: the dig1 DPP libraries hardcode
# /usr/local/share/dpp-digitizer for their XML machinery (verified with
# `strings libCAENDPPDigitizer.so`). Those XMLs are additive data, not used by
# CoMPASS, so placing them there is harmless.
#
# Usage:   sudo bash scripts/setup_caen_felib.sh [PREFIX]
# Default: PREFIX=/opt/delila-caen
# Inputs (relative to repo root):
#   - external/caen-felib/                      (git submodule, FELib core source)
#   - external/caen_dig1-*-bin.tar.gz*          (CAEN dig1 binary package, from CAEN)
#   - external/caen-dig2/ (optional, Gen2)      (git submodule; built if present)
#
# Re-runnable and machine-portable: on a new host, clone the repo, drop the
# CAEN tarball into external/, run this once with sudo, then
#   CAEN_PREFIX=<PREFIX> cargo build --release --bins
set -euo pipefail

PREFIX="${1:-/opt/delila-caen}"
XMLDIR="/usr/local/share/dpp-digitizer"   # hardcoded in libCAENDPPDigitizer.so
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FELIB_SRC="$REPO/external/caen-felib"
DIG2_SRC="$REPO/external/caen-dig2"

log() { printf '\033[0;36m[setup-caen]\033[0m %s\n' "$*"; }
die() { printf '\033[0;31m[setup-caen] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root (sudo): writes to $PREFIX and $XMLDIR"
command -v cmake >/dev/null || die "cmake not found"
[ -d "$FELIB_SRC" ] || die "missing $FELIB_SRC — run: git submodule update --init external/caen-felib"

DIG1_TARBALL="$(ls "$REPO"/external/caen_dig1-*-bin.tar.gz* 2>/dev/null | head -1 || true)"
[ -n "$DIG1_TARBALL" ] || die "missing external/caen_dig1-*-bin.tar.gz* (get it from CAEN; it is gitignored)"

log "PREFIX=$PREFIX   dig1=$(basename "$DIG1_TARBALL")"
mkdir -p "$PREFIX/lib" "$PREFIX/include" "$XMLDIR"

# 1) FELib core: build + install into PREFIX -------------------------------
log "Building + installing CAEN FELib core -> $PREFIX"
cmake -S "$FELIB_SRC" -B "$FELIB_SRC/build" -DCMAKE_INSTALL_PREFIX="$PREFIX" >/dev/null
cmake --build "$FELIB_SRC/build" -j"$(nproc)" >/dev/null
cmake --install "$FELIB_SRC/build" >/dev/null

# 2) dig1 backend: unwrap the (sometimes double-gzipped) package -----------
log "Extracting dig1 binary package"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp "$DIG1_TARBALL" "$STAGE/pkg"
# Strip any gzip layers until we reach the tar, then extract.
while file "$STAGE/pkg" | grep -q "gzip compressed"; do
  mv "$STAGE/pkg" "$STAGE/pkg.gz"; gunzip -f "$STAGE/pkg.gz"
done
tar xf "$STAGE/pkg" -C "$STAGE"
# The package nests a per-arch tarball under bin/.
ARCH_TGZ="$(find "$STAGE" -name 'caen_dig1-*-x86_64-linux-gnu.tar.gz' | head -1)"
[ -n "$ARCH_TGZ" ] || die "x86_64 dig1 archive not found inside package"
mkdir -p "$STAGE/x"; tar xzf "$ARCH_TGZ" -C "$STAGE/x"

# 3) Install dig1 libs into PREFIX/lib (NOT system), XMLs into the hardcoded dir.
#    Copy ALL bundled libs (FELib backend + the matched CAEN DPP/base stack) so
#    delila-rs uses a self-consistent set; the system libs stay untouched.
log "Installing dig1 libraries -> $PREFIX/lib"
cp -a "$STAGE"/x/usr/local/lib/*.so* "$PREFIX/lib/"
log "Installing dpp-digitizer XMLs -> $XMLDIR (hardcoded path)"
cp -a "$STAGE"/x/usr/local/share/dpp-digitizer/*.xml "$XMLDIR/"

# 4) dig2 backend (Gen2) — optional; ELIADE is Gen1-only so a dig2 build
#    failure must NOT abort the (already-successful) dig1 install. Fully
#    non-fatal: the whole block runs in a subshell guarded with `|| true`.
if [ -d "$DIG2_SRC" ] && [ -f "$DIG2_SRC/CMakeLists.txt" ]; then
  log "Building + installing CAEN dig2 backend -> $PREFIX (optional)"
  (
    cmake -S "$DIG2_SRC" -B "$DIG2_SRC/build" -DCMAKE_INSTALL_PREFIX="$PREFIX" \
          -DCMAKE_PREFIX_PATH="$PREFIX" >/dev/null &&
    cmake --build "$DIG2_SRC/build" -j"$(nproc)" >/dev/null &&
    cmake --install "$DIG2_SRC/build" >/dev/null
  ) || log "dig2 backend skipped (non-fatal; not needed for Gen1/ELIADE)"
fi

log "Done. Installed under $PREFIX (libs) + $XMLDIR (dig1 XML)."
echo
echo "  Build:   CAEN_PREFIX=$PREFIX cargo build --release --bins"
echo "  Runtime: export LD_LIBRARY_PATH=$PREFIX/lib   (so FELib finds the dig1 backend)"
echo
ls -1 "$PREFIX/lib" | grep -iE 'CAEN_FELib|CAEN_Dig1' || log "WARNING: FELib/Dig1 libs not visible in $PREFIX/lib"
