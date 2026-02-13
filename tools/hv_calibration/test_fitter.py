"""Unit tests for fitter.py — Gaussian peak fitting and auto-detection."""

import numpy as np
import pytest

from fitter import FitResult, fit_peak, find_peaks_auto, gaussian


def _make_histogram(peaks=None, n_bins=65536, background=5):
    """Generate a synthetic histogram with Gaussian peaks.

    Args:
        peaks: list of (center, sigma, amplitude) tuples.
        n_bins: total number of bins.
        background: constant background level.
    Returns:
        list[int] of bin counts.
    """
    x = np.arange(n_bins, dtype=np.float64)
    y = np.full(n_bins, background, dtype=np.float64)
    if peaks:
        for center, sigma, amplitude in peaks:
            y += amplitude * np.exp(-(x - center) ** 2 / (2 * sigma ** 2))
    return [int(v) for v in y]


# --- fit_peak tests ---


def test_gaussian_fit_known_peak():
    """Fit a clean Gaussian: center=400, sigma=30, amp=500."""
    bins = _make_histogram(peaks=[(400, 30, 500)])
    result = fit_peak(bins, region=(300, 500), min_counts=100)

    assert result.success
    assert abs(result.peak_position - 400) < 1.0
    assert abs(result.peak_sigma - 30) < 2.0
    assert result.peak_amplitude > 0
    assert result.chi_squared >= 0


def test_gaussian_fit_with_noise():
    """Fit a Gaussian with Poisson noise — should still converge."""
    rng = np.random.default_rng(42)
    x = np.arange(65536, dtype=np.float64)
    y = 500.0 * np.exp(-(x - 1000) ** 2 / (2 * 25 ** 2)) + 10.0
    noisy = rng.poisson(np.maximum(y, 0)).astype(int)
    bins = noisy.tolist()

    result = fit_peak(bins, region=(900, 1100), min_counts=100)

    assert result.success
    assert abs(result.peak_position - 1000) < 5.0


def test_insufficient_counts():
    """Low counts in region → failure with 'Insufficient' message."""
    bins = [1] * 65536  # total in any 200-bin region = 200
    result = fit_peak(bins, region=(300, 500), min_counts=1000)

    assert not result.success
    assert "Insufficient" in result.message


def test_flat_data_no_peak():
    """Constant data — fit should fail or produce nonsensical result."""
    bins = [100] * 65536
    result = fit_peak(bins, region=(300, 500), min_counts=100)

    # A flat region has no peak: either fit fails or sigma check catches it
    # We just verify it doesn't claim success with a meaningful peak
    if result.success:
        # If scipy somehow converges, sigma should be unreasonably large
        # or amplitude near zero
        assert result.peak_sigma > 20 or result.peak_amplitude < 10


def test_invalid_region_reversed():
    """Region with low > high → failure."""
    bins = _make_histogram(peaks=[(400, 30, 500)])
    result = fit_peak(bins, region=(500, 300), min_counts=100)

    assert not result.success
    assert "Invalid region" in result.message


def test_region_negative_start():
    """Region starting below 0 → failure."""
    bins = _make_histogram(peaks=[(50, 10, 300)])
    result = fit_peak(bins, region=(-1, 100), min_counts=100)

    assert not result.success


def test_region_beyond_bins():
    """Region extending past bin count → failure."""
    bins = _make_histogram(peaks=[(400, 30, 500)])
    result = fit_peak(bins, region=(0, 70000), min_counts=100)

    assert not result.success


def test_peak_near_region_edge():
    """Peak close to region boundary — still fittable."""
    bins = _make_histogram(peaks=[(310, 15, 800)])
    result = fit_peak(bins, region=(300, 500), min_counts=100)

    assert result.success
    assert abs(result.peak_position - 310) < 3.0


def test_fit_result_fields():
    """Verify all FitResult fields are populated on success."""
    bins = _make_histogram(peaks=[(600, 20, 1000)])
    result = fit_peak(bins, region=(550, 650), min_counts=100)

    assert result.success
    assert result.peak_position > 0
    assert result.peak_sigma > 0
    assert result.peak_amplitude > 0
    assert result.chi_squared >= 0
    assert result.message == "OK"


# --- find_peaks_auto tests ---


def test_find_single_peak():
    """Single peak histogram → one position returned."""
    bins = _make_histogram(peaks=[(500, 20, 300)])
    peaks = find_peaks_auto(bins, min_height=50, min_distance=30)

    assert len(peaks) >= 1
    assert abs(peaks[0] - 500) < 5


def test_find_multiple_peaks_sorted_by_height():
    """Two peaks: taller one should be first in results."""
    bins = _make_histogram(peaks=[(300, 20, 200), (700, 20, 500)])
    peaks = find_peaks_auto(bins, min_height=50, min_distance=30)

    assert len(peaks) >= 2
    # 700 peak (amp=500) should come before 300 peak (amp=200)
    assert abs(peaks[0] - 700) < 5
    assert abs(peaks[1] - 300) < 5


def test_no_peaks_flat():
    """Flat low data → empty peak list."""
    bins = [5] * 65536
    peaks = find_peaks_auto(bins, min_height=100)

    assert peaks == []


def test_skip_below():
    """Peak below skip_below threshold should be ignored."""
    bins = _make_histogram(peaks=[(5, 2, 500), (200, 20, 300)])
    # skip_below=50 should ignore the peak at ADC ch 5
    peaks = find_peaks_auto(bins, min_height=50, skip_below=50)

    # Only the peak at ~200 should be found
    for p in peaks:
        assert p >= 50


def test_gaussian_function():
    """Verify the gaussian helper function itself."""
    # At center, value should be amplitude + offset
    val = gaussian(100.0, 500.0, 100.0, 30.0, 10.0)
    assert abs(val - 510.0) < 0.01

    # Far from center, value should approach offset
    val_far = gaussian(1000.0, 500.0, 100.0, 30.0, 10.0)
    assert abs(val_far - 10.0) < 0.01
