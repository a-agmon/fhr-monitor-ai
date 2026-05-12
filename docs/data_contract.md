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

```json
{
  "episode_id": "18664805",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "chunk_hint_minutes": null,
  "samples": [
    {
      "t": "2026-05-12T11:52:35.052Z",
      "hr1": 129,
      "hr2": 0,
      "hr3": 0,
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

Zero values in heart-rate channels are treated as missing signal. `TOCO` zero is allowed because it can be a real external-toco value, but long flat zero segments should still be surfaced as a signal-quality concern by the API layer.

## Response Shape

The service should return both the classification and the evidence behind it:

```json
{
  "classification": "category_ii",
  "alert_level": "warning",
  "baseline_bpm": 165,
  "variability_class": "minimal",
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

The alerting system should interrupt clinicians only for high-risk Category II, possible Category III, or important data-quality failures. Lower-risk Category II findings should still be returned in the response for context and audit.

The `features` object is deliberately numeric so other systems can trend fetal-rate status without parsing alert text. It should be returned even when no alert fires or the tracing is unclassified due to signal quality.
