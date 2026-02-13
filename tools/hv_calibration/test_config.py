"""Unit tests for config.py — YAML configuration loader."""

import os
import textwrap

import pytest
import yaml

from config import ChannelConfig, GainMatcherConfig, load_config

# Path to the example config shipped with the tool
EXAMPLE_CONFIG = os.path.join(os.path.dirname(__file__), "examples", "gain_config.yaml")


def _write_yaml(tmp_path, data: dict, filename: str = "test_config.yaml") -> str:
    """Write a dict as YAML to a temp file, return path."""
    path = tmp_path / filename
    path.write_text(yaml.dump(data, default_flow_style=False))
    return str(path)


# --- Basic loading ---


def test_load_example_config():
    """Load the shipped example config and verify key fields."""
    cfg = load_config(EXAMPLE_CONFIG)

    assert cfg.hv_host == "172.18.5.215"
    assert cfg.hv_username == "admin"
    assert cfg.hv_password == "eli-np"
    assert cfg.operator_url == "http://localhost:8080"
    assert cfg.monitor_url == "http://localhost:8081"
    assert cfg.measure_time == 10
    assert cfg.pmt_alpha == 7.0
    assert len(cfg.channels) == 2  # Two explicit channels in example


def test_load_example_channel_details():
    """Verify individual channel properties from example config."""
    cfg = load_config(EXAMPLE_CONFIG)

    ch0 = cfg.channels[0]
    assert ch0.name == "LaBr3-01"
    assert ch0.hv_slot == 0
    assert ch0.hv_ch == 0
    assert ch0.dig_module == 0
    assert ch0.dig_ch == 0
    assert ch0.peak_region == (300, 500)  # from defaults
    assert ch0.target_position == 1000    # from defaults

    ch1 = cfg.channels[1]
    assert ch1.name == "LaBr3-02"
    assert ch1.peak_region == (400, 600)  # per-channel override


# --- Defaults ---


def test_defaults_applied(tmp_path):
    """Defaults section values propagate to channels."""
    data = {
        "hv": {"host": "10.0.0.1"},
        "defaults": {"peak_region": [100, 200], "target_position": 2000},
        "channels": [
            {"name": "det-0", "hv_slot": 0, "hv_ch": 0,
             "dig_module": 0, "dig_ch": 0},
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.channels[0].peak_region == (100, 200)
    assert cfg.channels[0].target_position == 2000


def test_per_channel_override_beats_defaults(tmp_path):
    """Per-channel peak_region overrides the defaults section."""
    data = {
        "defaults": {"peak_region": [100, 200], "target_position": 500},
        "channels": [
            {"name": "det-0", "hv_slot": 0, "hv_ch": 0,
             "dig_module": 0, "dig_ch": 0,
             "peak_region": [800, 900], "target_position": 3000},
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.channels[0].peak_region == (800, 900)
    assert cfg.channels[0].target_position == 3000


# --- channel_ranges expansion ---


def test_channel_ranges_expansion(tmp_path):
    """channel_ranges generates sequential channels with correct names."""
    data = {
        "defaults": {"peak_region": [300, 500], "target_position": 1000},
        "channel_ranges": [
            {
                "name_prefix": "LaBr3",
                "hv_slot": 2,
                "hv_ch_start": 0,
                "hv_ch_end": 3,   # 4 channels: 0,1,2,3
                "dig_module": 1,
                "dig_ch_start": 4,
            },
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert len(cfg.channels) == 4
    assert cfg.channels[0].name == "LaBr3-00"
    assert cfg.channels[3].name == "LaBr3-03"
    assert cfg.channels[0].hv_slot == 2
    assert cfg.channels[0].hv_ch == 0
    assert cfg.channels[3].hv_ch == 3
    assert cfg.channels[0].dig_module == 1
    assert cfg.channels[0].dig_ch == 4
    assert cfg.channels[3].dig_ch == 7


def test_channel_ranges_with_override(tmp_path):
    """channel_ranges can override peak_region per range."""
    data = {
        "defaults": {"peak_region": [300, 500]},
        "channel_ranges": [
            {
                "name_prefix": "NaI",
                "hv_slot": 0,
                "hv_ch_start": 0,
                "hv_ch_end": 1,
                "dig_module": 0,
                "dig_ch_start": 0,
                "peak_region": [600, 800],
            },
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.channels[0].peak_region == (600, 800)
    assert cfg.channels[1].peak_region == (600, 800)


def test_mixed_channels_and_ranges(tmp_path):
    """Explicit channels + channel_ranges combine correctly."""
    data = {
        "defaults": {"peak_region": [300, 500], "target_position": 1000},
        "channels": [
            {"name": "special-0", "hv_slot": 5, "hv_ch": 10,
             "dig_module": 3, "dig_ch": 7},
        ],
        "channel_ranges": [
            {
                "name_prefix": "det",
                "hv_slot": 0,
                "hv_ch_start": 0,
                "hv_ch_end": 2,
                "dig_module": 0,
                "dig_ch_start": 0,
            },
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    # 1 explicit + 3 from range
    assert len(cfg.channels) == 4
    assert cfg.channels[0].name == "special-0"
    assert cfg.channels[1].name == "det-00"


# --- skip_channels ---


def test_skip_channels(tmp_path):
    """Channels in skip_channels list get skip=True."""
    data = {
        "defaults": {"peak_region": [300, 500]},
        "channel_ranges": [
            {
                "name_prefix": "det",
                "hv_slot": 0,
                "hv_ch_start": 0,
                "hv_ch_end": 3,
                "dig_module": 0,
                "dig_ch_start": 0,
            },
        ],
        "skip_channels": [
            {"hv_slot": 0, "hv_ch": 1},
            {"hv_slot": 0, "hv_ch": 3},
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert not cfg.channels[0].skip  # hv_ch=0
    assert cfg.channels[1].skip      # hv_ch=1 (skipped)
    assert not cfg.channels[2].skip  # hv_ch=2
    assert cfg.channels[3].skip      # hv_ch=3 (skipped)


def test_skip_applies_to_explicit_channels(tmp_path):
    """skip_channels also applies to explicitly listed channels."""
    data = {
        "channels": [
            {"name": "det-0", "hv_slot": 0, "hv_ch": 5,
             "dig_module": 0, "dig_ch": 0},
        ],
        "skip_channels": [
            {"hv_slot": 0, "hv_ch": 5},
        ],
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.channels[0].skip


# --- Edge cases ---


def test_empty_channels(tmp_path):
    """Config with no channels or channel_ranges → empty list."""
    data = {
        "hv": {"host": "10.0.0.1", "username": "user", "password": "pass"},
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.channels == []
    assert cfg.hv_host == "10.0.0.1"


def test_matching_params_override(tmp_path):
    """Custom matching parameters override defaults."""
    data = {
        "matching": {
            "measure_time": 30,
            "max_iterations": 5,
            "tolerance_percent": 1.0,
            "pmt_alpha": 8.5,
            "min_counts": 500,
            "hv_step_limit": 25.0,
        },
    }
    cfg = load_config(_write_yaml(tmp_path, data))

    assert cfg.measure_time == 30
    assert cfg.max_iterations == 5
    assert cfg.tolerance_percent == 1.0
    assert cfg.pmt_alpha == 8.5
    assert cfg.min_counts == 500
    assert cfg.hv_step_limit == 25.0


# --- Fix #4: settling_time ---


def test_settling_time_default(tmp_path):
    """Default settling_time is 5.0s."""
    data = {"hv": {"host": "10.0.0.1"}}
    cfg = load_config(_write_yaml(tmp_path, data))
    assert cfg.settling_time == 5.0


def test_settling_time_override(tmp_path):
    """settling_time can be overridden in matching section."""
    data = {"matching": {"settling_time": 15.0}}
    cfg = load_config(_write_yaml(tmp_path, data))
    assert cfg.settling_time == 15.0


# --- Fix #6: channel_ranges missing keys ---


def test_channel_ranges_missing_key_raises(tmp_path):
    """channel_ranges with missing required key raises ValueError."""
    data = {
        "channel_ranges": [
            {
                "name_prefix": "det",
                # missing hv_slot, hv_ch_start, etc.
            },
        ],
    }
    with pytest.raises(ValueError, match="missing required keys"):
        load_config(_write_yaml(tmp_path, data))
