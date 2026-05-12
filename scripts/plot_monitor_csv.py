#!/usr/bin/env python3
"""Plot fetal monitor CSV exports for quick visual review.

The Rust CLI is the source of truth for analysis. This script is intentionally
diagnostic: it helps inspect raw monitor chunks, signal loss, maternal HR
overlap, and TOCO activity before tuning the analysis rules.
"""

from __future__ import annotations

import argparse
import csv
import math
import os
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path
from statistics import mean
from tempfile import gettempdir

cache_root = Path(gettempdir()) / "fhr_monitor_plot_cache"
cache_root.mkdir(parents=True, exist_ok=True)
os.environ.setdefault("XDG_CACHE_HOME", str(cache_root))
os.environ.setdefault("MPLCONFIGDIR", str(cache_root / "matplotlib"))

import matplotlib.dates as mdates
import matplotlib.pyplot as plt


FETAL_CHANNELS = ("HR1", "HR2", "HR3")


@dataclass
class Sample:
    timestamp: datetime
    hr1: float | None
    hr2: float | None
    hr3: float | None
    hrm: float | None
    toco: float | None

    def fetal_value(self, channel: str) -> float | None:
        return {
            "HR1": self.hr1,
            "HR2": self.hr2,
            "HR3": self.hr3,
        }[channel]


def main() -> None:
    args = parse_args()
    samples = read_samples(args.csv_path)
    if args.max_minutes is not None:
        samples = crop_latest(samples, args.max_minutes)
    if not samples:
        raise SystemExit("no samples available after filtering")

    output = args.output or default_output_path(args.csv_path)
    plot_samples(
        samples=samples,
        selected_channel=args.channel,
        output=output,
        show_all_fetal=args.all_fetal,
        title=args.title,
        dpi=args.dpi,
    )
    print(f"wrote {output}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Visualize fetal monitor CSV data as fetal HR, maternal HR, and TOCO graphs."
    )
    parser.add_argument("csv_path", type=Path, help="CSV file with Date, HR1, HR2, HR3, HRM, TOCO columns.")
    parser.add_argument(
        "--channel",
        choices=FETAL_CHANNELS,
        default="HR1",
        help="Primary fetal channel to highlight. Default: HR1.",
    )
    parser.add_argument(
        "--all-fetal",
        action="store_true",
        help="Also plot nonselected fetal channels when present.",
    )
    parser.add_argument(
        "--max-minutes",
        type=float,
        default=None,
        help="Plot only the latest N minutes from the CSV.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="PNG output path. Default: ./<csv-name>_monitor_plot.png.",
    )
    parser.add_argument("--title", default=None, help="Optional figure title.")
    parser.add_argument("--dpi", type=int, default=150, help="PNG resolution. Default: 150.")
    return parser.parse_args()


def read_samples(path: Path) -> list[Sample]:
    with path.open(newline="") as f:
        reader = csv.DictReader(f)
        samples = [
            Sample(
                timestamp=parse_timestamp(row["Date"]),
                hr1=parse_hr(row.get("HR1")),
                hr2=parse_hr(row.get("HR2")),
                hr3=parse_hr(row.get("HR3")),
                hrm=parse_hr(row.get("HRM")),
                toco=parse_number(row.get("TOCO"), zero_is_missing=False),
            )
            for row in reader
            if row.get("Date")
        ]
    return sorted(samples, key=lambda sample: sample.timestamp)


def parse_timestamp(value: str) -> datetime:
    value = value.strip()
    if value.endswith("Z"):
        value = value[:-1] + "+00:00"
    try:
        return datetime.fromisoformat(value)
    except ValueError:
        return datetime.strptime(value, "%Y-%m-%d %H:%M:%S.%f")


def parse_hr(value: str | None) -> float | None:
    return parse_number(value, zero_is_missing=True)


def parse_number(value: str | None, *, zero_is_missing: bool) -> float | None:
    if value is None or value.strip() == "":
        return None
    parsed = float(value)
    if zero_is_missing and parsed == 0:
        return None
    return parsed


def crop_latest(samples: list[Sample], minutes: float) -> list[Sample]:
    cutoff = samples[-1].timestamp - timedelta(minutes=minutes)
    return [sample for sample in samples if sample.timestamp >= cutoff]


def default_output_path(csv_path: Path) -> Path:
    return Path.cwd() / f"{csv_path.stem}_monitor_plot.png"


def plot_samples(
    *,
    samples: list[Sample],
    selected_channel: str,
    output: Path,
    show_all_fetal: bool,
    title: str | None,
    dpi: int,
) -> None:
    times = [sample.timestamp for sample in samples]
    selected_values = [sample.fetal_value(selected_channel) for sample in samples]
    hrm_values = [sample.hrm for sample in samples]
    toco_values = [sample.toco for sample in samples]

    fig, axes = plt.subplots(
        nrows=3,
        ncols=1,
        figsize=(15, 9),
        sharex=True,
        gridspec_kw={"height_ratios": [2.3, 1.2, 1.4]},
        constrained_layout=True,
    )
    ax_fhr, ax_mhr, ax_toco = axes

    ax_fhr.axhspan(110, 160, color="#d9f2df", alpha=0.6, label="110-160 bpm")
    ax_fhr.axhline(110, color="#3c8d4a", linewidth=0.8)
    ax_fhr.axhline(160, color="#3c8d4a", linewidth=0.8)
    ax_fhr.plot(times, selected_values, color="#1f5fbf", linewidth=1.2, label=selected_channel)
    if show_all_fetal:
        for channel, color in [("HR1", "#7aa6ff"), ("HR2", "#9966cc"), ("HR3", "#d8892b")]:
            if channel == selected_channel:
                continue
            values = [sample.fetal_value(channel) for sample in samples]
            if any(value is not None for value in values):
                ax_fhr.plot(times, values, color=color, linewidth=0.8, alpha=0.65, label=channel)
    ax_fhr.set_ylabel("Fetal HR (bpm)")
    ax_fhr.set_ylim(50, 210)
    ax_fhr.grid(True, alpha=0.25)
    ax_fhr.legend(loc="upper right")

    ax_mhr.plot(times, hrm_values, color="#b23a48", linewidth=1.0, label="HRM")
    ax_mhr.set_ylabel("Maternal HR")
    ax_mhr.set_ylim(40, 210)
    ax_mhr.grid(True, alpha=0.25)
    ax_mhr.legend(loc="upper right")

    ax_toco.fill_between(
        times,
        [0 if value is None else value for value in toco_values],
        step="pre",
        color="#2f7d68",
        alpha=0.35,
    )
    ax_toco.plot(times, toco_values, color="#1b5f4d", linewidth=0.8, label="TOCO")
    ax_toco.set_ylabel("TOCO")
    ax_toco.set_xlabel("Time")
    ax_toco.grid(True, alpha=0.25)
    ax_toco.legend(loc="upper right")

    ax_toco.xaxis.set_major_formatter(mdates.DateFormatter("%H:%M:%S"))
    for label in ax_toco.get_xticklabels():
        label.set_rotation(30)
        label.set_horizontalalignment("right")

    fig.suptitle(title or build_title(samples, selected_channel, selected_values), fontsize=13)
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output, dpi=dpi)
    plt.close(fig)


def build_title(samples: list[Sample], channel: str, values: list[float | None]) -> str:
    duration_min = (samples[-1].timestamp - samples[0].timestamp).total_seconds() / 60
    usable = [value for value in values if value is not None and not math.isnan(value)]
    if usable:
        summary = f"{min(usable):.0f}/{mean(usable):.0f}/{max(usable):.0f} bpm min/mean/max"
        usable_pct = len(usable) / len(values) * 100
    else:
        summary = "no usable fetal HR"
        usable_pct = 0.0
    return f"{channel} monitor view | {duration_min:.1f} min | usable {usable_pct:.1f}% | {summary}"


if __name__ == "__main__":
    main()
