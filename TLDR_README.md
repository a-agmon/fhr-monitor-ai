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
python -m pip install --upgrade pip maturin pytest matplotlib notebook ipykernel
```

- Build/install Python package:

```bash
maturin develop --features python
python -c "import fhr_monitor_analyzer; print(fhr_monitor_analyzer.__file__)"
```

- Build wheel artifacts:

```bash
maturin build --release --features python --out dist
```

## Load And Use The Python Lib

- Analyze CSV and parse JSON:

```python
import json
import fhr_monitor_analyzer as analyzer

report_json = analyzer.analyze_csv_file("/path/to/monitor.csv", channel="HR1")
report = json.loads(report_json)
```

- Analyze service-style JSON:

```python
report_json = analyzer.analyze_json(request_json)
```

- Create a tracing diagram:

```python
analyzer.plot_csv_file("/path/to/monitor.csv", output="monitor_plot.png", channel="HR1")
```

- Run the demo notebook:

```bash
source .venv/bin/activate
python -c "import sys, fhr_monitor_analyzer; print(sys.executable); print(fhr_monitor_analyzer.__file__)"
python -m ipykernel install --user --name fhr-monitor-analyzer --display-name "Python (fhr-monitor-analyzer)"
python -m jupyter notebook examples/fhr_monitor_analyzer_demo.ipynb
```

- In Jupyter, select the `Python (fhr-monitor-analyzer)` kernel and run the notebook from the first cell.
- If import fails, run `import sys; print(sys.executable)` in a notebook cell. It should point to this repo's `.venv`.
- Use `import fhr_monitor_analyzer as analyzer`; do not reuse the module alias as a numeric variable in the notebook.

## Trigger GitHub Actions And Publish

- CI runs automatically on push to `main` and on pull requests.
- The PyPI publish workflow runs automatically only when you push a version tag like `v0.1.0`.
- The TestPyPI publish workflow is manual.

- Publish to TestPyPI:
  - In GitHub, open Actions.
  - Run `Publish Python Package To TestPyPI`.
  - Requires TestPyPI Trusted Publishing configured for the `testpypi` environment.

- Publish to PyPI:
  - Configure PyPI Trusted Publishing for the `pypi` environment.
  - Update the version in `pyproject.toml` and `Cargo.toml`.
  - Commit and push the version change to `main`; this runs CI.
  - Push a matching version tag; this runs the PyPI publish workflow:

```bash
git add pyproject.toml Cargo.toml
git commit -m "Release v0.1.0"
git push origin main

git tag v0.1.0
git push origin v0.1.0
```

- The publish workflow builds Linux, macOS, and Windows wheels plus an sdist, then publishes to PyPI if Trusted Publishing is configured correctly.
