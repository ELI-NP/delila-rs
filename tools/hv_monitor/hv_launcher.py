#!/usr/bin/env python3
"""HV Monitor Launcher — supervises N1408 and SY5527 monitor subprocesses.

Each monitor runs as an independent child process. If one crashes, only that
process is restarted (with backoff). SIGTERM/SIGINT cleanly shuts down all children.

Usage:
    python3 hv_launcher.py --config launcher_config.yaml
"""

import argparse
import logging
import os
import signal
import subprocess
import sys
import time

import yaml

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
log = logging.getLogger("hv_launcher")

RESTART_DELAY_BASE = 5      # seconds
RESTART_DELAY_MAX = 60      # seconds — caps exponential backoff
RESTART_RESET_AFTER = 300   # seconds — reset backoff after stable run


class MonitorProcess:
    """Manages one child monitor process with auto-restart."""

    def __init__(self, name: str, cmd: list[str], env: dict | None = None):
        self.name = name
        self.cmd = cmd
        self.env = env
        self.proc: subprocess.Popen | None = None
        self.restart_count = 0
        self.last_start: float = 0

    def start(self):
        merged_env = os.environ.copy()
        if self.env:
            merged_env.update(self.env)

        self.proc = subprocess.Popen(
            self.cmd,
            env=merged_env,
            stdout=sys.stdout,
            stderr=sys.stderr,
        )
        self.last_start = time.time()
        log.info("[%s] Started (PID %d): %s", self.name, self.proc.pid,
                 " ".join(self.cmd))

    def poll(self) -> bool:
        """Check if process is still running. Returns True if alive."""
        if self.proc is None:
            return False
        return self.proc.poll() is None

    def restart_if_dead(self) -> bool:
        """Restart if crashed. Returns True if a restart happened."""
        if self.poll():
            return False

        retcode = self.proc.returncode if self.proc else "?"
        uptime = time.time() - self.last_start

        # Reset backoff if it ran long enough
        if uptime > RESTART_RESET_AFTER:
            self.restart_count = 0

        delay = min(RESTART_DELAY_BASE * (2 ** self.restart_count),
                    RESTART_DELAY_MAX)
        self.restart_count += 1

        log.warning("[%s] Exited (code=%s, uptime=%.0fs). "
                    "Restarting in %ds (attempt %d)...",
                    self.name, retcode, uptime, delay, self.restart_count)
        time.sleep(delay)
        self.start()
        return True

    def stop(self):
        if self.proc is None or self.proc.poll() is not None:
            return
        log.info("[%s] Sending SIGTERM to PID %d", self.name, self.proc.pid)
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            log.warning("[%s] SIGKILL PID %d", self.name, self.proc.pid)
            self.proc.kill()
            self.proc.wait()


def load_config(path: str) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def main():
    parser = argparse.ArgumentParser(description="HV Monitor Launcher")
    parser.add_argument("--config", default="launcher_config.yaml",
                        help="Path to launcher YAML config")
    args = parser.parse_args()

    config = load_config(args.config)
    base_dir = config.get("base_dir", os.path.dirname(os.path.abspath(args.config)))
    monitors = []

    for entry in config["monitors"]:
        if not entry.get("enabled", True):
            log.info("Skipping disabled monitor: %s", entry["name"])
            continue

        cmd = [sys.executable, "-u", os.path.join(base_dir, entry["script"])]
        if "config" in entry:
            cmd.extend(["--config", os.path.join(base_dir, entry["config"])])

        monitors.append(MonitorProcess(
            name=entry["name"],
            cmd=cmd,
            env=entry.get("env"),
        ))

    if not monitors:
        log.error("No monitors configured")
        sys.exit(1)

    # Handle SIGTERM/SIGINT
    shutdown = False

    def on_signal(signum, _frame):
        nonlocal shutdown
        shutdown = True
        log.info("Received signal %d, shutting down...", signum)

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)

    # Start all monitors
    for m in monitors:
        m.start()

    log.info("Launcher running: %d monitor(s)", len(monitors))

    # Supervision loop
    while not shutdown:
        for m in monitors:
            m.restart_if_dead()
        time.sleep(2)

    # Clean shutdown
    for m in monitors:
        m.stop()
    log.info("All monitors stopped. Exiting.")


if __name__ == "__main__":
    main()
