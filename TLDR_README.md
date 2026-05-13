# TLDR README

## Run The Rust CLI

- Analyze a CSV chunk:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --json
```

- Human-readable output instead of JSON:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1
```

- Rolling replay:

```bash
cargo run --bin fhr-monitor-analyzer-cli -- /path/to/monitor.csv --channel HR1 --window-min 20 --step-sec 60
```

## Create The Python Lib Locally

- Create env and install build tools:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip maturin pytest matplotlib notebook
```

- Build/install editable Python package:

```bash
maturin develop --features python
```

- Build wheel artifacts:

```bash
maturin build --release --features python --out dist
```

## Load And Use The Python Lib

- Analyze CSV and parse JSON:

```python
import json
import fhr_monitor_analyzer as fhr

report_json = fhr.analyze_csv_file("/path/to/monitor.csv", channel="HR1")
report = json.loads(report_json)
```

- Analyze service-style JSON:

```python
report_json = fhr.analyze_json(request_json)
```

- Create a tracing diagram:

```python
fhr.plot_csv_file("/path/to/monitor.csv", output="monitor_plot.png", channel="HR1")
```

- Run the demo notebook:

```bash
jupyter notebook examples/fhr_monitor_analyzer_demo.ipynb
```

## Trigger GitHub Actions And Publish

- CI runs automatically on push to `main` and on pull requests.

- Publish to TestPyPI:
  - In GitHub, open Actions.
  - Run `Publish Python Package To TestPyPI`.
  - Requires TestPyPI Trusted Publishing configured for the `testpypi` environment.

- Publish to PyPI:
  - Configure PyPI Trusted Publishing for the `pypi` environment.
  - Push a version tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

- The publish workflow builds Linux, macOS, and Windows wheels plus an sdist.

