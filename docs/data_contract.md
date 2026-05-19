# FHR Decision-Support Data Contract

This prototype assumes the monitor pushes a recent data chunk. The caller does not need to declare whether the chunk is exactly 20, 22, or 30 minutes. The service infers the actual span from timestamps, analyzes what is available, and reports which clinical checks are complete or incomplete.

## Recommended Window

- Preferred: send the most recent 30 minutes every 60 seconds.
- Acceptable: send any recent chunk between 20 and 30 minutes.
- Minimum: 10 minutes for baseline and variability only.
- If more than 30 minutes is sent, the current-state analysis should use the latest 30 minutes and still return full input metadata.

Why:

- Baseline is evaluated over a 10-minute segment and needs at least 2 usable minutes.
- Recurrent deceleration screening needs a 20-minute view.
- Tachysystole assessment needs a 30-minute contraction average.

## JSON Shape

The future service endpoint should accept one JSON object per analysis request:

```http
POST /v1/analyze
Content-Type: application/json
```

Machine-readable schema: [`docs/request.schema.json`](request.schema.json).

The caller sends the recent monitor chunk it has. The caller does not need to calculate, label, or declare whether the chunk is 10, 20, 22, or 30 minutes. The service derives the actual duration from `samples[0].t` through the last sample timestamp after sorting.

### Minimal Valid Request

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

### Full Request Example

```json
{
  "episode_id": "18664805",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "analysis_options": {
    "fetal_channel": "HR1",
    "max_analysis_minutes": 30
  },
  "samples": [
    {
      "t": "2026-05-12T11:52:35.052Z",
      "hr1": 129,
      "hr2": null,
      "hr3": null,
      "hrm": 101,
      "toco": 33
    }
  ],
  "metadata": {
    "gestational_age_weeks": null,
    "labor_stage": null,
    "oxytocin_running": null,
    "recent_epidural": null,
    "pushing": null
  }
}
```

### Top-Level Fields

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `episode_id` | string | Yes | Stable identifier for the monitoring episode/labor encounter. Used for audit, alert deduplication, and future stateful service behavior. |
| `sent_at` | ISO-8601 timestamp string | Yes | Time the upstream system sent the request. Prefer UTC with `Z`, for example `2026-05-12T12:22:35.052Z`. |
| `samples` | array of sample objects | Yes | Raw monitor samples for the recent chunk. Must contain at least one sample. Sorting is not required; the service sorts by timestamp. |
| `analysis_options` | object | No | Optional caller preferences. Omit this for normal chunk analysis. |
| `metadata` | object | No | Optional clinical/context fields. Unknown fields should be preserved or ignored, not rejected. |

### `analysis_options`

| Field | Type | Required | Default | Meaning |
| --- | --- | --- | --- | --- |
| `fetal_channel` | string enum | No | `HR1` | Fetal channel to analyze. Allowed values: `HR1`, `HR2`, `HR3`. |
| `max_analysis_minutes` | number | No | `30` | Maximum trailing span used for current-state analysis. The service should cap at 30 minutes for now. This is not the chunk length and callers usually should omit it. |

Do not send a `window_minutes` or `chunk_minutes` field for normal operation. The service should infer the chunk duration from timestamps and return the inferred duration in response metadata.

### Sample Object

Each item in `samples` represents one monitor reading. The device may send irregular cadence, duplicate timestamps, or out-of-order rows; the service will sort and report those quality metrics.

| Field | Type | Required | Units | Meaning |
| --- | --- | --- | --- | --- |
| `t` | ISO-8601 timestamp string | Yes | time | Sample timestamp. Prefer UTC with milliseconds, for example `2026-05-12T11:52:35.052Z`. |
| `hr1` | number or `null` | No | bpm | Fetal heart-rate channel 1. |
| `hr2` | number or `null` | No | bpm | Fetal heart-rate channel 2, if present. |
| `hr3` | number or `null` | No | bpm | Fetal heart-rate channel 3, if present. |
| `hrm` | number or `null` | No | bpm | Maternal heart rate, used to detect possible fetal/maternal signal confusion. |
| `toco` | number or `null` | No | monitor units | External TOCO/IUPC-like uterine activity value. Unit scale can be device-specific. |

### Value Rules

- Heart-rate values are numeric bpm values.
- `null` or omitted heart-rate fields mean missing/unavailable signal.
- `0` in `hr1`, `hr2`, `hr3`, or `hrm` is treated as missing signal for compatibility with current device exports.
- `0` in `toco` is allowed because zero can be a real uterine-activity value.
- Fetal HR values outside the plausible analysis range are filtered before feature extraction.
- The request can contain more than 30 minutes. The service should use the latest 30 minutes for current-state analysis and still return full input metadata.
- The request can contain less than 30 minutes. The service should still analyze what is possible and return `limitations` for incomplete checks.

### Metadata Object

All metadata fields are optional. These fields are not required for the current CLI prototype, but they are useful for later alert routing and clinical display:

| Field | Type | Meaning |
| --- | --- | --- |
| `gestational_age_weeks` | number or `null` | Used for acceleration thresholds. Before 32 weeks, acceleration detection uses 10 bpm for 10 seconds; otherwise it uses 15 bpm for 15 seconds. |
| `labor_stage` | string or `null` | Example: `first_stage`, `second_stage`, `unknown`. |
| `oxytocin_running` | boolean or `null` | Useful for interpreting tachysystole-related alerts. |
| `recent_epidural` | boolean or `null` | Useful context for hypotension-related tracing changes. |
| `pushing` | boolean or `null` | Useful for second-stage interpretation and maternal HR confusion. |

## Response Shape

The service should return both the classification and the evidence behind it:

```json
{
  "classification": "category_ii",
  "alert_level": "warning",
  "baseline_bpm": 165,
  "variability_class": "minimal",
  "sinusoidal_pattern": false,
  "features": {
    "fetal_hr_mean_bpm": 147.0,
    "fetal_hr_p05_bpm": 138.0,
    "fetal_hr_p95_bpm": 156.0,
    "fetal_hr_percent_below_110": 0.0,
    "fetal_hr_percent_above_160": 0.2,
    "acceleration_count": 0,
    "deceleration_count": 0,
    "contraction_count": 1,
    "contractions_per_10_min": 0.333,
    "fetal_maternal_mean_difference_bpm": 63.5
  },
  "data_quality": {
    "fetal_usable_ratio": 0.92,
    "maternal_usable_ratio": 0.88,
    "toco_usable_ratio": 0.95,
    "suspected_maternal_capture_ratio": 0.01
  },
  "reasons": ["tachycardia", "minimal variability"],
  "high_risk_features": ["baseline changed from normal to tachycardia"],
  "protective_features": [],
  "limitations": []
}
```

The alerting system should interrupt clinicians only for concerning Category II patterns, higher-risk Category II patterns, possible Category III, or important data-quality failures. Lower-risk Category II findings should still be returned in the response for context and audit.

Alert levels are:

| Level | Meaning |
| --- | --- |
| `none` | No interruptive alert; structured findings are still returned. |
| `warning` | Category II pattern warrants clinician review. |
| `urgent_review` | Higher-risk Category II pattern should stand out above routine warnings. |
| `critical` | Possible Category III pattern. |
| `data_quality` | Tracing cannot be interpreted reliably enough. |

The full user-facing explanation is in [`docs/alerting_strategy.md`](alerting_strategy.md).

The `features` object is deliberately numeric so other systems can trend fetal-rate status without parsing alert text. It should be returned even when no alert fires or the tracing is unclassified due to signal quality.
