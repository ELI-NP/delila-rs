#!/usr/bin/env bash
# provision_eliade_node.sh — bring a bare ELIADE DAQ node to the point where
# delila-rs can be built and a CAEN digitizer can be talked to.
#
# Written for the eliadeSN0x fleet (Ubuntu 20.04, 64 threads, A3818 optical
# link); also used for the ThGEM host pf-qaq (Ubuntu 24.04, kernel 7.0,
# 2026-10-05). Everything here is the part that needs root; the per-user bits
# (rustup, cargo build, ROOT, caen-toolbox) are deliberately left out so this
# stays a single, auditable sudo step.
#
# Usage:   sudo bash scripts/provision_eliade_node.sh [OPTIONS]
#   --user NAME        account that owns the checkout      (default: eliade)
#   --no-mongo         skip the MongoDB server install
#   --no-driver        skip the A3818 kernel driver install
#   --docker           install Docker + compose v2 (docker/ stack: MongoDB with
#                      auth, InfluxDB 3, Grafana, eLog) and add the user to the
#                      docker group. Use with --no-mongo: the stack's MongoDB
#                      listens on 27017 too.
#   --usb-driver       install CAEN's USB driver (desktop digitizers over USB)
#                      from external/CAENUSBdrvB-*.tar via DKMS
#   --x743-headers     put CAENDigitizer.h (+Type.h) from
#                      external/CAENDigitizer-*.tar into /opt/delila-caen/include
#                      so `cargo build --features x743` (V1743) finds them; the
#                      library itself comes with the dig1 bundle
#   --repo-url URL     delila-rs origin (default: ELI-NP/delila-rs)
#
# It is idempotent: re-running it upgrades/repairs rather than duplicating.
#
# Prerequisite staged by hand (it is gitignored, obtain it from CAEN):
#   /tmp/caen_dig1-*-bin.tar.gz*   — the dig1 backend package; copied into the
#   checkout's external/ so setup_caen_felib.sh can find it.
#
# What it does, in order:
#   1. apt packages: ROOT's documented dependency set (root.cern) + the build
#      chain delila-rs needs (dkms, libclang for bindgen, libzmq3-dev, ...)
#   2. MongoDB 7.0 from the upstream repo, enabled as a systemd service
#   3. clone delila-rs + its submodules as $USER (skipped if already there)
#   4. CAEN FELib into the isolated prefix /opt/delila-caen
#   5. the patched A3818 driver (v1.6.12-delila2) via DKMS
# NOTE: deliberately no `pipefail`. This script leans on `cmd | grep -q` and
# `cmd | head -1`, where the reader exits early and the writer dies of SIGPIPE.
# Under pipefail that non-zero status propagates and every such test silently
# reports failure — it made the package-availability probe below reject the
# entire package list on the first run across the fleet.
set -u

USER_NAME=eliade
REPO_URL=https://github.com/ELI-NP/delila-rs.git
DO_MONGO=1
DO_DRIVER=1
DO_DOCKER=0
DO_USB=0
DO_X743H=0

while [ $# -gt 0 ]; do
  case "$1" in
    --user)     USER_NAME="$2"; shift 2 ;;
    --repo-url) REPO_URL="$2";  shift 2 ;;
    --no-mongo)  DO_MONGO=0;  shift ;;
    --no-driver) DO_DRIVER=0; shift ;;
    --docker)       DO_DOCKER=1; shift ;;
    --usb-driver)   DO_USB=1;    shift ;;
    --x743-headers) DO_X743H=1;  shift ;;
    -h|--help)  sed -n '2,42p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

C='\033[0;36m'; G='\033[0;32m'; Y='\033[1;33m'; R='\033[0;31m'; N='\033[0m'
log()  { printf "${C}[provision]${N} %s\n" "$*"; }
ok()   { printf "${G}[provision] OK:${N} %s\n" "$*"; }
warn() { printf "${Y}[provision] WARN:${N} %s\n" "$*"; }
die()  { printf "${R}[provision] ERROR:${N} %s\n" "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root: sudo bash $0"
id "$USER_NAME" >/dev/null 2>&1 || die "no such user: $USER_NAME"
HOME_DIR=$(getent passwd "$USER_NAME" | cut -d: -f6)
REPO_DIR="$HOME_DIR/delila-rs"
as_user() { sudo -u "$USER_NAME" -H "$@"; }

log "host=$(hostname)  user=$USER_NAME  repo=$REPO_DIR"

# ---------------------------------------------------------------- 1. packages
# ROOT's own documented prerequisites (https://root.cern/install/dependencies/),
# required + optional, so ROOT stays buildable/servicable on every node — plus
# what delila-rs itself needs:
#   dkms          - A3818 driver, rebuilt automatically on kernel upgrades
#   libclang1-*   - bindgen generates the CAEN FELib FFI bindings against it
#                   (-10 on focal, -18 on noble; the filter keeps the one present)
#   libboost-dev  - headers for the CAEN dig2 backend build (setup_caen_felib.sh)
#   libzmq3-dev   - zmq.h for the C++ tools (root_sink); the Rust side vendors
#   build-essential/cmake/git/curl/rsync - the rest of the chain
PKGS="
binutils cmake dpkg-dev g++ gcc libssl-dev git libx11-dev libxext-dev
libxft-dev libxpm-dev python3 libtbb-dev libvdt-dev libgif-dev
gfortran libpcre3-dev libglu1-mesa-dev libglew-dev libftgl-dev fftw3-dev
libcfitsio-dev libgraphviz-dev avahi-compat-libdnssd-dev libldap2-dev
python3-dev python3-numpy libxml2-dev libkrb5-dev libgsl-dev qtwebengine5-dev
nlohmann-json3-dev libmysqlclient-dev libgl2ps-dev liblzma-dev libxxhash-dev
liblz4-dev libzstd-dev libcurl4-openssl-dev
dkms build-essential libclang1-10 libclang1-18 libboost-dev libzmq3-dev pkg-config curl rsync
"

log "apt-get update"
DEBIAN_FRONTEND=noninteractive apt-get update -qq || warn "apt-get update reported errors"

# Filter to what this release actually ships. On focal libvdt-dev and
# avahi-compat-libdnssd-dev do not exist; without this filter a single missing
# name would abort the whole install.
# awk (not `grep -q`) reads its input to EOF, so apt-cache is never SIGPIPEd —
# the probe then reports the truth no matter how the shell is configured.
AVAIL=""; SKIPPED=""
for p in $PKGS; do
  cand=$(LC_ALL=C apt-cache policy "$p" 2>/dev/null | awk '/Candidate:/ {c=$2} END {print c}')
  if [ -n "$cand" ] && [ "$cand" != "(none)" ]; then
    AVAIL="$AVAIL $p"
  else
    SKIPPED="$SKIPPED $p"
  fi
done
[ -n "$AVAIL" ] || die "no candidate packages resolved at all — apt metadata looks broken; check 'apt-get update' output above"
[ -n "$SKIPPED" ] && warn "not in this release, skipping:$SKIPPED"

log "installing $(echo $AVAIL | wc -w) packages (this takes a few minutes)"
# shellcheck disable=SC2086
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $AVAIL \
  || die "package install failed"
ok "packages installed"

# Kernel headers for the running kernel — DKMS cannot build the driver without.
if [ ! -d "/usr/src/linux-headers-$(uname -r)" ]; then
  log "installing kernel headers for $(uname -r)"
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "linux-headers-$(uname -r)" \
    || warn "kernel headers unavailable — the A3818 driver will not build"
fi

# ----------------------------------------------------------------- 2. MongoDB
if [ "$DO_MONGO" -eq 1 ]; then
  if systemctl is-active --quiet mongod; then
    ok "MongoDB already running"
  else
    log "installing MongoDB 7.0"
    . /etc/os-release
    curl -fsSL https://pgp.mongodb.com/server-7.0.asc \
      | gpg --dearmor -o /usr/share/keyrings/mongodb-server-7.0.gpg --yes 2>/dev/null
    echo "deb [ arch=amd64,arm64 signed-by=/usr/share/keyrings/mongodb-server-7.0.gpg ] https://repo.mongodb.org/apt/ubuntu ${UBUNTU_CODENAME}/mongodb-org/7.0 multiverse" \
      > /etc/apt/sources.list.d/mongodb-org-7.0.list
    DEBIAN_FRONTEND=noninteractive apt-get update -qq
    if DEBIAN_FRONTEND=noninteractive apt-get install -y -qq mongodb-org; then
      systemctl enable --now mongod && ok "MongoDB running on 127.0.0.1:27017 (no auth)"
    else
      warn "MongoDB install failed — run history will not be persisted"
    fi
  fi
else
  log "skipping MongoDB (--no-mongo)"
fi

# ------------------------------------------------------------------- 3. clone
if [ -d "$REPO_DIR/.git" ]; then
  ok "checkout already present: $REPO_DIR ($(as_user git -C "$REPO_DIR" log --oneline -1 2>/dev/null))"
else
  log "cloning $REPO_URL -> $REPO_DIR"
  as_user git clone -q "$REPO_URL" "$REPO_DIR" || die "clone failed"
fi
log "initialising submodules (caen-felib, caen-a3818-driver)"
as_user git -C "$REPO_DIR" submodule update --init --recursive -- \
  external/caen-felib external/caen-a3818-driver >/dev/null 2>&1 \
  || warn "submodule init reported errors"

# The dig1 backend package is CAEN's, gitignored, and must be staged by hand.
if ! ls "$REPO_DIR"/external/caen_dig1-*-bin.tar.gz* >/dev/null 2>&1; then
  STAGED=$(ls /tmp/caen_dig1-*-bin.tar.gz* 2>/dev/null | head -1)
  if [ -n "$STAGED" ]; then
    log "staging $(basename "$STAGED") into external/"
    install -o "$USER_NAME" -g "$(id -gn "$USER_NAME")" -m 644 \
      "$STAGED" "$REPO_DIR/external/$(basename "$STAGED")"
  fi
fi

# -------------------------------------------------------------------- 4. FELib
if [ -e /opt/delila-caen/lib/libCAEN_FELib.so ]; then
  ok "CAEN FELib already installed in /opt/delila-caen"
elif ls "$REPO_DIR"/external/caen_dig1-*-bin.tar.gz* >/dev/null 2>&1; then
  log "installing CAEN FELib -> /opt/delila-caen"
  if bash "$REPO_DIR/scripts/setup_caen_felib.sh" >/tmp/felib_setup.log 2>&1; then
    ok "FELib installed (log: /tmp/felib_setup.log)"
  else
    warn "FELib setup failed — see /tmp/felib_setup.log"
  fi
else
  warn "no external/caen_dig1-*-bin.tar.gz — skipping FELib."
  warn "  stage it at /tmp/ and re-run, or delila-rs will not build."
fi

# ------------------------------------------------------------- 5. A3818 driver
if [ "$DO_DRIVER" -eq 0 ]; then
  log "skipping A3818 driver (--no-driver)"
elif [ ! -d "$REPO_DIR/external/caen-a3818-driver" ]; then
  warn "a3818 submodule missing — skipping driver"
else
  CUR=$(dkms status a3818 2>/dev/null | awk 'NR==1')
  case "$CUR" in
    *1.6.12*)
    ok "patched A3818 driver already installed: $CUR" ;;
  *)
    # Remove CAEN's stock driver first; ours is the patched v1.6.12-delila2.
    for v in $(dkms status a3818 2>/dev/null | sed -E 's#^a3818[/,] *([^,]+),.*#\1#' | sort -u); do
      [ "$v" = "1.6.12" ] && continue
      log "removing stock A3818 driver $v"
      dkms remove "a3818/$v" --all >/dev/null 2>&1 || true
    done
    log "building + installing patched A3818 driver v1.6.12-delila2"
    if (cd "$REPO_DIR/external/caen-a3818-driver" && bash install.sh) >/tmp/a3818_install.log 2>&1; then
      ok "A3818 driver installed: $(dkms status a3818 2>/dev/null | head -1)"
    else
      # install.sh ends with `modprobe a3818`; with no card fitted the module
      # loads but binds nothing, and that is not a failure for us — what
      # matters is that DKMS registered it so it survives kernel upgrades.
      if [ -n "$(dkms status a3818 2>/dev/null | grep installed)" ]; then
        warn "driver installed but modprobe reported an issue (expected with no A3818 card fitted)"
        dkms status a3818
      else
        warn "A3818 driver install failed — see /tmp/a3818_install.log"
      fi
    fi ;;
  esac
  [ "$(lspci 2>/dev/null | grep -ciE 'xilinx|caen')" != "0" ] \
    || warn "no A3818 card detected on the PCIe bus — driver is staged for when one is fitted"
fi

# ------------------------------------------------------------- 6. extras
X743_INC=/opt/delila-caen/include
if [ "$DO_X743H" -eq 1 ]; then
  T=$(ls "$REPO_DIR"/external/CAENDigitizer-*.tar 2>/dev/null | head -1)
  if [ -z "$T" ]; then
    warn "--x743-headers: no external/CAENDigitizer-*.tar — skipping"
  else
    TMPX=$(mktemp -d); tar xf "$T" -C "$TMPX"
    mkdir -p "$X743_INC"
    if install -m 644 "$TMPX"/*/include/CAENDigitizer.h "$TMPX"/*/include/CAENDigitizerType.h "$X743_INC/"; then
      ok "CAENDigitizer headers ($(basename "$T" .tar)) -> $X743_INC"
    else
      warn "CAENDigitizer headers not found inside $(basename "$T")"
    fi
    rm -rf "$TMPX"
  fi
fi

if [ "$DO_USB" -eq 1 ]; then
  T=$(ls "$REPO_DIR"/external/CAENUSBdrvB-*.tar 2>/dev/null | head -1)
  if [ -z "$T" ]; then
    warn "--usb-driver: no external/CAENUSBdrvB-*.tar — skipping"
  else
    TMPU=$(mktemp -d); tar xf "$T" -C "$TMPU"
    log "installing CAEN USB driver $(basename "$T" .tar) via DKMS"
    if (cd "$TMPU"/CAENUSBdrvB-* && bash install.sh) >/tmp/caenusb_install.log 2>&1; then
      ok "CAEN USB driver: $(dkms status CAENUSBdrvB 2>/dev/null | head -1)"
    else
      warn "CAEN USB driver install reported errors — see /tmp/caenusb_install.log"
    fi
    rm -rf "$TMPU"
  fi
fi

if [ "$DO_DOCKER" -eq 1 ]; then
  log "installing Docker + compose v2 (distribution packages)"
  if DEBIAN_FRONTEND=noninteractive apt-get install -y -qq docker.io docker-compose-v2; then
    systemctl enable --now docker >/dev/null 2>&1
    usermod -aG docker "$USER_NAME"
    ok "Docker $(docker --version 2>/dev/null | awk '{print $3}' | tr -d ,) + $(docker compose version 2>/dev/null | head -1); $USER_NAME added to group docker (re-login to take effect)"
  else
    warn "Docker install failed"
  fi
fi

# ------------------------------------------------------------------ 7. summary
echo
log "=== summary for $(hostname) ==="
printf '  %-22s %s\n' "FELib"        "$([ -e /opt/delila-caen/lib/libCAEN_FELib.so ] && echo /opt/delila-caen || echo MISSING)"
printf '  %-22s %s\n' "A3818 dkms"   "$(dkms status a3818 2>/dev/null | head -1 || echo none)"
printf '  %-22s %s\n' "A3818 card"   "$(lspci 2>/dev/null | grep -icE 'xilinx|caen') device(s) on PCIe"
printf '  %-22s %s\n' "MongoDB"      "$(systemctl is-active mongod 2>/dev/null || echo not-installed)"
printf '  %-22s %s\n' "checkout"     "$(as_user git -C "$REPO_DIR" log --oneline -1 2>/dev/null || echo MISSING)"
printf '  %-22s %s\n' "kernel hdrs"  "$([ -d "/usr/src/linux-headers-$(uname -r)" ] && echo present || echo MISSING)"
[ "$DO_USB" -eq 1 ]    && printf '  %-22s %s\n' "CAEN USB dkms" "$(dkms status CAENUSBdrvB 2>/dev/null | head -1 || echo none)"
[ "$DO_X743H" -eq 1 ]  && printf '  %-22s %s\n' "x743 headers"  "$([ -e "$X743_INC/CAENDigitizer.h" ] && echo "$X743_INC" || echo MISSING)"
[ "$DO_DOCKER" -eq 1 ] && printf '  %-22s %s\n' "Docker"        "$(systemctl is-active docker 2>/dev/null || echo not-installed)"
echo
ok "root-level provisioning done. Remaining per-user steps (no sudo): rustup,"
log "  cargo build, ROOT, caen-toolbox, ~/.bashrc — see docs/eliade_node_setup.md"
