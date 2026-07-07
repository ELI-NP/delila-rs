#!/bin/bash
# Gzip every *.log file under logs/ EXCEPT the session directory that
# `logs/latest` points to (the live / most recent run). Each .log is
# compressed in place to .log.gz — directory structure is preserved
# so you can still drill into a specific session.
#
# Usage:
#   ./scripts/gzip_old_logs.sh            # default: ./logs
#   ./scripts/gzip_old_logs.sh /path/to/logs

set -eu

LOG_BASE="${1:-./logs}"

if [ ! -d "$LOG_BASE" ]; then
    echo "Log directory not found: $LOG_BASE" >&2
    exit 1
fi

# Resolve `latest` symlink to its session-dir basename (or empty if
# the symlink is missing). We compare basenames so the script doesn't
# care whether `latest` points to an absolute or relative path.
LATEST_TARGET=""
if [ -L "$LOG_BASE/latest" ]; then
    LATEST_TARGET="$(basename "$(readlink "$LOG_BASE/latest")")"
fi

echo "Log base:      $LOG_BASE"
echo "Skipping live: ${LATEST_TARGET:-<none>}"

compressed=0
skipped=0
already=0

# Iterate session dirs only (logs/20YYMMDD_HHMMSS/), not files at the
# top level. `-maxdepth 1` keeps it cheap; we descend per-session below.
while IFS= read -r -d '' session; do
    name="$(basename "$session")"
    if [ "$name" = "$LATEST_TARGET" ]; then
        echo "  skip (live): $name"
        skipped=$((skipped + 1))
        continue
    fi
    # gzip each .log inside this session. -f overwrites any stale .gz
    # left over from a previous interrupted run; without it gzip aborts.
    # Run in a subshell so a single failure doesn't kill the whole loop.
    while IFS= read -r -d '' logfile; do
        if gzip -f "$logfile" 2>/dev/null; then
            compressed=$((compressed + 1))
        else
            already=$((already + 1))
        fi
    done < <(find "$session" -maxdepth 1 -type f -name "*.log" -print0)
done < <(find "$LOG_BASE" -mindepth 1 -maxdepth 1 -type d -name "20*" -print0)

echo
echo "Compressed: $compressed file(s)"
echo "Skipped:    $skipped session(s) (live)"
[ "$already" -gt 0 ] && echo "Errors:     $already file(s)"
echo "Total size: $(du -sh "$LOG_BASE" 2>/dev/null | cut -f1)"
