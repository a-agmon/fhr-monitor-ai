# fhr-monitor-ai

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

## Chunk-Based Input

The intended service behavior is chunk-based. The device or upstream system sends the recent data it has; it does not need to tell the analyzer whether the chunk is exactly 20, 22, or 30 minutes.

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
cargo run --bin fhr-cli -- /path/to/monitor.csv --channel HR1
```

Return JSON:

```bash
cargo run --bin fhr-cli -- /path/to/monitor.csv --channel HR1 --json
```

Replay a file with fixed rolling windows:

```bash
cargo run --bin fhr-cli -- /path/to/monitor.csv --channel HR1 --window-min 30 --step-sec 60
```

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
- `info`: non-urgent Category II context
- `warning`: high-risk Category II or concerning trend
- `critical`: possible Category III
- `data_quality`: the tracing cannot be interpreted reliably

Even when no alert fires, the service should return all detected features and limitations so the UI can show useful context.

## Numeric Features

Each analysis window includes a `features` object intended for other systems that need fetal-rate status without parsing alert text. It includes distribution metrics such as min, p05, mean, median, p95, max, and standard deviation; seconds and percentages below 110 bpm, within 110-160 bpm, and above 160 bpm; acceleration/deceleration counts and duration; contraction counts; TOCO summary values; and maternal-vs-fetal mean HR difference when maternal HR is available.

## Repository Layout

```text
src/fhr_core/      Reusable analysis module
src/main.rs        CLI wrapper
docs/              Data contract and design notes
```

## Next Implementation Steps

- Add an HTTP service layer, likely `axum`, around the existing Rust core
- Replace the prototype JSON formatter with `serde`
- Add fixture-based tests with clinician-reviewed sample tracings
- Tune TOCO/contraction detection against more real monitor exports
- Add persistent alert deduplication and resolution tracking per episode
