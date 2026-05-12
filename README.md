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
