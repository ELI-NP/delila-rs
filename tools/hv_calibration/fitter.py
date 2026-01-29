"""Gaussian peak fitting and auto-detection for PMT gain matching."""

import logging
from dataclasses import dataclass

import numpy as np
from scipy.optimize import curve_fit
from scipy.signal import find_peaks

logger = logging.getLogger(__name__)


@dataclass
class FitResult:
    """Result of a Gaussian fit."""
    peak_position: float = 0.0
    peak_sigma: float = 0.0
    peak_amplitude: float = 0.0
    chi_squared: float = 0.0
    success: bool = False
    message: str = ""


def gaussian(x, amplitude, center, sigma, offset):
    """Gaussian + constant background."""
    return amplitude * np.exp(-(x - center) ** 2 / (2 * sigma ** 2)) + offset


def fit_peak(bins: list[int], region: tuple[int, int],
             min_counts: int = 1000) -> FitResult:
    """Fit a Gaussian peak within the specified ADC channel region.

    Args:
        bins: Full histogram data (up to 65536 bins).
        region: (low, high) ADC channel range to fit.
        min_counts: Minimum total counts required in region.

    Returns:
        FitResult with peak parameters or failure info.
    """
    low, high = region
    if low < 0 or high > len(bins) or low >= high:
        return FitResult(message=f"Invalid region [{low}, {high})")

    y = np.array(bins[low:high], dtype=np.float64)
    x = np.arange(low, high, dtype=np.float64)

    total = np.sum(y)
    if total < min_counts:
        return FitResult(
            message=f"Insufficient counts: {total:.0f} < {min_counts}"
        )

    # Initial value estimation
    i_max = np.argmax(y)
    center0 = x[i_max]
    amplitude0 = y[i_max]
    offset0 = (y[0] + y[-1]) / 2.0

    # FWHM estimation for sigma
    half_max = (amplitude0 - offset0) / 2.0 + offset0
    above_half = np.where(y > half_max)[0]
    if len(above_half) > 1:
        fwhm = x[above_half[-1]] - x[above_half[0]]
        sigma0 = max(fwhm / 2.35, 1.0)
    else:
        sigma0 = (high - low) / 6.0

    p0 = [amplitude0, center0, sigma0, offset0]
    bounds = (
        [0, low, 0.5, 0],                          # lower
        [amplitude0 * 10, high, (high - low), np.inf],  # upper
    )

    try:
        popt, pcov = curve_fit(gaussian, x, y, p0=p0, bounds=bounds,
                               maxfev=5000)
    except (RuntimeError, ValueError) as e:
        return FitResult(message=f"Fit failed: {e}")

    amplitude, center, sigma, offset = popt

    # Chi-squared
    y_fit = gaussian(x, *popt)
    residuals = y - y_fit
    # Avoid division by zero for bins with 0 counts
    y_err = np.where(y > 0, np.sqrt(y), 1.0)
    chi2 = np.sum((residuals / y_err) ** 2)
    ndf = len(y) - len(popt)
    chi2_ndf = chi2 / ndf if ndf > 0 else chi2

    # Sanity checks
    if center < low or center > high:
        return FitResult(message=f"Fit center {center:.1f} outside region")
    if sigma < 0.5:
        return FitResult(message=f"Fit sigma too small: {sigma:.2f}")

    return FitResult(
        peak_position=center,
        peak_sigma=sigma,
        peak_amplitude=amplitude,
        chi_squared=chi2_ndf,
        success=True,
        message="OK",
    )


def find_peaks_auto(bins: list[int], min_height: int = 100,
                    min_distance: int = 50,
                    skip_below: int = 10) -> list[int]:
    """Auto-detect peaks in histogram (for scan mode).

    Args:
        bins: Full histogram data.
        min_height: Minimum peak height.
        min_distance: Minimum distance between peaks (ADC channels).
        skip_below: Skip bins below this index (noise region).

    Returns:
        List of peak positions (ADC channel indices), sorted by height descending.
    """
    y = np.array(bins[skip_below:], dtype=np.float64)
    peaks, properties = find_peaks(y, height=min_height,
                                   distance=min_distance,
                                   prominence=min_height * 0.3)
    # Offset back by skip_below
    peak_positions = peaks + skip_below

    # Sort by height (descending)
    heights = [bins[p] for p in peak_positions]
    sorted_peaks = [p for _, p in sorted(zip(heights, peak_positions),
                                         reverse=True)]
    return sorted_peaks
