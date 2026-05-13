#!/usr/bin/env python3
"""Plot fetal monitor CSV exports for quick visual review.

The Rust CLI is the source of truth for analysis. This script is intentionally
diagnostic: it helps inspect raw monitor chunks, signal loss, maternal HR
overlap, and TOCO activity before tuning the analysis rules.
"""

import argparse
from pathlib import Path
import sys

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "python"))

from fhr_monitor_analyzer.plotting import FETAL_CHANNELS, plot_csv_file


def main() -> None:
    args = parse_args()
    output = plot_csv_file(
        args.csv_path,
        output=args.output,
        channel=args.channel,
        all_fetal=args.all_fetal,
        max_minutes=args.max_minutes,
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


if __name__ == "__main__":
    main()
