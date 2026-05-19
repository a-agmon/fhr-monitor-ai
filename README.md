# fhr-monitor-analyzer

Decision-support prototype for fetal heart-rate monitoring. The current implementation focuses on explainable feature extraction and alert triage for fetal heart tracing chunks, with Category II detection as the first target.

This is not an autonomous clinical decision system. It is intended to return structured metadata, detected tracing features, data-quality limitations, and alerts for clinician review.

## Current Scope

The Rust core analyzes monitor data containing:

- `HR1`, `HR2`, `HR3`: fetal heart-rate channels
- `HRM`: maternal heart rate
- `TOCO`: uterine activity
- `Date`: sample timestamp

The analyzer currently performs:

- Timestamp sorting and duplicate/out-of-order metadata
- Heart-rate signal cleanup, treating `0` as missing signal
- Resampling to one-second buckets
- Baseline estimation from the latest 10-minute segment
- Variability classification
- Acceleration and deceleration event detection
- TOCO peak/contraction detection
- Basic Category I/II/III classification
- Higher-risk Category II feature flags
- Data-quality alerting when the tracing cannot be interpreted safely

## What This Prototype Is Testing

This project is trying to answer a practical question: can a small, deterministic rules engine turn pushed monitor chunks into useful fetal-rate status metadata without creating another noisy alarm stream?

The current implementation is testing several assumptions:

- Whether recent chunks can be analyzed without the caller declaring an exact window length.
- Whether data quality should be a first-class output instead of silently producing a misleading classification.
- Whether Category II alerts can be split into low-interruption context versus higher-risk warning signals.
- Whether numeric features such as fetal HR distribution, time below/above normal range, deceleration burden, and contraction frequency are useful to downstream systems even when no alert fires.
- Whether a deterministic, explainable Rust core is a good foundation before adding any ML or clinician-labeled tuning.

## How It Works

The pipeline is intentionally staged so each output can be traced back to simple intermediate measurements:

1. Parse the monitor chunk and sort by timestamp.
2. Treat zero heart-rate values as missing signal and preserve signal-loss metadata.
3. Resample the raw monitor feed into one-second buckets so irregular device cadence does not dominate the logic.
4. Analyze the latest available span, capped at 30 minutes for current-state interpretation.
5. Estimate baseline and variability from the most recent 10-minute segment when enough usable fetal HR exists.
6. Detect accelerations, decelerations, and TOCO-derived contractions over the recent context window.
7. Classify the tracing when possible, but return `data_quality` instead of forcing a category when the signal is insufficient.
8. Emit numeric features, reasons, high-risk features, protective features, and limitations for downstream systems.

## Chunk-Based Input

The intended service behavior is chunk-based. The device or upstream system sends the recent data it has; it does not need to tell the analyzer whether the chunk is exactly 20, 22, or 30 minutes.

The canonical request format is documented in [docs/data_contract.md](docs/data_contract.md), with a machine-readable schema in [docs/request.schema.json](docs/request.schema.json). In short, a request is a JSON object with `episode_id`, `sent_at`, and a `samples` array. Each sample has a timestamp `t` and any available monitor channels: `hr1`, `hr2`, `hr3`, `hrm`, and `toco`.

Minimal request:

```json
{
  "episode_id": "18664805",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "samples": [
    {
      "t": "2026-05-12T11:52:35.052Z",
      "hr1": 129,
      "hrm": 101,
      "toco": 33
    }
  ]
}
```

Important request rules:

- Do not send `window_minutes` or `chunk_minutes` for normal operation.
- The service infers the chunk duration from sample timestamps.
- `null` or omitted HR fields mean missing signal.
- `0` in heart-rate channels is treated as missing signal for compatibility with current device exports.
- `0` in `toco` is valid.
- Samples do not need to be pre-sorted; the service sorts them and reports ordering/duplicate metadata.

The analyzer infers the span from timestamps and returns:

- The actual input start/end/duration
- The analyzed window start/end/duration
- Data quality metrics
- Numeric fetal-rate and TOCO features for downstream systems
- Features found
- Classification, when possible
- Alert level
- Reasons, high-risk features, protective features, and limitations

Recommended sending pattern:

- Push every 60 seconds
- Prefer the latest 30 minutes
- Accept variable chunks from 10 to 30 minutes
- If more than 30 minutes is sent, analyze the latest 30 minutes for current status and keep the larger input span as metadata

Clinical completeness by chunk length:

- Less than 10 minutes: insufficient for baseline/variability
- 10-19 minutes: baseline/variability possible, recurrent deceleration assessment incomplete
- 20-29 minutes: recurrent deceleration screening possible, tachysystole assessment incomplete
- 30 minutes or more: full current-state window for this prototype

## CLI Usage

Build and run checks:

```bash
cargo check
cargo test
```

Analyze whatever chunk is present in a CSV:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1
```

Return JSON:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --json
```

Replay a file with fixed rolling windows:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --window-min 30 --step-sec 60
```

### CLI Flags

The CLI currently accepts CSV monitor exports and is mainly for local replay/testing of the core logic. The future service should use the JSON request contract documented above.

```bash
fhr-monitor-analyzer-cli <csv-path> [--channel HR1|HR2|HR3] [--window-min 10..30] [--step-sec N] [--last-only] [--json] [--ga-weeks N]
```

| Argument or flag | Required | Default | Meaning |
| --- | --- | --- | --- |
| `<csv-path>` | Yes | none | Path to a CSV export with `Date`, `HR1`, `HR2`, `HR3`, `HRM`, and `TOCO` columns. Extra columns are ignored by the current parser. |
| `--channel HR1|HR2|HR3` | No | `HR1` | Selects which fetal heart-rate channel to analyze. Use this when the active fetal trace is on `HR2` or `HR3`. |
| `--window-min 10..30` | No | omitted | Enables fixed rolling-window replay. Without this flag, the CLI analyzes the available chunk and infers its duration, capped at the latest 30 minutes. |
| `--step-sec N` | No | `60` | Step size between rolling windows when `--window-min` is supplied. For example, `--window-min 30 --step-sec 60` simulates one analysis every minute. |
| `--last-only` | No | false | Prints only the final analyzed window. Useful when testing a long CSV but only caring about the current-state result. |
| `--json` | No | false | Emits JSON instead of the human-readable text report. Use this for downstream integration tests. |
| `--ga-weeks N` | No | omitted | Gestational age in completed weeks. Values below 32 use the preterm acceleration threshold: 10 bpm for 10 seconds. Omitted or 32+ uses 15 bpm for 15 seconds. |
| `-h`, `--help` | No | false | Prints CLI usage. |

Default chunk mode:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --json
```

Rolling replay mode:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --window-min 20 --step-sec 60
```

Current-state only from a long CSV:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --last-only
```

## Python Package

The Python package is named `fhr-monitor-analyzer` and imports as `fhr_monitor_analyzer`. The Python analysis APIs return JSON strings, matching the service/CLI output contract.

Local development install:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip maturin pytest matplotlib notebook ipykernel
maturin develop --features python
python -c "import sys, fhr_monitor_analyzer; print(sys.executable); print(fhr_monitor_analyzer.__file__)"
```

`maturin develop --features python` installs the Rust extension into the active virtualenv. Use it for notebook and local development. If you specifically want wheel artifacts, run `maturin build --features python --out dist` and then `python -m pip install --force-reinstall dist/*.whl`.

Notebook kernel setup:

```bash
source .venv/bin/activate
python -c "import sys, fhr_monitor_analyzer; print(sys.executable); print(fhr_monitor_analyzer.__file__)"
python -m ipykernel install --user --name fhr-monitor-analyzer --display-name "Python (fhr-monitor-analyzer)"
python -m jupyter notebook examples/fhr_monitor_analyzer_demo.ipynb
```

In Jupyter, select the `Python (fhr-monitor-analyzer)` kernel, then run the notebook from the first cell. If `import fhr_monitor_analyzer` fails inside the notebook, run `import sys; print(sys.executable)` in a notebook cell and confirm it points at this repo's `.venv`. If the path points to a different Python installation, change the notebook kernel to `Python (fhr-monitor-analyzer)` or reinstall the kernel while the repo virtualenv is active.

Use a module alias that will not collide with data variables:

```python
import fhr_monitor_analyzer as analyzer

report_json = analyzer.analyze_csv_file(csv_path, channel="HR1")
```

Avoid reusing that alias for numeric fetal heart-rate values in the notebook. For example, call sample values `fetal_hr`, not `analyzer` or `fhr`.

Python usage:

```python
import json
import fhr_monitor_analyzer as analyzer

report_json = analyzer.analyze_csv_file("/path/to/monitor.csv", channel="HR1")
report = json.loads(report_json)

request_json = """{"episode_id":"e1","sent_at":"2026-05-12T12:22:35.052Z","samples":[{"t":"2026-05-12T11:52:35.052Z","hr1":129,"hrm":101,"toco":33}]}"""
report_json = analyzer.analyze_json(request_json)
```

Python entry points:

| Function | Input | Output |
| --- | --- | --- |
| `analyze_json(request_json)` | JSON request string following [docs/data_contract.md](docs/data_contract.md). | JSON report string. |
| `analyze_csv(csv_text, ...)` | CSV text plus optional `channel`, `ga_weeks`, `window_min`, `step_sec`, and `last_only`. | JSON report string. |
| `analyze_json_file(path)` | JSON request file. | JSON report string. |
| `analyze_csv_file(path, ...)` | CSV monitor export file. | JSON report string. |

Plotting is also available from the Python package:

```python
import fhr_monitor_analyzer as analyzer

analyzer.plot_csv_file("/path/to/monitor.csv", output="monitor_plot.png", channel="HR1")
```

A runnable notebook demo is available at [examples/fhr_monitor_analyzer_demo.ipynb](examples/fhr_monitor_analyzer_demo.ipynb). It creates demo monitor data, reads it through the Python library, displays the JSON report, and renders the tracing diagram.

For plotting-only installs, include the optional extra after release:

```bash
pip install "fhr-monitor-analyzer[plot]"
```

## Visualization Script

The repository includes a Python plotting helper for quick visual review of monitor exports. The same plotting code is available as `fhr_monitor_analyzer.plot_csv` and `fhr_monitor_analyzer.plot_csv_file`.

```bash
python3 scripts/plot_monitor_csv.py /path/to/monitor.csv --channel HR1 --output monitor_plot.png
```

It requires Python 3 and `matplotlib`; optional plotting dependencies are listed in [requirements-plot.txt](requirements-plot.txt) and in the package extra `fhr-monitor-analyzer[plot]`.

Useful options:

| Flag | Default | Meaning |
| --- | --- | --- |
| `--channel HR1|HR2|HR3` | `HR1` | Primary fetal HR channel to highlight. |
| `--all-fetal` | false | Also plots other fetal channels when present. |
| `--max-minutes N` | omitted | Crops the graph to the latest `N` minutes. |
| `--output PATH` | `./<csv-name>_monitor_plot.png` | PNG destination. |
| `--title TEXT` | auto-generated | Overrides the figure title. |
| `--dpi N` | `150` | Output image resolution. |

The graph has three panels: fetal HR with the 110-160 bpm normal baseline band, maternal HR, and TOCO. This is for visual debugging and review; the Rust analyzer remains the source of truth for classification and alert metadata.

## Example Response Fields

```json
{
  "analysis_mode": "chunk",
  "requested_window_minutes": null,
  "input": {
    "rows": 5980,
    "start_timestamp": "2026-05-12 11:12:34.972",
    "end_timestamp": "2026-05-12 11:45:15.220",
    "duration_seconds": 1960.248
  },
  "windows": [
    {
      "duration_seconds": 1800.0,
      "classification": "unclassified",
      "alert_level": "data_quality",
      "baseline_bpm": null,
      "features": {
        "fetal_hr_mean_bpm": 147.0,
        "fetal_hr_p05_bpm": 138.0,
        "fetal_hr_p95_bpm": 156.0,
        "fetal_hr_percent_below_110": 0.0,
        "fetal_hr_percent_above_160": 0.2,
        "acceleration_count": 0,
        "deceleration_count": 0,
        "contraction_count": 1,
        "contractions_per_10_min": 0.333
      },
      "reasons": ["cannot classify without a determinate baseline"],
      "limitations": ["baseline indeterminate: fewer than 120 usable fetal-HR seconds in the current 10-minute segment"]
    }
  ]
}
```

## Alert Philosophy

The goal is to reduce alert fatigue. The analyzer should not fire the same kind of interruptive alert for every Category II feature. It should separate:

- `none`: no interruptive alert
- `warning`: concerning Category II pattern that warrants review
- `urgent_review`: high-risk Category II pattern that should stand out above routine warnings
- `critical`: possible Category III
- `data_quality`: the tracing cannot be interpreted reliably

Even when no alert fires, the service should return all detected features and limitations so the UI can show useful context.

The user-facing explanation of categories and alert rules is in [docs/alerting_strategy.md](docs/alerting_strategy.md).

## Numeric Features

Each analysis window includes a `features` object intended for other systems that need fetal-rate status without parsing alert text. It includes distribution metrics such as min, p05, mean, median, p95, max, and standard deviation; seconds and percentages below 110 bpm, within 110-160 bpm, and above 160 bpm; acceleration/deceleration counts and duration; contraction counts; TOCO summary values; and maternal-vs-fetal mean HR difference when maternal HR is available.

## FHR Categories And Metric Values

The project follows the ACOG/NICHD three-tier terminology for fetal heart-rate tracing interpretation. These categories describe the current tracing; they are not a diagnosis and can change as the tracing evolves.

| Category | Meaning in this project | Core criteria used by the prototype |
| --- | --- | --- |
| `category_i` | Reassuring/normal tracing features. | Baseline 110-160 bpm, moderate variability, and no detected late or variable decelerations. Early decelerations and accelerations may be present or absent. |
| `category_ii` | Indeterminate tracing. This is intentionally broad. | Anything that is not Category I or Category III when enough signal exists to classify. Examples include tachycardia, bradycardia without absent variability, minimal variability, marked variability, prolonged deceleration, or recurrent late/variable decelerations without Category III criteria. |
| `category_iii` | Abnormal tracing features where abnormal fetal acid-base status cannot be excluded. | Absent variability with recurrent late decelerations, recurrent variable decelerations, or bradycardia; or a detected sinusoidal pattern persisting for at least 20 minutes. |
| `unclassified` | The engine refuses to force a category. | Usually caused by too little usable fetal HR in the current 10-minute baseline segment or other signal-quality limitations. |

Baseline metrics:

| Metric | Value assigned |
| --- | --- |
| `baseline_bpm` | Mean FHR over the current 10-minute segment, rounded to the nearest 5 bpm after excluding obvious artifacts and large excursions. Requires at least 120 usable fetal-HR seconds in the segment. |
| `baseline_class = bradycardia` | Baseline below 110 bpm. |
| `baseline_class = normal` | Baseline 110-160 bpm. |
| `baseline_class = tachycardia` | Baseline above 160 bpm. |

Variability metrics:

| Metric | Value assigned |
| --- | --- |
| `variability_bpm` | Prototype numeric estimate of baseline variability from one-second buckets in the current 10-minute segment. It is intended to approximate clinical peak-to-trough variability, not replace clinician visual interpretation. |
| `variability_class = absent` | Amplitude effectively undetectable, currently represented as <= 1 bpm. |
| `variability_class = minimal` | Detectable variability but <= 5 bpm. |
| `variability_class = moderate` | 6-25 bpm. This is the normal/reassuring variability range. |
| `variability_class = marked` | Greater than 25 bpm. |

Event metrics:

| Metric | Value assigned |
| --- | --- |
| `acceleration_count` | Count of detected abrupt increases lasting less than 2 minutes. At 32 weeks or later, or when gestational age is unknown, the threshold is at least 15 bpm above baseline for at least 15 seconds. Before 32 weeks, the threshold is at least 10 bpm above baseline for at least 10 seconds. |
| `deceleration_count` | Count of detected decreases at least 15 bpm below baseline lasting at least 15 seconds. |
| `prolonged_deceleration_count` | Count of decelerations lasting at least 2 minutes and less than 10 minutes. |
| `total_deceleration_seconds` | Sum of detected deceleration durations in the analysis window. |
| `deepest_deceleration_nadir_bpm` | Lowest detected nadir among deceleration events. |
| `max_deceleration_depth_bpm` | Largest baseline-to-nadir drop among deceleration events. |
| `contraction_count` | Count of TOCO-derived contraction-like peaks in the analysis window. This needs tuning against more labeled monitor exports. |
| `contractions_per_10_min` | Contraction count normalized to a 10-minute rate. |
| `tachysystole` | `true` only when a 30-minute contraction view is available and the detector finds more than 15 contractions in that 30-minute span, equivalent to more than 5 contractions per 10 minutes on average. Shorter chunks return `null` because the check is incomplete. |

High-risk Category II features currently surfaced separately from the category include absent variability, persistent minimal variability, marked variability, change from normal baseline to tachycardia, recurrent late decelerations, recurrent variable decelerations, gradual deceleration with absent or minimal variability, severe variable deceleration, deep deceleration nadir below 80 bpm, high deceleration burden, tachysystole, more than one prolonged deceleration, and possible maternal HR capture.

References for the terminology include ACOG's [fetal tracing summary](https://www.acog.org/community/districts-and-sections/district-iv/whats-new/countdown-to-intern-year-week-4-fetal-heart-tracings) and [Clinical Practice Guideline No. 10](https://www.acog.org/clinical/clinical-guidance/clinical-practice-guideline/articles/2025/10/intrapartum-fetal-heart-rate-monitoring-interpretation-and-management) on intrapartum fetal heart-rate monitoring.

## Repository Layout

```text
src/fhr_core/      Reusable analysis module
src/main.rs        CLI wrapper
docs/              Data contract and design notes
python/            Python package wrapper and plotting API
examples/          Notebook and runnable examples
scripts/           Local plotting and inspection helpers
.github/workflows/ CI and Python publishing workflows
```

Detailed architecture notes are in [docs/architecture.md](docs/architecture.md). The plan for wrapping the Rust core as a Python package is in [docs/python_library_plan.md](docs/python_library_plan.md).

## Testing And Publishing

Run Rust checks:

```bash
cargo fmt --check
cargo test
```

Run Python checks locally:

```bash
python -m pip install --upgrade pip maturin pytest matplotlib
maturin build --features python --out dist
python -m pip install --force-reinstall dist/*.whl
pytest tests/python
```

Build local Python artifacts:

```bash
maturin build --release --features python --out dist
```

The Python extension is built with PyO3 `abi3-py39`. This produces stable-ABI wheels such as `cp39-abi3-win_amd64.whl`, so one wheel per operating system and CPU architecture works across supported CPython versions 3.9 and newer. That avoids forcing users on Python 3.12 or 3.13 to compile from source.

Publish to TestPyPI:

1. Create a TestPyPI project named `fhr-monitor-analyzer`.
2. Configure TestPyPI Trusted Publishing for this GitHub repository and the `testpypi` environment.
3. Run the `Publish Python Package To TestPyPI` workflow manually from GitHub Actions.
4. Test the package:

```bash
python -m pip install --only-binary=:all: --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ fhr-monitor-analyzer
python -c "import fhr_monitor_analyzer; print(fhr_monitor_analyzer.__all__)"
```

Publish to PyPI:

1. Create the PyPI project named `fhr-monitor-analyzer`.
2. Configure PyPI Trusted Publishing for this GitHub repository and the `pypi` environment.
3. Update the version in `pyproject.toml` and `Cargo.toml`.
4. Commit and push the version change to `main`.
5. Push a matching version tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

6. Watch the `Publish Python Package` workflow in GitHub Actions.
7. After it succeeds, verify the release from a clean environment:

```bash
python -m pip install --only-binary=:all: fhr-monitor-analyzer
python -c "import fhr_monitor_analyzer; print(fhr_monitor_analyzer.__all__)"
```

The `Publish Python Package` workflow builds `abi3` wheels on Linux, macOS, and Windows, builds a source distribution, then publishes on tagged releases. The publish job only runs for tags; manual `workflow_dispatch` runs build artifacts but do not publish to PyPI.

## Next Implementation Steps

- Add an HTTP service layer, likely `axum`, around the existing Rust core
- Replace the prototype JSON formatter with `serde`
- Add fixture-based tests with clinician-reviewed sample tracings
- Tune TOCO/contraction detection against more real monitor exports
- Add persistent alert deduplication and resolution tracking per episode
