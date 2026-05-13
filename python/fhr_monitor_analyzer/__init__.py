"""Python API for fhr-monitor-analyzer.

The analysis functions return JSON strings. Use ``json.loads(...)`` in the
caller when a Python dictionary is needed.
"""

from __future__ import annotations

from importlib import import_module
from pathlib import Path

from .plotting import plot_csv, plot_csv_file

__all__ = [
    "analyze_json",
    "analyze_csv",
    "analyze_json_file",
    "analyze_csv_file",
    "plot_csv",
    "plot_csv_file",
]


def analyze_json(request_json: str) -> str:
    """Analyze one JSON request string and return a JSON report string."""
    return _native_module().analyze_json_string(request_json)


def analyze_csv(
    csv_text: str,
    *,
    channel: str = "HR1",
    ga_weeks: int | None = None,
    window_min: int | None = None,
    step_sec: int = 60,
    last_only: bool = False,
) -> str:
    """Analyze CSV text and return a JSON report string."""
    return _native_module().analyze_csv_string(
        csv_text,
        channel=channel,
        ga_weeks=ga_weeks,
        window_min=window_min,
        step_sec=step_sec,
        last_only=last_only,
    )


def analyze_json_file(path: str | Path) -> str:
    """Read a JSON request file and return a JSON report string."""
    return analyze_json(Path(path).read_text())


def analyze_csv_file(
    path: str | Path,
    *,
    channel: str = "HR1",
    ga_weeks: int | None = None,
    window_min: int | None = None,
    step_sec: int = 60,
    last_only: bool = False,
) -> str:
    """Read a CSV file and return a JSON report string."""
    return analyze_csv(
        Path(path).read_text(),
        channel=channel,
        ga_weeks=ga_weeks,
        window_min=window_min,
        step_sec=step_sec,
        last_only=last_only,
    )


def _native_module():
    try:
        return import_module("fhr_monitor_analyzer._native")
    except ImportError as err:
        raise ImportError(
            "fhr-monitor-analyzer native extension is not installed. "
            "Run `maturin develop --features python` from the repository root "
            "for local development, or install the wheel with pip."
        ) from err
