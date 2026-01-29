"""REST API client for delila-rs Monitor and Operator."""

import logging
import time

import requests

logger = logging.getLogger(__name__)


class DAQError(Exception):
    """DAQ communication error."""
    pass


class DAQClient:
    """Client for delila-rs Operator and Monitor REST APIs."""

    def __init__(self, operator_url: str = "http://localhost:8080",
                 monitor_url: str = "http://localhost:8081",
                 timeout: float = 10.0):
        self._operator = operator_url.rstrip("/")
        self._monitor = monitor_url.rstrip("/")
        self._timeout = timeout

    def _get(self, url: str, retries: int = 3) -> dict:
        """GET with retry logic."""
        last_err = None
        for attempt in range(retries):
            try:
                r = requests.get(url, timeout=self._timeout)
                r.raise_for_status()
                return r.json()
            except requests.ConnectionError as e:
                last_err = e
                if attempt < retries - 1:
                    time.sleep(1.0)
            except requests.HTTPError as e:
                raise DAQError(f"HTTP {r.status_code}: {url}") from e
        raise DAQError(f"Cannot connect to {url}: {last_err}")

    def _post(self, url: str) -> dict | None:
        """POST request."""
        try:
            r = requests.post(url, timeout=self._timeout)
            r.raise_for_status()
            if r.content:
                return r.json()
            return None
        except requests.ConnectionError as e:
            raise DAQError(f"Cannot connect to {url}: {e}")
        except requests.HTTPError as e:
            raise DAQError(f"HTTP {r.status_code}: {url}") from e

    # --- Operator API ---

    def get_daq_status(self) -> dict:
        """GET /api/status — DAQ component states."""
        return self._get(f"{self._operator}/api/status")

    def is_running(self) -> bool:
        """Check if all DAQ components are in Running state."""
        status = self.get_daq_status()
        components = status.get("components", [])
        return all(c.get("state") == "Running" for c in components)

    # --- Monitor API ---

    def get_histogram(self, module: int, channel: int) -> dict:
        """GET /api/histograms/:module/:ch — histogram data.

        Returns dict with keys: module_id, channel_id, bins (list[int]),
        total_counts, config.
        """
        url = f"{self._monitor}/api/histograms/{module}/{channel}"
        return self._get(url)

    def get_all_histograms(self) -> dict:
        """GET /api/histograms — all channel summaries."""
        return self._get(f"{self._monitor}/api/histograms")

    def clear_histograms(self):
        """POST /api/histograms/clear — reset all histograms."""
        self._post(f"{self._monitor}/api/histograms/clear")
        logger.info("Histograms cleared")
