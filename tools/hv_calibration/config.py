"""YAML configuration loader for gain matcher."""

import logging
from dataclasses import dataclass, field

import yaml

logger = logging.getLogger(__name__)


@dataclass
class ChannelConfig:
    """One detector channel mapping: HV ↔ Digitizer."""
    name: str
    hv_slot: int
    hv_ch: int
    dig_module: int
    dig_ch: int
    peak_region: tuple[int, int] = (300, 500)
    target_position: int = 1000
    skip: bool = False


@dataclass
class GainMatcherConfig:
    """Full configuration for gain matching."""
    # HV
    hv_host: str = "172.18.5.215"
    hv_username: str = "admin"
    hv_password: str = "eli-np"

    # DAQ
    operator_url: str = "http://localhost:8080"
    monitor_url: str = "http://localhost:8081"

    # Matching parameters
    measure_time: int = 10
    max_iterations: int = 10
    tolerance_percent: float = 2.0
    pmt_alpha: float = 7.0
    min_counts: int = 1000
    hv_step_limit: float = 50.0
    settling_time: float = 5.0  # PMT settling time after HV ramp (seconds)

    # Channels
    channels: list[ChannelConfig] = field(default_factory=list)


def load_config(path: str) -> GainMatcherConfig:
    """Load YAML configuration and expand channel_ranges."""
    with open(path) as f:
        raw = yaml.safe_load(f)

    cfg = GainMatcherConfig()

    # HV section
    hv = raw.get("hv", {})
    cfg.hv_host = hv.get("host", cfg.hv_host)
    cfg.hv_username = hv.get("username", cfg.hv_username)
    cfg.hv_password = hv.get("password", cfg.hv_password)

    # DAQ section
    daq = raw.get("daq", {})
    cfg.operator_url = daq.get("operator_url", cfg.operator_url)
    cfg.monitor_url = daq.get("monitor_url", cfg.monitor_url)

    # Matching section
    matching = raw.get("matching", {})
    cfg.measure_time = matching.get("measure_time", cfg.measure_time)
    cfg.max_iterations = matching.get("max_iterations", cfg.max_iterations)
    cfg.tolerance_percent = matching.get("tolerance_percent", cfg.tolerance_percent)
    cfg.pmt_alpha = matching.get("pmt_alpha", cfg.pmt_alpha)
    cfg.min_counts = matching.get("min_counts", cfg.min_counts)
    cfg.hv_step_limit = matching.get("hv_step_limit", cfg.hv_step_limit)
    cfg.settling_time = matching.get("settling_time", cfg.settling_time)

    # Defaults
    defaults = raw.get("defaults", {})
    default_region = tuple(defaults.get("peak_region", [300, 500]))
    default_target = defaults.get("target_position", 1000)

    # Skip channels set
    skip_set = set()
    for s in raw.get("skip_channels", []):
        skip_set.add((s["hv_slot"], s["hv_ch"]))

    # Explicit channels
    for ch in raw.get("channels", []):
        region = tuple(ch.get("peak_region", default_region))
        target = ch.get("target_position", default_target)
        key = (ch["hv_slot"], ch["hv_ch"])
        cfg.channels.append(ChannelConfig(
            name=ch.get("name", f"ch-{key[0]}-{key[1]}"),
            hv_slot=ch["hv_slot"],
            hv_ch=ch["hv_ch"],
            dig_module=ch["dig_module"],
            dig_ch=ch["dig_ch"],
            peak_region=region,
            target_position=target,
            skip=key in skip_set,
        ))

    # Expand channel_ranges
    _RANGE_REQUIRED_KEYS = [
        "name_prefix", "hv_slot", "hv_ch_start", "hv_ch_end",
        "dig_module", "dig_ch_start",
    ]
    for idx, rng in enumerate(raw.get("channel_ranges", [])):
        missing = [k for k in _RANGE_REQUIRED_KEYS if k not in rng]
        if missing:
            raise ValueError(
                f"channel_ranges[{idx}]: missing required keys: {missing}"
            )
        prefix = rng["name_prefix"]
        hv_slot = rng["hv_slot"]
        hv_start = rng["hv_ch_start"]
        hv_end = rng["hv_ch_end"]
        dig_mod = rng["dig_module"]
        dig_start = rng["dig_ch_start"]
        region = tuple(rng.get("peak_region", default_region))
        target = rng.get("target_position", default_target)

        for i, hv_ch in enumerate(range(hv_start, hv_end + 1)):
            dig_ch = dig_start + i
            key = (hv_slot, hv_ch)
            cfg.channels.append(ChannelConfig(
                name=f"{prefix}-{i:02d}",
                hv_slot=hv_slot,
                hv_ch=hv_ch,
                dig_module=dig_mod,
                dig_ch=dig_ch,
                peak_region=region,
                target_position=target,
                skip=key in skip_set,
            ))

    logger.info("Loaded %d channels (%d active)",
                len(cfg.channels),
                sum(1 for c in cfg.channels if not c.skip))
    return cfg
