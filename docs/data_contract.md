# FHR Decision-Support Data Contract

This prototype assumes the monitor pushes a recent rolling time window. The service should accept variable-length windows, but it should report which parts of the analysis are complete.

## Recommended Window

- Preferred: 30 minutes, pushed every 60 seconds.
- Acceptable: 20 minutes for baseline, variability, and recurrent deceleration screening.
- Minimum: 10 minutes for baseline and variability only.

Why:

- Baseline is evaluated over a 10-minute segment and needs at least 2 usable minutes.
- Recurrent deceleration screening needs a 20-minute view.
- Tachysystole assessment needs a 30-minute contraction average.

## JSON Shape

```json
{
  "episode_id": "18664805",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "window_minutes": 30,
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
