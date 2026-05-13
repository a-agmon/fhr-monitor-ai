# Python Library Plan

This document describes the Python package implementation for the Rust FHR rules engine. The package name is `fhr-monitor-analyzer`, and the import name is `fhr_monitor_analyzer`.

## Recommendation

Use `PyO3` plus `maturin` to publish a Python package backed by the existing Rust core.

Why this is the best fit:

- The rules engine stays deterministic and fast.
- Python users get a normal importable package.
- The CLI, future service, and Python library all call the same implementation.
- The analysis functions return the same JSON report shape across wrappers.
- `maturin` supports GitHub Actions builds for Linux, macOS, and Windows wheels.

Avoid a pure-Python port for the analyzer. It would increase maintenance risk because every rule change would need to be reimplemented and retested twice.

## Package Names

```text
Python distribution name: fhr-monitor-analyzer
Python import name:       fhr_monitor_analyzer
Rust crate name:          fhr-monitor-analyzer
Rust library name:        fhr_monitor_analyzer
CLI binary name:          fhr-monitor-analyzer-cli
Native extension:         fhr_monitor_analyzer._native
```

## Public Python Entry Points

```python
from fhr_monitor_analyzer import (
    analyze_json,
    analyze_csv,
    analyze_json_file,
    analyze_csv_file,
    plot_csv,
    plot_csv_file,
)
```

### 1. JSON String

```python
def analyze_json(request_json: str) -> str:
    """Analyze one JSON request string using docs/data_contract.md."""
```

Input:

- A JSON string following the request contract in `docs/data_contract.md`.
- The string contains `episode_id`, `sent_at`, optional `analysis_options`, optional `metadata`, and `samples`.

Output:

- A JSON report string matching the service response shape.
- Invalid JSON or invalid request shape raises `ValueError`.

### 2. CSV String

```python
def analyze_csv(
    csv_text: str,
    *,
    channel: str = "HR1",
    ga_weeks: int | None = None,
    window_min: int | None = None,
    step_sec: int = 60,
    last_only: bool = False,
) -> str:
    """Analyze monitor CSV text using the current CSV export columns."""
```

Input:

- Raw CSV text with `Date` and any available `HR1`, `HR2`, `HR3`, `HRM`, and `TOCO` columns.
- Options mirror the CLI flags.

Output:

- A JSON report string.

### 3. JSON File

```python
def analyze_json_file(path: str | Path) -> str:
    """Read and analyze a JSON request file."""
```

Output:

- A JSON report string.

### 4. CSV File

```python
def analyze_csv_file(
    path: str | Path,
    *,
    channel: str = "HR1",
    ga_weeks: int | None = None,
    window_min: int | None = None,
    step_sec: int = 60,
    last_only: bool = False,
) -> str:
    """Read and analyze a monitor CSV file."""
```

Output:

- A JSON report string.

### 5. CSV Plotting

```python
def plot_csv(csv_text: str, output: str | Path, **options) -> str:
    """Render CSV text to a PNG and return the written path."""

def plot_csv_file(path: str | Path, output: str | Path | None = None, **options) -> str:
    """Render a CSV file to a PNG and return the written path."""
```

Plotting options:

- `channel`: primary fetal channel, default `HR1`.
- `all_fetal`: also plot nonselected fetal channels.
- `max_minutes`: plot only the latest N minutes.
- `title`: optional figure title.
- `dpi`: output resolution, default `150`.

## Return Type Policy

The Rust binding layer and public Python analysis functions return JSON strings. Callers can use `json.loads(report_json)` when a Python dictionary is needed.

That gives each integration a clean boundary:

- Rust owns validation, feature extraction, classification, and report generation.
- Python, CLI, and future HTTP callers share the same report contract.
- API servers can return the string directly as `application/json`.

## File Layout

```text
pyproject.toml
src/
  lib.rs
  main.rs
  python.rs                 PyO3 module and Rust/Python boundary.
  fhr_core/
    analysis.rs
    csv_input.rs
    json_input.rs           Parser for docs/data_contract.md.
    model.rs
    time.rs
python/
  fhr_monitor_analyzer/
    __init__.py             Thin Python wrappers around the native extension.
    plotting.py             Importable diagramming API.
    py.typed
tests/
  python/
    test_python_api.py
.github/
  workflows/
    ci.yml
    publish-python.yml
    publish-testpypi.yml
```

The repository can stay as a single Rust crate. It does not need to become a multi-crate workspace until there is a real need.

## Implementation Status

Completed:

- `read_monitor_csv_str`.
- `read_analysis_request_json`.
- PyO3 native functions returning JSON strings.
- Four Python analysis functions returning JSON strings.
- Python plotting functions for CSV diagrams.
- GitHub Actions workflow for CI.
- GitHub Actions workflows for TestPyPI and PyPI publishing.

Still recommended:

- Replace manual response JSON formatting with `serde` serialization.
- Add more fixture parity tests with clinician-reviewed data.
- Add response JSON schema after serialization is stable.

## GitHub Actions Publishing

The publishing workflows build wheels on:

- Linux
- macOS
- Windows

They also build a source distribution.

### CI Workflow

Runs on pushes to `main` and pull requests:

- `cargo fmt --check`
- `cargo test`
- `maturin develop --features python`
- `pytest tests/python`

### TestPyPI Workflow

`Publish Python Package To TestPyPI` is manually triggered with `workflow_dispatch`.

It builds Linux, macOS, and Windows wheels plus an sdist, then publishes to TestPyPI through Trusted Publishing.

### PyPI Workflow

`Publish Python Package` runs on tags such as `v0.1.0`.

It builds Linux, macOS, and Windows wheels plus an sdist, then publishes to PyPI through Trusted Publishing.

Use PyPI Trusted Publishing instead of a long-lived API token. It is safer because GitHub receives temporary publish permission for the specific project and environment.

## Local Developer Commands

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip maturin pytest matplotlib
maturin develop --features python
pytest tests/python
```

Build local artifacts:

```bash
maturin build --release --features python --out dist
```

Install after release:

```bash
pip install fhr-monitor-analyzer
pip install "fhr-monitor-analyzer[plot]"
```

## Versioning

Use semantic versioning:

- Patch version: rule documentation, bug fixes, and parser fixes that do not change response fields.
- Minor version: additive fields, new metrics, new optional request metadata, or new wrappers.
- Major version: renamed fields, removed fields, changed alert-level meanings, or incompatible request changes.

The Python package version and Rust crate version should stay the same.

