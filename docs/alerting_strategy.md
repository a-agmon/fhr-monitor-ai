# FHR Alerting Strategy

This page explains how the FHR monitor decision-support engine classifies fetal heart-rate tracing chunks and decides whether to alert.

The system is decision support only. It does not diagnose fetal status, prescribe treatment, or replace clinician interpretation of the tracing. Its job is to identify concerning patterns, explain the evidence, and return enough metadata for a clinician-facing system to decide how to display the result.

## Core Idea

The engine separates two related outputs:

| Output | Meaning |
| --- | --- |
| `classification` | The ACOG/NICHD tracing category inferred from the current chunk: `category_i`, `category_ii`, `category_iii`, or `unclassified`. |
| `alert_level` | How interruptive the system thinks the finding should be: `none`, `warning`, `urgent_review`, `critical`, or `data_quality`. |

This separation matters because Category II is broad. Some Category II chunks have reassuring features and should not cause alert fatigue. Other Category II chunks deserve review because they contain recurrent, deep, prolonged, worsening, or poorly recovered decelerations, abnormal variability, or possible signal problems.

## Tracing Categories

| Category | Meaning | Current engine criteria |
| --- | --- | --- |
| `category_i` | Normal/reassuring tracing features. | Baseline 110-160 bpm, moderate variability, no late or variable decelerations. Early decelerations and accelerations may be present or absent. |
| `category_ii` | Indeterminate tracing. | Any classifiable tracing that is not Category I or Category III. Examples include marked variability, minimal variability, tachycardia, bradycardia without Category III criteria, prolonged deceleration, recurrent variable decelerations with preserved variability, or recurrent late decelerations with moderate variability. |
| `category_iii` | Abnormal tracing features where abnormal fetal acid-base status cannot be excluded. | Absent variability with recurrent late decelerations, recurrent variable decelerations, or bradycardia; or a detected sinusoidal pattern. |
| `unclassified` | The engine refuses to force a clinical category. | Usually caused by too little usable fetal HR for baseline/variability estimation or severe signal-quality limits. |

## Alert Levels

| Alert level | Meaning | Typical use |
| --- | --- | --- |
| `none` | No interruptive alert. | Low-concern Category I, or low-concern Category II with moderate variability and no high-risk features. The response still includes reasons, features, and limitations. |
| `warning` | Clinician review is warranted. | Category II with concerning but not near-critical features, such as marked variability, deep variable deceleration, high total deceleration burden, tachycardia trend, or possible maternal HR capture. |
| `urgent_review` | Higher-priority clinician review is warranted. | High-risk Category II patterns such as recurrent late decelerations, recurrent variable decelerations with deep nadir or high deceleration burden, absent variability without full Category III criteria, persistent minimal variability, or more than one prolonged deceleration. |
| `critical` | Possible Category III pattern. | Category III criteria detected. |
| `data_quality` | The tracing cannot be interpreted reliably enough. | Fetal HR signal is too sparse, baseline/variability cannot be determined, or other quality limits make the classification unsafe. |

There is intentionally no `info` alert level. If a finding should not interrupt the user, it stays `none` and appears in the structured response. If it should interrupt, the first interruptive level is `warning`.

## Protective Features

The engine reports protective features separately from alert reasons.

| Protective feature | How it affects alerting |
| --- | --- |
| Moderate variability | Strongly reassuring in the current rules. It can suppress a low-concern Category II alert when no high-risk features are present. |
| Accelerations present | Reassuring context. They can suppress low-concern Category II findings such as mild variable decelerations, unclear gradual decelerations without low variability, or borderline marked variability when no high-risk features are present. Accelerations do not cancel deep decelerations, recurrent variables, persistent minimal variability, tachycardia change, or concerning deceleration burden. |

This rule is important for alert fatigue. A single isolated variable deceleration with normal baseline, moderate variability, and accelerations may remain `none`. A tracing with accelerations but marked variability and deep decelerations should still alert.

## Warning Rules

The engine uses `warning` for Category II patterns that need attention but do not meet urgent-review or critical criteria.

Current warning triggers include:

- Marked variability above 30 bpm. Borderline marked variability remains Category II, but does not interrupt by itself.
- Category II without moderate-variability protection unless it is a low-concern marked/variable pattern with accelerations and no high-risk features.
- Severe variable deceleration, defined by any variable deceleration with nadir below 80 bpm, depth at least 60 bpm below baseline, or duration at least 60 seconds.
- Any deceleration with nadir below 80 bpm.
- Concerning deceleration burden: at least 120 total detected deceleration seconds, or at least 60 seconds plus recurrent decelerations, nadir below 100 bpm, or depth at least 30 bpm below baseline.
- Baseline changing from normal to tachycardia when enough history exists.
- Recurrent variable decelerations that do not meet urgent-review severity criteria.
- Tachysystole without other high-risk features.
- Possible maternal heart-rate capture.

## Urgent Review Rules

The engine uses `urgent_review` when the tracing is not Category III but the pattern is more concerning than a generic warning.

Current urgent-review triggers include:

- Recurrent late decelerations.
- Recurrent variable decelerations plus either nadir below 80 bpm or at least 60 total seconds in deceleration.
- Absent variability that does not otherwise meet Category III criteria.
- Gradual deceleration with absent or minimal variability when late/early timing cannot be determined cleanly.
- Persistent minimal variability for at least 20 minutes.
- Tachysystole with high-risk Category II features.
- More than one prolonged deceleration.

This level is intended for high-risk Category II patterns. It is deliberately below `critical` because preserved moderate variability or accelerations may still argue against current acidemia, but the tracing deserves prompt clinician review.

## Critical Rules

The engine uses `critical` for Category III criteria:

- Absent variability with recurrent late decelerations.
- Absent variability with recurrent variable decelerations.
- Absent variability with bradycardia.
- Sinusoidal pattern for at least 20 minutes.

Category III recurrence is intentionally more sensitive than lower-priority Category II alert recurrence: absent variability plus recurrent late/variable decelerations is treated as critical once a 20-minute window has at least two contractions and at least two matching decelerations involving at least half of contractions.

The sinusoidal detector is conservative: it requires enough usable fetal-HR signal, a smooth regular oscillation, peak-to-trough amplitude in the expected sinusoidal range, and dominant spectral power at 3-5 cycles per minute across the latest 20 minutes.

## Data Quality Rules

The engine returns `data_quality` when it should not force a clinical alert decision.

Current data-quality triggers include:

- Fetal usable ratio below 50%.
- Fetal usable ratio below 70% when the tracing is classifiable but has no high-risk features, to avoid clinical warning from marginal signal quality alone.
- `unclassified` tracing because baseline or variability could not be determined.

The response still includes numeric features when possible, but downstream systems should display the result as limited by signal quality.

## Window Length Limits

The service accepts variable-length chunks and infers duration from timestamps.

| Available span | What can be assessed |
| --- | --- |
| Less than 10 minutes | Baseline and variability are incomplete. |
| 10-20 minutes | Baseline and variability can be assessed if signal quality is sufficient, but recurrent deceleration assessment is limited. |
| 20-30 minutes | Recurrent deceleration logic is more useful. |
| 30 minutes or more | Tachysystole assessment can use the intended 30-minute contraction average. |

For current-state analysis, the engine caps the analyzed span at the latest 30 minutes even if the caller sends more.

## Numeric Proxies

Some values are deterministic approximations of visual EFM interpretation:

| Measurement | Current implementation |
| --- | --- |
| Baseline | Ten-minute trimmed mean rounded to the nearest 5 bpm. This approximates excluding accelerations, decelerations, marked variability, and large baseline shifts. |
| Variability | Per-minute p95-p05 range near baseline, then the median minute range. This is more robust to artifacts than raw max-min, but it is not identical to visual peak-to-trough interpretation. |
| Absent variability | Numeric threshold of 1 bpm or less. ACOG uses the visual term "undetectable"; this value should be calibrated against labeled examples. |
| Deceleration duration | Time below the deceleration threshold, with limited recovery-gap tolerance. This is close to but not identical to visual onset-to-recovery duration. |
| Late/early timing | Gradual decelerations are compared with detected TOCO peaks. If TOCO association is unclear and variability is absent or minimal, the engine treats that combination as higher risk rather than silently ignoring it. |
| Accelerations | Uses 15 bpm for 15 seconds at 32 weeks or later, and 10 bpm for 10 seconds before 32 weeks when gestational age is provided. Unknown gestational age defaults to the 32-week-or-later threshold. |

## Important Limitations

- The engine does not yet maintain state across chunks. The service layer should eventually track whether Category II features are persistent, improving, or worsening over consecutive pushes.
- The engine uses gestational age only for acceleration thresholds. It does not yet use labor stage, pushing, oxytocin, epidural timing, maternal fever, or intrauterine resuscitative interventions.
- The TOCO detector is a prototype and should be tuned against more examples.
- Late/early timing depends on TOCO quality.
- The variability estimate is numeric and deterministic; it approximates visual interpretation but does not replace it.
- This is not a treatment algorithm.

## References

- ACOG, Clinical Practice Guideline No. 10, Intrapartum Fetal Heart Rate Monitoring: Interpretation and Management: https://www.acog.org/clinical/clinical-guidance/clinical-practice-guideline/articles/2025/10/intrapartum-fetal-heart-rate-monitoring-interpretation-and-management
- ACOG fetal tracing summary: https://www.acog.org/community/districts-and-sections/district-iv/whats-new/countdown-to-intern-year-week-4-fetal-heart-tracings
- NICE/NICHD fetal heart-rate classification appendix: https://www.ncbi.nlm.nih.gov/books/NBK550641/
