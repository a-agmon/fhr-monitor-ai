use std::collections::{BTreeMap, HashSet};

use super::model::*;
use super::time::{seconds_between, whole_seconds_between};

const TEN_MIN_MS: i64 = 10 * 60 * 1_000;
const TWENTY_MIN_MS: i64 = 20 * 60 * 1_000;
const THIRTY_MIN_MS: i64 = 30 * 60 * 1_000;
const DEEP_DECELERATION_NADIR_BPM: f64 = 80.0;
const CONCERNING_DECELERATION_NADIR_BPM: f64 = 100.0;
const CONCERNING_DECELERATION_DEPTH_BPM: f64 = 30.0;
const DECELERATION_BURDEN_SECONDS: f64 = 60.0;
const STANDALONE_DECELERATION_BURDEN_SECONDS: f64 = 120.0;
const CONCERNING_MARKED_VARIABILITY_BPM: f64 = 30.0;
const MARGINAL_FETAL_USABLE_RATIO: f64 = 0.70;
const SEVERE_VARIABLE_DEPTH_BPM: f64 = 60.0;
const SEVERE_VARIABLE_DURATION_SECONDS: f64 = 60.0;
const EARLY_DECELERATION_PEAK_TOLERANCE_MS: i64 = 5_000;
const LATE_DECELERATION_DELAY_MS: i64 = 5_000;

#[derive(Clone, Debug)]
struct SecondSample {
    timestamp_ms: i64,
    timestamp: String,
    fhr: Option<f64>,
    hrm: Option<f64>,
    toco: Option<f64>,
}

#[derive(Default)]
struct Bucket {
    label: Option<String>,
    fhr_sum: f64,
    fhr_count: usize,
    hrm_sum: f64,
    hrm_count: usize,
    toco_sum: f64,
    toco_count: usize,
}

impl Bucket {
    fn add_fhr(&mut self, value: Option<f64>) {
        if let Some(value) = value.filter(|value| valid_heart_rate(*value)) {
            self.fhr_sum += value;
            self.fhr_count += 1;
        }
    }

    fn add_hrm(&mut self, value: Option<f64>) {
        if let Some(value) = value.filter(|value| valid_heart_rate(*value)) {
            self.hrm_sum += value;
            self.hrm_count += 1;
        }
    }

    fn add_toco(&mut self, value: Option<f64>) {
        if let Some(value) = value.filter(|value| (0.0..=300.0).contains(value)) {
            self.toco_sum += value;
            self.toco_count += 1;
        }
    }

    fn mean_fhr(&self) -> Option<f64> {
        (self.fhr_count > 0).then_some(self.fhr_sum / self.fhr_count as f64)
    }

    fn mean_hrm(&self) -> Option<f64> {
        (self.hrm_count > 0).then_some(self.hrm_sum / self.hrm_count as f64)
    }

    fn mean_toco(&self) -> Option<f64> {
        (self.toco_count > 0).then_some(self.toco_sum / self.toco_count as f64)
    }
}

pub fn analyze_rolling_windows(input: &InputData, config: AnalysisConfig) -> AnalysisReport {
    let start = input.samples.first();
    let end = input.samples.last();
    let input_summary = InputSummary {
        rows: input.samples.len(),
        start_timestamp: start.map(|sample| sample.timestamp.clone()),
        end_timestamp: end.map(|sample| sample.timestamp.clone()),
        duration_seconds: match (start, end) {
            (Some(start), Some(end)) => seconds_between(start.timestamp_ms, end.timestamp_ms),
            _ => 0.0,
        },
        out_of_order_rows: input.out_of_order_rows,
        duplicate_timestamps: input.duplicate_timestamps,
    };

    if input.samples.is_empty() {
        return AnalysisReport {
            config,
            input: input_summary,
            windows: Vec::new(),
        };
    }

    let data_start = input.samples.first().unwrap().timestamp_ms;
    let data_end = input.samples.last().unwrap().timestamp_ms;
    let mut windows = Vec::new();

    // Two modes are supported because we need both operational analysis and
    // offline replay. Service callers send an arbitrary recent chunk; replay
    // callers can request fixed rolling windows to inspect how features evolve.
    if let Some(window_minutes) = config.window_minutes {
        let window_ms = window_minutes as i64 * 60 * 1_000;
        let step_ms = config.step_seconds.max(1) as i64 * 1_000;
        let first_end = if data_end - data_start < window_ms {
            data_end
        } else {
            data_start + window_ms
        };
        let mut cursor = first_end;
        let mut last_analyzed_end = None;

        while cursor <= data_end {
            let window_start = (cursor - window_ms).max(data_start);
            windows.push(analyze_window(input, &config, window_start, cursor));
            last_analyzed_end = Some(cursor);
            cursor += step_ms;
        }

        if last_analyzed_end != Some(data_end) {
            let window_start = (data_end - window_ms).max(data_start);
            windows.push(analyze_window(input, &config, window_start, data_end));
        }
    } else {
        let chunk_ms = data_end - data_start;
        // Current-state analysis only needs the latest 30 minutes. We still
        // report the full input duration separately so upstream systems can
        // audit what was sent versus what was clinically interpretable.
        let window_start = if chunk_ms > THIRTY_MIN_MS {
            data_end - THIRTY_MIN_MS
        } else {
            data_start
        };
        windows.push(analyze_window(input, &config, window_start, data_end));
    }

    AnalysisReport {
        config,
        input: input_summary,
        windows,
    }
}

fn analyze_window(
    input: &InputData,
    config: &AnalysisConfig,
    window_start_ms: i64,
    window_end_ms: i64,
) -> WindowAnalysis {
    // The window analysis is intentionally ordered from raw signal handling to
    // clinical interpretation. This lets data-quality failures stop short of a
    // misleading category while still returning useful numeric status features.
    let raw_window: Vec<&MonitorSample> = input
        .samples
        .iter()
        .filter(|sample| {
            sample.timestamp_ms >= window_start_ms && sample.timestamp_ms <= window_end_ms
        })
        .collect();
    let seconds = resample_to_seconds(
        &raw_window,
        config.fetal_channel,
        window_start_ms,
        window_end_ms,
    );
    let data_quality =
        calculate_quality(&seconds, raw_window.len(), window_start_ms, window_end_ms);
    let duration_seconds = seconds_between(window_start_ms, window_end_ms);
    let window_start = raw_window
        .first()
        .map(|sample| sample.timestamp.clone())
        .or_else(|| seconds.first().map(|sample| sample.timestamp.clone()))
        .unwrap_or_else(|| "unknown".to_string());
    let window_end = raw_window
        .last()
        .map(|sample| sample.timestamp.clone())
        .or_else(|| seconds.last().map(|sample| sample.timestamp.clone()))
        .unwrap_or_else(|| "unknown".to_string());

    // ACOG/NICHD baseline and variability are assessed on a 10-minute segment.
    // If the most recent segment has too little usable FHR, the result remains
    // unclassified and the caller receives a data-quality limitation.
    let current_start = (window_end_ms - TEN_MIN_MS).max(window_start_ms);
    let baseline_bpm = estimate_baseline(&seconds, current_start, window_end_ms);
    let baseline_class = baseline_bpm.map(classify_baseline);
    let variability_bpm = baseline_bpm.and_then(|baseline| {
        estimate_variability(&seconds, baseline as f64, current_start, window_end_ms)
    });
    let variability_class = variability_bpm.map(classify_variability);

    // Event detection uses the recent 20-minute context when available. That is
    // enough to start testing recurrent-pattern logic without requiring callers
    // to know or declare whether they sent exactly 20 or 30 minutes.
    let eval_start = (window_end_ms - TWENTY_MIN_MS).max(window_start_ms);
    let accelerations = baseline_bpm
        .map(|baseline| {
            detect_accelerations(
                &seconds,
                baseline as f64,
                eval_start,
                window_end_ms,
                config.gestational_age_weeks,
            )
        })
        .unwrap_or_default();
    let mut decelerations = baseline_bpm
        .map(|baseline| detect_decelerations(&seconds, baseline as f64, eval_start, window_end_ms))
        .unwrap_or_default();
    let contractions = detect_contractions(&seconds, eval_start, window_end_ms);
    associate_decelerations_with_contractions(&mut decelerations, &contractions);

    let toco = summarize_toco(&seconds, contractions, window_start_ms, window_end_ms);
    // Numeric features are emitted even when the tracing is unclassified. Other
    // systems can trend fetal-rate status without parsing alert text.
    let features = calculate_numeric_features(
        &seconds,
        baseline_bpm,
        &accelerations,
        &decelerations,
        &toco,
    );
    let mut reasons = Vec::new();
    let mut high_risk_features = Vec::new();
    let mut protective_features = Vec::new();
    let mut limitations = calculate_limitations(
        duration_seconds,
        &data_quality,
        window_start_ms,
        window_end_ms,
        &toco,
    );

    let classification = classify_window(
        baseline_class,
        variability_class,
        &decelerations,
        &toco,
        window_start_ms,
        window_end_ms,
        &mut reasons,
    );
    find_risk_features(
        &seconds,
        baseline_bpm,
        baseline_class,
        variability_bpm,
        variability_class,
        &accelerations,
        &decelerations,
        &toco,
        window_start_ms,
        window_end_ms,
        &mut high_risk_features,
        &mut protective_features,
    );

    if baseline_bpm.is_none() {
        limitations.push("baseline indeterminate: fewer than 120 usable fetal-HR seconds in the current 10-minute segment".to_string());
    }
    if data_quality.suspected_maternal_capture_ratio > 0.25 {
        high_risk_features.push(
            "possible maternal heart-rate capture: fetal channel often tracks HRM within 5 bpm"
                .to_string(),
        );
    }
    apply_tachysystole_context(classification, &toco, &mut reasons, &mut high_risk_features);
    sort_and_dedup(&mut reasons);
    sort_and_dedup(&mut high_risk_features);
    sort_and_dedup(&mut protective_features);

    let alert_level = choose_alert_level(
        classification,
        &high_risk_features,
        &protective_features,
        &reasons,
        &features,
        &data_quality,
    );

    WindowAnalysis {
        window_start,
        window_end,
        duration_seconds,
        data_quality,
        features,
        baseline_bpm,
        baseline_class,
        variability_bpm,
        variability_class,
        accelerations,
        decelerations,
        toco,
        classification,
        alert_level,
        reasons,
        high_risk_features,
        protective_features,
        limitations,
    }
}

fn resample_to_seconds(
    raw: &[&MonitorSample],
    channel: FetalChannel,
    start_ms: i64,
    end_ms: i64,
) -> Vec<SecondSample> {
    let mut buckets: BTreeMap<i64, Bucket> = BTreeMap::new();
    for sample in raw {
        let sec = sample.timestamp_ms.div_euclid(1_000) * 1_000;
        let bucket = buckets.entry(sec).or_default();
        if bucket.label.is_none() {
            bucket.label = Some(sample.timestamp.clone());
        }
        bucket.add_fhr(sample.fetal_value(channel));
        bucket.add_hrm(sample.hrm);
        bucket.add_toco(sample.toco);
    }

    let first_sec = start_ms.div_euclid(1_000) * 1_000;
    let last_sec = end_ms.div_euclid(1_000) * 1_000;
    let mut seconds = Vec::new();
    let mut sec = first_sec;
    while sec <= last_sec {
        let bucket = buckets.get(&sec);
        seconds.push(SecondSample {
            timestamp_ms: sec,
            timestamp: bucket
                .and_then(|bucket| bucket.label.clone())
                .unwrap_or_else(|| format!("{}ms", sec)),
            fhr: bucket.and_then(Bucket::mean_fhr),
            hrm: bucket.and_then(Bucket::mean_hrm),
            toco: bucket.and_then(Bucket::mean_toco),
        });
        sec += 1_000;
    }
    seconds
}

fn calculate_quality(
    seconds: &[SecondSample],
    raw_samples: usize,
    window_start_ms: i64,
    window_end_ms: i64,
) -> DataQuality {
    let expected_seconds = whole_seconds_between(window_start_ms, window_end_ms).max(seconds.len());
    let fetal_usable_seconds = seconds.iter().filter(|sample| sample.fhr.is_some()).count();
    let maternal_usable_seconds = seconds.iter().filter(|sample| sample.hrm.is_some()).count();
    let toco_usable_seconds = seconds
        .iter()
        .filter(|sample| sample.toco.is_some())
        .count();
    let mut overlap = 0;
    let mut close = 0;
    for sample in seconds {
        if let (Some(fhr), Some(hrm)) = (sample.fhr, sample.hrm) {
            overlap += 1;
            if (fhr - hrm).abs() <= 5.0 {
                close += 1;
            }
        }
    }
    DataQuality {
        expected_seconds,
        raw_samples,
        fetal_usable_seconds,
        fetal_usable_ratio: ratio(fetal_usable_seconds, expected_seconds),
        maternal_usable_ratio: ratio(maternal_usable_seconds, expected_seconds),
        toco_usable_ratio: ratio(toco_usable_seconds, expected_seconds),
        suspected_maternal_capture_ratio: ratio(close, overlap),
    }
}

fn calculate_numeric_features(
    seconds: &[SecondSample],
    baseline_bpm: Option<i32>,
    accelerations: &[AccelerationEvent],
    decelerations: &[DecelerationEvent],
    toco: &TocoSummary,
) -> NumericFeatures {
    let mut fhr_values: Vec<f64> = seconds.iter().filter_map(|sample| sample.fhr).collect();
    let fhr_count = fhr_values.len();
    let (
        fetal_hr_min_bpm,
        fetal_hr_p05_bpm,
        fetal_hr_mean_bpm,
        fetal_hr_median_bpm,
        fetal_hr_p95_bpm,
        fetal_hr_max_bpm,
        fetal_hr_std_dev_bpm,
    ) = summarize_values(&mut fhr_values);
    let fetal_hr_seconds_below_110 = seconds
        .iter()
        .filter(|sample| sample.fhr.is_some_and(|value| value < 110.0))
        .count();
    let fetal_hr_seconds_above_160 = seconds
        .iter()
        .filter(|sample| sample.fhr.is_some_and(|value| value > 160.0))
        .count();
    let fetal_hr_seconds_110_to_160 = seconds
        .iter()
        .filter(|sample| {
            sample
                .fhr
                .is_some_and(|value| (110.0..=160.0).contains(&value))
        })
        .count();

    let mut toco_values: Vec<f64> = seconds.iter().filter_map(|sample| sample.toco).collect();
    let (_, _, toco_mean, _, _, toco_max, _) = summarize_values(&mut toco_values);
    let mut maternal_values: Vec<f64> = seconds.iter().filter_map(|sample| sample.hrm).collect();
    let (_, _, maternal_hr_mean_bpm, _, _, _, _) = summarize_values(&mut maternal_values);

    let baseline_delta_mean_bpm = match (fetal_hr_mean_bpm, baseline_bpm) {
        (Some(mean), Some(baseline)) => Some(mean - baseline as f64),
        _ => None,
    };
    let fetal_maternal_mean_difference_bpm = match (fetal_hr_mean_bpm, maternal_hr_mean_bpm) {
        (Some(fetal), Some(maternal)) => Some(fetal - maternal),
        _ => None,
    };

    NumericFeatures {
        fetal_hr_min_bpm,
        fetal_hr_p05_bpm,
        fetal_hr_mean_bpm,
        fetal_hr_median_bpm,
        fetal_hr_p95_bpm,
        fetal_hr_max_bpm,
        fetal_hr_std_dev_bpm,
        baseline_delta_mean_bpm,
        fetal_hr_seconds_below_110,
        fetal_hr_seconds_110_to_160,
        fetal_hr_seconds_above_160,
        fetal_hr_percent_below_110: ratio(fetal_hr_seconds_below_110, fhr_count) * 100.0,
        fetal_hr_percent_110_to_160: ratio(fetal_hr_seconds_110_to_160, fhr_count) * 100.0,
        fetal_hr_percent_above_160: ratio(fetal_hr_seconds_above_160, fhr_count) * 100.0,
        acceleration_count: accelerations.len(),
        deceleration_count: decelerations.len(),
        prolonged_deceleration_count: decelerations
            .iter()
            .filter(|event| event.kind == DecelerationKind::Prolonged)
            .count(),
        total_deceleration_seconds: if decelerations.is_empty() {
            0.0
        } else {
            decelerations
                .iter()
                .map(|event| event.duration_seconds)
                .sum()
        },
        deepest_deceleration_nadir_bpm: decelerations
            .iter()
            .map(|event| event.nadir_bpm)
            .min_by(f64::total_cmp),
        max_deceleration_depth_bpm: decelerations
            .iter()
            .map(|event| event.depth_bpm)
            .max_by(f64::total_cmp),
        contraction_count: toco.contractions.len(),
        contractions_per_10_min: toco.contractions_per_10_min,
        toco_mean,
        toco_max,
        maternal_hr_mean_bpm,
        fetal_maternal_mean_difference_bpm,
    }
}

fn estimate_baseline(seconds: &[SecondSample], start_ms: i64, end_ms: i64) -> Option<i32> {
    let mut values: Vec<f64> = seconds
        .iter()
        .filter(|sample| sample.timestamp_ms >= start_ms && sample.timestamp_ms <= end_ms)
        .filter_map(|sample| sample.fhr)
        .collect();
    if values.len() < 120 {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let low = values.len() / 10;
    let high = values.len() - low;
    let trimmed = &values[low..high.max(low + 1)];
    let mean = trimmed.iter().sum::<f64>() / trimmed.len() as f64;
    Some((mean / 5.0).round() as i32 * 5)
}

fn classify_baseline(baseline: i32) -> BaselineClass {
    if baseline < 110 {
        BaselineClass::Bradycardia
    } else if baseline > 160 {
        BaselineClass::Tachycardia
    } else {
        BaselineClass::Normal
    }
}

fn estimate_variability(
    seconds: &[SecondSample],
    baseline: f64,
    start_ms: i64,
    end_ms: i64,
) -> Option<f64> {
    let mut minute_ranges = Vec::new();
    let mut cursor = start_ms;
    while cursor < end_ms {
        let next = (cursor + 60_000).min(end_ms);
        let mut values: Vec<f64> = seconds
            .iter()
            .filter(|sample| sample.timestamp_ms >= cursor && sample.timestamp_ms < next)
            .filter_map(|sample| sample.fhr)
            .filter(|value| (*value - baseline).abs() <= 25.0)
            .collect();
        if values.len() >= 30 {
            values.sort_by(f64::total_cmp);
            minute_ranges.push(percentile(&values, 0.95) - percentile(&values, 0.05));
        }
        cursor = next;
    }
    if minute_ranges.len() < 2 {
        return None;
    }
    minute_ranges.sort_by(f64::total_cmp);
    Some(percentile(&minute_ranges, 0.50))
}

fn classify_variability(amplitude: f64) -> VariabilityClass {
    if amplitude <= 1.0 {
        VariabilityClass::Absent
    } else if amplitude <= 5.0 {
        VariabilityClass::Minimal
    } else if amplitude <= 25.0 {
        VariabilityClass::Moderate
    } else {
        VariabilityClass::Marked
    }
}

fn detect_accelerations(
    seconds: &[SecondSample],
    baseline: f64,
    start_ms: i64,
    end_ms: i64,
    gestational_age_weeks: Option<u8>,
) -> Vec<AccelerationEvent> {
    let threshold = baseline + acceleration_threshold_bpm(gestational_age_weeks);
    let minimum_duration_seconds = acceleration_minimum_duration_seconds(gestational_age_weeks);
    detect_segments(seconds, start_ms, end_ms, |sample| {
        sample.fhr.is_some_and(|value| value >= threshold)
    })
    .into_iter()
    .filter_map(|segment| {
        let duration = seconds_between(segment.start_ms, segment.end_ms);
        if !(minimum_duration_seconds..120.0).contains(&duration) {
            return None;
        }
        let peak = seconds
            .iter()
            .filter(|sample| {
                sample.timestamp_ms >= segment.start_ms && sample.timestamp_ms <= segment.end_ms
            })
            .filter_map(|sample| sample.fhr.map(|value| (sample.timestamp_ms, value)))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if seconds_between(segment.start_ms, peak.0) >= 30.0 {
            return None;
        }
        Some(AccelerationEvent {
            start: label_for_time(seconds, segment.start_ms),
            end: label_for_time(seconds, segment.end_ms),
            duration_seconds: duration,
            peak_bpm: peak.1,
        })
    })
    .collect()
}

fn acceleration_threshold_bpm(gestational_age_weeks: Option<u8>) -> f64 {
    if gestational_age_weeks.is_some_and(|weeks| weeks < 32) {
        10.0
    } else {
        15.0
    }
}

fn acceleration_minimum_duration_seconds(gestational_age_weeks: Option<u8>) -> f64 {
    if gestational_age_weeks.is_some_and(|weeks| weeks < 32) {
        10.0
    } else {
        15.0
    }
}

fn detect_decelerations(
    seconds: &[SecondSample],
    baseline: f64,
    start_ms: i64,
    end_ms: i64,
) -> Vec<DecelerationEvent> {
    let threshold = baseline - 15.0;
    detect_segments(seconds, start_ms, end_ms, |sample| {
        sample.fhr.is_some_and(|value| value <= threshold)
    })
    .into_iter()
    .filter_map(|segment| {
        let values: Vec<&SecondSample> = seconds
            .iter()
            .filter(|sample| {
                sample.timestamp_ms >= segment.start_ms && sample.timestamp_ms <= segment.end_ms
            })
            .filter(|sample| sample.fhr.is_some())
            .collect();
        let duration = seconds_between(segment.start_ms, segment.end_ms);
        if duration < 15.0 {
            return None;
        }
        let nadir = values
            .iter()
            .min_by(|a, b| a.fhr.unwrap().total_cmp(&b.fhr.unwrap()))?;
        let nadir_bpm = nadir.fhr?;
        let depth = baseline - nadir_bpm;
        if depth < 15.0 {
            return None;
        }
        let onset_to_nadir = seconds_between(segment.start_ms, nadir.timestamp_ms);
        let kind = if duration >= 120.0 && duration < 600.0 {
            DecelerationKind::Prolonged
        } else if onset_to_nadir < 30.0 {
            DecelerationKind::Variable
        } else {
            DecelerationKind::GradualUnclassified
        };
        Some(DecelerationEvent {
            start: label_for_time(seconds, segment.start_ms),
            end: label_for_time(seconds, segment.end_ms),
            duration_seconds: duration,
            nadir_bpm,
            depth_bpm: depth,
            onset_to_nadir_seconds: onset_to_nadir,
            kind,
        })
    })
    .collect()
}

#[derive(Clone, Copy)]
struct Segment {
    start_ms: i64,
    end_ms: i64,
}

fn detect_segments(
    seconds: &[SecondSample],
    start_ms: i64,
    end_ms: i64,
    predicate: impl Fn(&SecondSample) -> bool,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut active_start = None;
    let mut last_matching = None;
    let mut gap_seconds = 0;
    for sample in seconds
        .iter()
        .filter(|sample| sample.timestamp_ms >= start_ms && sample.timestamp_ms <= end_ms)
    {
        if predicate(sample) {
            if active_start.is_none() {
                active_start = Some(sample.timestamp_ms);
            }
            last_matching = Some(sample.timestamp_ms + 1_000);
            gap_seconds = 0;
        } else if active_start.is_some() {
            gap_seconds += 1;
            if gap_seconds > 3 {
                segments.push(Segment {
                    start_ms: active_start.unwrap(),
                    end_ms: last_matching.unwrap_or(sample.timestamp_ms),
                });
                active_start = None;
                last_matching = None;
                gap_seconds = 0;
            }
        }
    }
    if let Some(start) = active_start {
        segments.push(Segment {
            start_ms: start,
            end_ms: last_matching.unwrap_or(end_ms),
        });
    }
    segments
}

fn detect_contractions(
    seconds: &[SecondSample],
    start_ms: i64,
    end_ms: i64,
) -> Vec<ContractionEvent> {
    let toco_values: Vec<(i64, f64)> = seconds
        .iter()
        .filter(|sample| sample.timestamp_ms >= start_ms && sample.timestamp_ms <= end_ms)
        .filter_map(|sample| sample.toco.map(|value| (sample.timestamp_ms, value)))
        .collect();
    if toco_values.len() < 60 {
        return Vec::new();
    }
    let mut values: Vec<f64> = toco_values.iter().map(|(_, value)| *value).collect();
    values.sort_by(f64::total_cmp);
    let baseline = percentile(&values, 0.20);
    let p90 = percentile(&values, 0.90);
    let p95 = percentile(&values, 0.95);
    let p99 = percentile(&values, 0.99);
    if p95 - baseline < 8.0 {
        return Vec::new();
    }
    let threshold = baseline + 6.0_f64.max((p90 - baseline) * 0.30);
    let peak_threshold = baseline + 15.0_f64.max((p99 - baseline) * 0.50);
    let smoothed = smooth_toco(seconds, start_ms, end_ms);
    let mut events = Vec::new();
    let mut active_start = None;
    let mut active_peak = None;
    let mut active_peak_value = f64::MIN;
    let mut last_above = None;

    for (timestamp_ms, value) in smoothed {
        if value >= threshold {
            if active_start.is_none() {
                active_start = Some(timestamp_ms);
                active_peak = Some(timestamp_ms);
                active_peak_value = value;
            }
            if value > active_peak_value {
                active_peak = Some(timestamp_ms);
                active_peak_value = value;
            }
            last_above = Some(timestamp_ms + 1_000);
        } else if let Some(start) = active_start {
            let end = last_above.unwrap_or(timestamp_ms);
            let duration = seconds_between(start, end);
            if duration >= 5.0 && active_peak_value >= peak_threshold {
                events.push(ContractionEvent {
                    start: label_for_time(seconds, start),
                    peak: label_for_time(seconds, active_peak.unwrap_or(start)),
                    end: label_for_time(seconds, end),
                    duration_seconds: duration,
                    peak_toco: active_peak_value,
                });
            }
            active_start = None;
            active_peak = None;
            active_peak_value = f64::MIN;
            last_above = None;
        }
    }
    if let Some(start) = active_start {
        let end = last_above.unwrap_or(end_ms);
        let duration = seconds_between(start, end);
        if duration >= 5.0 && active_peak_value >= peak_threshold {
            events.push(ContractionEvent {
                start: label_for_time(seconds, start),
                peak: label_for_time(seconds, active_peak.unwrap_or(start)),
                end: label_for_time(seconds, end),
                duration_seconds: duration,
                peak_toco: active_peak_value,
            });
        }
    }
    merge_close_contractions(events, seconds)
}

fn smooth_toco(seconds: &[SecondSample], start_ms: i64, end_ms: i64) -> Vec<(i64, f64)> {
    let range: Vec<&SecondSample> = seconds
        .iter()
        .filter(|sample| sample.timestamp_ms >= start_ms && sample.timestamp_ms <= end_ms)
        .collect();
    let mut smoothed = Vec::new();
    for (idx, sample) in range.iter().enumerate() {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(range.len());
        let mut sum = 0.0;
        let mut count = 0;
        for item in &range[lo..hi] {
            if let Some(value) = item.toco {
                sum += value;
                count += 1;
            }
        }
        if count > 0 {
            smoothed.push((sample.timestamp_ms, sum / count as f64));
        }
    }
    smoothed
}

fn merge_close_contractions(
    events: Vec<ContractionEvent>,
    seconds: &[SecondSample],
) -> Vec<ContractionEvent> {
    let mut merged: Vec<ContractionEvent> = Vec::new();
    for event in events {
        if let Some(previous) = merged.last_mut() {
            let previous_end = parse_label_ms(seconds, &previous.end);
            let next_start = parse_label_ms(seconds, &event.start);
            if let (Some(previous_end), Some(next_start)) = (previous_end, next_start) {
                if next_start - previous_end <= 45_000 {
                    if event.peak_toco > previous.peak_toco {
                        previous.peak = event.peak;
                        previous.peak_toco = event.peak_toco;
                    }
                    previous.end = event.end;
                    previous.duration_seconds = seconds_between(
                        parse_label_ms(seconds, &previous.start).unwrap_or(previous_end),
                        parse_label_ms(seconds, &previous.end).unwrap_or(next_start),
                    );
                    continue;
                }
            }
        }
        merged.push(event);
    }
    merged
}

fn associate_decelerations_with_contractions(
    decelerations: &mut [DecelerationEvent],
    contractions: &[ContractionEvent],
) {
    for decel in decelerations {
        if decel.kind == DecelerationKind::Variable || decel.kind == DecelerationKind::Prolonged {
            continue;
        }
        let Some((decel_start, decel_nadir, decel_end)) = decel_times(decel) else {
            continue;
        };
        let mut best_peak = None;
        let mut best_distance = i64::MAX;
        for contraction in contractions {
            let Some(start) = timestamp_from_label(&contraction.start) else {
                continue;
            };
            let Some(peak) = timestamp_from_label(&contraction.peak) else {
                continue;
            };
            let Some(end) = timestamp_from_label(&contraction.end) else {
                continue;
            };
            let associated = decel_nadir >= start - 30_000 && decel_nadir <= end + 60_000
                || ranges_overlap(decel_start, decel_end, start, end);
            if associated {
                let distance = (decel_nadir - peak).abs();
                if distance < best_distance {
                    best_distance = distance;
                    best_peak = Some(peak);
                }
            }
        }
        if let Some(peak) = best_peak {
            if (decel_nadir - peak).abs() <= EARLY_DECELERATION_PEAK_TOLERANCE_MS {
                decel.kind = DecelerationKind::Early;
            } else if decel_nadir > peak + LATE_DECELERATION_DELAY_MS {
                decel.kind = DecelerationKind::Late;
            }
        }
    }
}

fn summarize_toco(
    seconds: &[SecondSample],
    contractions: Vec<ContractionEvent>,
    window_start_ms: i64,
    window_end_ms: i64,
) -> TocoSummary {
    let duration_minutes = seconds_between(window_start_ms, window_end_ms) / 60.0;
    let contractions_per_10_min = if duration_minutes > 0.0 {
        contractions.len() as f64 / duration_minutes * 10.0
    } else {
        0.0
    };
    let tachysystole = if window_end_ms - window_start_ms >= THIRTY_MIN_MS
        && seconds
            .iter()
            .filter(|sample| sample.toco.is_some())
            .count()
            >= 20 * 60
    {
        Some(contractions.len() > 15)
    } else {
        None
    };
    TocoSummary {
        contractions,
        contractions_per_10_min,
        tachysystole,
    }
}

fn classify_window(
    baseline_class: Option<BaselineClass>,
    variability_class: Option<VariabilityClass>,
    decelerations: &[DecelerationEvent],
    toco: &TocoSummary,
    window_start_ms: i64,
    window_end_ms: i64,
    reasons: &mut Vec<String>,
) -> CategoryClassification {
    let Some(baseline_class) = baseline_class else {
        reasons.push("cannot classify without a determinate baseline".to_string());
        return CategoryClassification::Unclassified;
    };
    let Some(variability_class) = variability_class else {
        reasons.push("cannot classify without determinate variability".to_string());
        return CategoryClassification::Unclassified;
    };

    let recurrent_late = recurrent_deceleration_kind(
        decelerations,
        toco,
        DecelerationKind::Late,
        window_start_ms,
        window_end_ms,
    );
    let recurrent_variable = recurrent_deceleration_kind(
        decelerations,
        toco,
        DecelerationKind::Variable,
        window_start_ms,
        window_end_ms,
    );
    let has_late_or_variable = decelerations.iter().any(|event| {
        matches!(
            event.kind,
            DecelerationKind::Late | DecelerationKind::Variable
        )
    });

    if variability_class == VariabilityClass::Absent
        && (baseline_class == BaselineClass::Bradycardia || recurrent_late || recurrent_variable)
    {
        reasons.push(
            "absent variability with bradycardia or recurrent late/variable decelerations"
                .to_string(),
        );
        return CategoryClassification::CategoryIII;
    }

    if baseline_class == BaselineClass::Normal
        && variability_class == VariabilityClass::Moderate
        && !has_late_or_variable
        && decelerations
            .iter()
            .all(|event| event.kind != DecelerationKind::Prolonged)
    {
        reasons.push(
            "normal baseline, moderate variability, and no detected late or variable decelerations"
                .to_string(),
        );
        return CategoryClassification::CategoryI;
    }

    match baseline_class {
        BaselineClass::Bradycardia => {
            reasons.push("bradycardia not accompanied by category-III criteria".to_string())
        }
        BaselineClass::Tachycardia => reasons.push("tachycardia".to_string()),
        BaselineClass::Normal => {}
    }
    match variability_class {
        VariabilityClass::Absent => {
            reasons.push("absent variability without recurrent decelerations".to_string())
        }
        VariabilityClass::Minimal => reasons.push("minimal variability".to_string()),
        VariabilityClass::Marked => reasons.push("marked variability".to_string()),
        VariabilityClass::Moderate => {}
    }
    for event in decelerations {
        match event.kind {
            DecelerationKind::Prolonged => reasons.push("prolonged deceleration".to_string()),
            DecelerationKind::Variable => reasons.push("variable deceleration".to_string()),
            DecelerationKind::Late => reasons.push("late deceleration".to_string()),
            DecelerationKind::GradualUnclassified => reasons.push(
                "gradual deceleration not classifiable without clearer TOCO association"
                    .to_string(),
            ),
            DecelerationKind::Early => {}
        }
    }
    reasons.sort();
    reasons.dedup();
    CategoryClassification::CategoryII
}

fn find_risk_features(
    seconds: &[SecondSample],
    baseline_bpm: Option<i32>,
    baseline_class: Option<BaselineClass>,
    variability_bpm: Option<f64>,
    variability_class: Option<VariabilityClass>,
    accelerations: &[AccelerationEvent],
    decelerations: &[DecelerationEvent],
    toco: &TocoSummary,
    window_start_ms: i64,
    window_end_ms: i64,
    high_risk_features: &mut Vec<String>,
    protective_features: &mut Vec<String>,
) {
    if variability_class == Some(VariabilityClass::Absent) {
        high_risk_features.push("absent variability".to_string());
    }
    if variability_class == Some(VariabilityClass::Marked)
        && variability_bpm.is_some_and(|value| value > CONCERNING_MARKED_VARIABILITY_BPM)
    {
        high_risk_features.push("marked variability".to_string());
    }
    if persistent_minimal_variability(seconds, baseline_bpm, window_start_ms, window_end_ms) {
        high_risk_features
            .push("persistent minimal variability for at least 20 minutes".to_string());
    }
    if baseline_class == Some(BaselineClass::Tachycardia)
        && baseline_changed_from_normal(seconds, window_start_ms, window_end_ms)
    {
        high_risk_features.push("baseline changed from normal to tachycardia".to_string());
    }
    let recurrent_late = recurrent_deceleration_kind(
        decelerations,
        toco,
        DecelerationKind::Late,
        window_start_ms,
        window_end_ms,
    );
    let recurrent_variable = recurrent_deceleration_kind(
        decelerations,
        toco,
        DecelerationKind::Variable,
        window_start_ms,
        window_end_ms,
    );
    if recurrent_late {
        high_risk_features.push("recurrent late decelerations".to_string());
    }
    if recurrent_variable {
        high_risk_features.push("recurrent variable decelerations".to_string());
    }
    if has_gradual_deceleration_with_low_variability(decelerations, variability_class) {
        high_risk_features
            .push("gradual deceleration with absent or minimal variability".to_string());
    }
    if has_severe_variable_deceleration(decelerations) {
        high_risk_features.push("severe variable deceleration".to_string());
    }
    if has_deep_deceleration(decelerations) {
        high_risk_features.push("deep deceleration nadir below 80 bpm".to_string());
    }
    if has_concerning_deceleration_burden(decelerations, recurrent_late || recurrent_variable) {
        high_risk_features.push("concerning deceleration burden".to_string());
    }
    let prolonged_count = decelerations
        .iter()
        .filter(|event| event.kind == DecelerationKind::Prolonged)
        .count();
    if prolonged_count > 1 {
        high_risk_features.push("more than one prolonged deceleration".to_string());
    }

    if variability_class == Some(VariabilityClass::Moderate) {
        protective_features.push("moderate variability".to_string());
    }
    if !accelerations.is_empty() {
        protective_features.push("accelerations present".to_string());
    }
}

fn choose_alert_level(
    classification: CategoryClassification,
    high_risk_features: &[String],
    protective_features: &[String],
    reasons: &[String],
    features: &NumericFeatures,
    data_quality: &DataQuality,
) -> AlertLevel {
    if data_quality.fetal_usable_ratio < 0.50 {
        return AlertLevel::DataQuality;
    }
    if classification == CategoryClassification::Unclassified {
        return AlertLevel::DataQuality;
    }
    if classification == CategoryClassification::CategoryIII {
        return AlertLevel::Critical;
    }
    if should_escalate_to_urgent_review(high_risk_features, features) {
        return AlertLevel::UrgentReview;
    }
    if data_quality.fetal_usable_ratio < MARGINAL_FETAL_USABLE_RATIO
        && high_risk_features.is_empty()
    {
        return AlertLevel::DataQuality;
    }
    if !high_risk_features.is_empty() {
        return AlertLevel::Warning;
    }
    if classification == CategoryClassification::CategoryII
        && !is_low_concern_category_ii(reasons, protective_features)
    {
        return AlertLevel::Warning;
    }
    AlertLevel::None
}

fn should_escalate_to_urgent_review(
    high_risk_features: &[String],
    features: &NumericFeatures,
) -> bool {
    let has_feature = |target: &str| high_risk_features.iter().any(|feature| feature == target);
    if has_feature("absent variability")
        || has_feature("persistent minimal variability for at least 20 minutes")
        || has_feature("recurrent late decelerations")
        || has_feature("gradual deceleration with absent or minimal variability")
        || has_feature("tachysystole with high-risk Category II features")
        || has_feature("more than one prolonged deceleration")
    {
        return true;
    }

    let deep_or_high_burden = features
        .deepest_deceleration_nadir_bpm
        .is_some_and(|nadir| nadir < DEEP_DECELERATION_NADIR_BPM)
        || features.total_deceleration_seconds >= DECELERATION_BURDEN_SECONDS;
    has_feature("recurrent variable decelerations") && deep_or_high_burden
}

fn is_low_concern_category_ii(reasons: &[String], protective_features: &[String]) -> bool {
    if has_moderate_variability_protection(protective_features) {
        return true;
    }
    if is_marked_variability_only(reasons) {
        return true;
    }

    has_acceleration_protection(protective_features)
        && reasons.iter().all(|reason| {
            matches!(
                reason.as_str(),
                "marked variability"
                    | "variable deceleration"
                    | "gradual deceleration not classifiable without clearer TOCO association"
            )
        })
}

fn is_marked_variability_only(reasons: &[String]) -> bool {
    reasons.len() == 1 && reasons[0] == "marked variability"
}

fn has_moderate_variability_protection(protective_features: &[String]) -> bool {
    protective_features
        .iter()
        .any(|feature| feature == "moderate variability")
}

fn has_acceleration_protection(protective_features: &[String]) -> bool {
    protective_features
        .iter()
        .any(|feature| feature == "accelerations present")
}

fn has_deep_deceleration(decelerations: &[DecelerationEvent]) -> bool {
    decelerations
        .iter()
        .any(|event| event.nadir_bpm < DEEP_DECELERATION_NADIR_BPM)
}

fn has_concerning_deceleration_burden(
    decelerations: &[DecelerationEvent],
    recurrent_deceleration: bool,
) -> bool {
    let total_seconds = total_deceleration_seconds(decelerations);
    if total_seconds >= STANDALONE_DECELERATION_BURDEN_SECONDS {
        return true;
    }
    total_seconds >= DECELERATION_BURDEN_SECONDS
        && (recurrent_deceleration
            || decelerations
                .iter()
                .any(|event| event.nadir_bpm < CONCERNING_DECELERATION_NADIR_BPM)
            || decelerations
                .iter()
                .any(|event| event.depth_bpm >= CONCERNING_DECELERATION_DEPTH_BPM))
}

fn total_deceleration_seconds(decelerations: &[DecelerationEvent]) -> f64 {
    decelerations
        .iter()
        .map(|event| event.duration_seconds)
        .sum()
}

fn has_severe_variable_deceleration(decelerations: &[DecelerationEvent]) -> bool {
    decelerations.iter().any(|event| {
        event.kind == DecelerationKind::Variable
            && (event.nadir_bpm < DEEP_DECELERATION_NADIR_BPM
                || event.depth_bpm >= SEVERE_VARIABLE_DEPTH_BPM
                || event.duration_seconds >= SEVERE_VARIABLE_DURATION_SECONDS)
    })
}

fn has_gradual_deceleration_with_low_variability(
    decelerations: &[DecelerationEvent],
    variability_class: Option<VariabilityClass>,
) -> bool {
    matches!(
        variability_class,
        Some(VariabilityClass::Absent | VariabilityClass::Minimal)
    ) && decelerations
        .iter()
        .any(|event| event.kind == DecelerationKind::GradualUnclassified)
}

fn apply_tachysystole_context(
    classification: CategoryClassification,
    toco: &TocoSummary,
    reasons: &mut Vec<String>,
    high_risk_features: &mut Vec<String>,
) {
    if toco.tachysystole != Some(true) {
        return;
    }

    reasons.push("tachysystole".to_string());
    if classification == CategoryClassification::CategoryII && !high_risk_features.is_empty() {
        high_risk_features.push("tachysystole with high-risk Category II features".to_string());
    } else {
        high_risk_features.push("tachysystole".to_string());
    }
}

fn sort_and_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn calculate_limitations(
    duration_seconds: f64,
    data_quality: &DataQuality,
    window_start_ms: i64,
    window_end_ms: i64,
    toco: &TocoSummary,
) -> Vec<String> {
    let mut limitations = Vec::new();
    if duration_seconds < 10.0 * 60.0 {
        limitations.push(
            "less than 10 minutes: baseline and variability assessment is incomplete".to_string(),
        );
    }
    if duration_seconds < 20.0 * 60.0 {
        limitations.push(
            "less than 20 minutes: recurrent deceleration assessment is incomplete".to_string(),
        );
    } else if toco.contractions.len() < 2 {
        limitations.push(
            "fewer than two detected contractions: recurrent deceleration assessment is limited"
                .to_string(),
        );
    }
    if duration_seconds < 30.0 * 60.0 {
        limitations.push("less than 30 minutes: tachysystole assessment is incomplete".to_string());
    }
    if data_quality.toco_usable_ratio < 0.50 {
        limitations.push(
            "TOCO unavailable or sparse: late/early timing and contraction recurrence are limited"
                .to_string(),
        );
    }
    if window_end_ms - window_start_ms < THIRTY_MIN_MS && toco.tachysystole.is_none() {
        limitations.push("tachysystole requires a 30-minute contraction average".to_string());
    }
    limitations
}

fn persistent_minimal_variability(
    seconds: &[SecondSample],
    baseline_bpm: Option<i32>,
    window_start_ms: i64,
    window_end_ms: i64,
) -> bool {
    let Some(baseline) = baseline_bpm else {
        return false;
    };
    if window_end_ms - window_start_ms < TWENTY_MIN_MS {
        return false;
    }
    let first_start = window_end_ms - TWENTY_MIN_MS;
    let first_end = window_end_ms - TEN_MIN_MS;
    let second_start = window_end_ms - TEN_MIN_MS;
    let first = estimate_variability(seconds, baseline as f64, first_start, first_end)
        .map(classify_variability);
    let second = estimate_variability(seconds, baseline as f64, second_start, window_end_ms)
        .map(classify_variability);
    first == Some(VariabilityClass::Minimal) && second == Some(VariabilityClass::Minimal)
}

fn baseline_changed_from_normal(
    seconds: &[SecondSample],
    window_start_ms: i64,
    window_end_ms: i64,
) -> bool {
    if window_end_ms - window_start_ms < TWENTY_MIN_MS {
        return false;
    }
    let early = estimate_baseline(
        seconds,
        window_end_ms - TWENTY_MIN_MS,
        window_end_ms - TEN_MIN_MS,
    )
    .map(classify_baseline);
    let current = estimate_baseline(seconds, window_end_ms - TEN_MIN_MS, window_end_ms)
        .map(classify_baseline);
    early == Some(BaselineClass::Normal) && current == Some(BaselineClass::Tachycardia)
}

fn recurrent_deceleration_kind(
    decelerations: &[DecelerationEvent],
    toco: &TocoSummary,
    kind: DecelerationKind,
    window_start_ms: i64,
    window_end_ms: i64,
) -> bool {
    if window_end_ms - window_start_ms < TWENTY_MIN_MS || toco.contractions.len() < 3 {
        return false;
    }
    let matching_decelerations = decelerations
        .iter()
        .filter(|decel| decel.kind == kind)
        .count();
    if matching_decelerations < 2 {
        return false;
    }
    let contraction_ids_with_kind: HashSet<usize> = toco
        .contractions
        .iter()
        .enumerate()
        .filter_map(|(idx, contraction)| {
            let start = timestamp_from_label(&contraction.start)?;
            let end = timestamp_from_label(&contraction.end)?;
            let found = decelerations.iter().any(|decel| {
                decel.kind == kind
                    && decel_times(decel).is_some_and(|(decel_start, _, decel_end)| {
                        ranges_overlap(decel_start, decel_end, start - 30_000, end + 60_000)
                    })
            });
            found.then_some(idx)
        })
        .collect();
    contraction_ids_with_kind.len() * 2 >= toco.contractions.len()
}

fn valid_heart_rate(value: f64) -> bool {
    (30.0..=240.0).contains(&value)
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percentile(sorted_values: &[f64], q: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_values.len() - 1) as f64 * q).round() as usize;
    sorted_values[idx.min(sorted_values.len() - 1)]
}

type ValueSummary = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

fn summarize_values(values: &mut [f64]) -> ValueSummary {
    if values.is_empty() {
        return (None, None, None, None, None, None, None);
    }
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    (
        values.first().copied(),
        Some(percentile(values, 0.05)),
        Some(mean),
        Some(percentile(values, 0.50)),
        Some(percentile(values, 0.95)),
        values.last().copied(),
        Some(variance.sqrt()),
    )
}

fn label_for_time(seconds: &[SecondSample], timestamp_ms: i64) -> String {
    seconds
        .iter()
        .min_by_key(|sample| (sample.timestamp_ms - timestamp_ms).abs())
        .map(|sample| sample.timestamp.clone())
        .unwrap_or_else(|| format!("{}ms", timestamp_ms))
}

fn timestamp_from_label(label: &str) -> Option<i64> {
    super::time::parse_monitor_timestamp(label).ok()
}

fn parse_label_ms(seconds: &[SecondSample], label: &str) -> Option<i64> {
    if let Some(sample) = seconds.iter().find(|sample| sample.timestamp == label) {
        Some(sample.timestamp_ms)
    } else {
        timestamp_from_label(label)
    }
}

fn decel_times(decel: &DecelerationEvent) -> Option<(i64, i64, i64)> {
    let start = timestamp_from_label(&decel.start)?;
    let end = timestamp_from_label(&decel.end)?;
    let nadir = start + (decel.onset_to_nadir_seconds * 1_000.0).round() as i64;
    Some((start, nadir, end))
}

fn ranges_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start <= b_end && b_start <= a_end
}

pub fn report_as_json(report: &AnalysisReport) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_line(
        &mut out,
        1,
        "channel",
        report.config.fetal_channel.as_str(),
        true,
    );
    push_json_line(
        &mut out,
        1,
        "analysis_mode",
        if report.config.window_minutes.is_some() {
            "rolling"
        } else {
            "chunk"
        },
        true,
    );
    push_json_option_u32(
        &mut out,
        1,
        "requested_window_minutes",
        report.config.window_minutes,
        true,
    );
    push_json_u32(
        &mut out,
        1,
        "step_seconds",
        report.config.step_seconds,
        true,
    );
    out.push_str("  \"input\": {\n");
    push_json_usize(&mut out, 2, "rows", report.input.rows, true);
    push_json_option(
        &mut out,
        2,
        "start_timestamp",
        report.input.start_timestamp.as_deref(),
        true,
    );
    push_json_option(
        &mut out,
        2,
        "end_timestamp",
        report.input.end_timestamp.as_deref(),
        true,
    );
    push_json_number(
        &mut out,
        2,
        "duration_seconds",
        report.input.duration_seconds,
        true,
    );
    push_json_usize(
        &mut out,
        2,
        "out_of_order_rows",
        report.input.out_of_order_rows,
        true,
    );
    push_json_usize(
        &mut out,
        2,
        "duplicate_timestamps",
        report.input.duplicate_timestamps,
        false,
    );
    out.push_str("  },\n");
    out.push_str("  \"windows\": [\n");
    for (idx, window) in report.windows.iter().enumerate() {
        out.push_str("    {\n");
        push_json_line(&mut out, 3, "window_start", &window.window_start, true);
        push_json_line(&mut out, 3, "window_end", &window.window_end, true);
        push_json_number(
            &mut out,
            3,
            "duration_seconds",
            window.duration_seconds,
            true,
        );
        push_json_line(
            &mut out,
            3,
            "classification",
            window.classification.as_str(),
            true,
        );
        push_json_line(
            &mut out,
            3,
            "alert_level",
            window.alert_level.as_str(),
            true,
        );
        push_json_option_i32(&mut out, 3, "baseline_bpm", window.baseline_bpm, true);
        push_json_option(
            &mut out,
            3,
            "baseline_class",
            window.baseline_class.map(BaselineClass::as_str),
            true,
        );
        push_json_option_number(&mut out, 3, "variability_bpm", window.variability_bpm, true);
        push_json_option(
            &mut out,
            3,
            "variability_class",
            window.variability_class.map(VariabilityClass::as_str),
            true,
        );
        out.push_str("      \"features\": {\n");
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_min_bpm",
            window.features.fetal_hr_min_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_p05_bpm",
            window.features.fetal_hr_p05_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_mean_bpm",
            window.features.fetal_hr_mean_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_median_bpm",
            window.features.fetal_hr_median_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_p95_bpm",
            window.features.fetal_hr_p95_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_max_bpm",
            window.features.fetal_hr_max_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_hr_std_dev_bpm",
            window.features.fetal_hr_std_dev_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "baseline_delta_mean_bpm",
            window.features.baseline_delta_mean_bpm,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "fetal_hr_seconds_below_110",
            window.features.fetal_hr_seconds_below_110,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "fetal_hr_seconds_110_to_160",
            window.features.fetal_hr_seconds_110_to_160,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "fetal_hr_seconds_above_160",
            window.features.fetal_hr_seconds_above_160,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "fetal_hr_percent_below_110",
            window.features.fetal_hr_percent_below_110,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "fetal_hr_percent_110_to_160",
            window.features.fetal_hr_percent_110_to_160,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "fetal_hr_percent_above_160",
            window.features.fetal_hr_percent_above_160,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "acceleration_count",
            window.features.acceleration_count,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "deceleration_count",
            window.features.deceleration_count,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "prolonged_deceleration_count",
            window.features.prolonged_deceleration_count,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "total_deceleration_seconds",
            window.features.total_deceleration_seconds,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "deepest_deceleration_nadir_bpm",
            window.features.deepest_deceleration_nadir_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "max_deceleration_depth_bpm",
            window.features.max_deceleration_depth_bpm,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "contraction_count",
            window.features.contraction_count,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "contractions_per_10_min",
            window.features.contractions_per_10_min,
            true,
        );
        push_json_option_number(&mut out, 4, "toco_mean", window.features.toco_mean, true);
        push_json_option_number(&mut out, 4, "toco_max", window.features.toco_max, true);
        push_json_option_number(
            &mut out,
            4,
            "maternal_hr_mean_bpm",
            window.features.maternal_hr_mean_bpm,
            true,
        );
        push_json_option_number(
            &mut out,
            4,
            "fetal_maternal_mean_difference_bpm",
            window.features.fetal_maternal_mean_difference_bpm,
            false,
        );
        out.push_str("      },\n");
        out.push_str("      \"data_quality\": {\n");
        push_json_usize(
            &mut out,
            4,
            "expected_seconds",
            window.data_quality.expected_seconds,
            true,
        );
        push_json_usize(
            &mut out,
            4,
            "raw_samples",
            window.data_quality.raw_samples,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "fetal_usable_ratio",
            window.data_quality.fetal_usable_ratio,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "maternal_usable_ratio",
            window.data_quality.maternal_usable_ratio,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "toco_usable_ratio",
            window.data_quality.toco_usable_ratio,
            true,
        );
        push_json_number(
            &mut out,
            4,
            "suspected_maternal_capture_ratio",
            window.data_quality.suspected_maternal_capture_ratio,
            false,
        );
        out.push_str("      },\n");
        push_json_usize(
            &mut out,
            3,
            "contractions",
            window.toco.contractions.len(),
            true,
        );
        push_json_number(
            &mut out,
            3,
            "contractions_per_10_min",
            window.toco.contractions_per_10_min,
            true,
        );
        push_json_bool_option(&mut out, 3, "tachysystole", window.toco.tachysystole, true);
        push_string_array(&mut out, 3, "reasons", &window.reasons, true);
        push_string_array(
            &mut out,
            3,
            "high_risk_features",
            &window.high_risk_features,
            true,
        );
        push_string_array(
            &mut out,
            3,
            "protective_features",
            &window.protective_features,
            true,
        );
        push_string_array(&mut out, 3, "limitations", &window.limitations, false);
        out.push_str("    }");
        if idx + 1 != report.windows.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn push_json_line(out: &mut String, indent: usize, key: &str, value: &str, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("\"{key}\": \"{}\"", escape_json(value)));
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_option(out: &mut String, indent: usize, key: &str, value: Option<&str>, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    match value {
        Some(value) => out.push_str(&format!("\"{key}\": \"{}\"", escape_json(value))),
        None => out.push_str(&format!("\"{key}\": null")),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_number(out: &mut String, indent: usize, key: &str, value: f64, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("\"{key}\": {:.3}", value));
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_usize(out: &mut String, indent: usize, key: &str, value: usize, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("\"{key}\": {value}"));
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_u32(out: &mut String, indent: usize, key: &str, value: u32, comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("\"{key}\": {value}"));
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_option_u32(
    out: &mut String,
    indent: usize,
    key: &str,
    value: Option<u32>,
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    match value {
        Some(value) => out.push_str(&format!("\"{key}\": {value}")),
        None => out.push_str(&format!("\"{key}\": null")),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_option_number(
    out: &mut String,
    indent: usize,
    key: &str,
    value: Option<f64>,
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    match value {
        Some(value) => out.push_str(&format!("\"{key}\": {:.3}", value)),
        None => out.push_str(&format!("\"{key}\": null")),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_option_i32(
    out: &mut String,
    indent: usize,
    key: &str,
    value: Option<i32>,
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    match value {
        Some(value) => out.push_str(&format!("\"{key}\": {value}")),
        None => out.push_str(&format!("\"{key}\": null")),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_bool_option(
    out: &mut String,
    indent: usize,
    key: &str,
    value: Option<bool>,
    comma: bool,
) {
    out.push_str(&"  ".repeat(indent));
    match value {
        Some(value) => out.push_str(&format!("\"{key}\": {value}")),
        None => out.push_str(&format!("\"{key}\": null")),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_string_array(out: &mut String, indent: usize, key: &str, values: &[String], comma: bool) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("\"{key}\": ["));
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{}\"", escape_json(value)));
    }
    out.push(']');
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_quality() -> DataQuality {
        DataQuality {
            expected_seconds: 600,
            raw_samples: 600,
            fetal_usable_seconds: 570,
            fetal_usable_ratio: 0.95,
            maternal_usable_ratio: 0.80,
            toco_usable_ratio: 1.0,
            suspected_maternal_capture_ratio: 0.01,
        }
    }

    fn base_features() -> NumericFeatures {
        NumericFeatures {
            fetal_hr_min_bpm: None,
            fetal_hr_p05_bpm: None,
            fetal_hr_mean_bpm: None,
            fetal_hr_median_bpm: None,
            fetal_hr_p95_bpm: None,
            fetal_hr_max_bpm: None,
            fetal_hr_std_dev_bpm: None,
            baseline_delta_mean_bpm: None,
            fetal_hr_seconds_below_110: 0,
            fetal_hr_seconds_110_to_160: 0,
            fetal_hr_seconds_above_160: 0,
            fetal_hr_percent_below_110: 0.0,
            fetal_hr_percent_110_to_160: 0.0,
            fetal_hr_percent_above_160: 0.0,
            acceleration_count: 0,
            deceleration_count: 0,
            prolonged_deceleration_count: 0,
            total_deceleration_seconds: 0.0,
            deepest_deceleration_nadir_bpm: None,
            max_deceleration_depth_bpm: None,
            contraction_count: 0,
            contractions_per_10_min: 0.0,
            toco_mean: None,
            toco_max: None,
            maternal_hr_mean_bpm: None,
            fetal_maternal_mean_difference_bpm: None,
        }
    }

    fn empty_toco(tachysystole: Option<bool>) -> TocoSummary {
        TocoSummary {
            contractions: Vec::new(),
            contractions_per_10_min: 0.0,
            tachysystole,
        }
    }

    fn second_samples(values: &[f64]) -> Vec<SecondSample> {
        values
            .iter()
            .enumerate()
            .map(|(idx, value)| SecondSample {
                timestamp_ms: idx as i64 * 1_000,
                timestamp: format!("2026-01-01 00:00:{idx:02}.000"),
                fhr: Some(*value),
                hrm: None,
                toco: Some(10.0),
            })
            .collect()
    }

    fn timestamp_at(second: i64) -> String {
        format!("2026-01-01 00:{:02}:{:02}.000", second / 60, second % 60)
    }

    fn variable_deceleration(start_sec: i64, end_sec: i64) -> DecelerationEvent {
        DecelerationEvent {
            start: timestamp_at(start_sec),
            end: timestamp_at(end_sec),
            duration_seconds: (end_sec - start_sec) as f64,
            nadir_bpm: 90.0,
            depth_bpm: 40.0,
            onset_to_nadir_seconds: 10.0,
            kind: DecelerationKind::Variable,
        }
    }

    fn contraction(start_sec: i64, end_sec: i64) -> ContractionEvent {
        ContractionEvent {
            start: timestamp_at(start_sec),
            peak: timestamp_at((start_sec + end_sec) / 2),
            end: timestamp_at(end_sec),
            duration_seconds: (end_sec - start_sec) as f64,
            peak_toco: 60.0,
        }
    }

    #[test]
    fn moderate_category_ii_without_high_risk_stays_non_alerting() {
        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &[],
            &[
                "moderate variability".to_string(),
                "accelerations present".to_string(),
            ],
            &[],
            &base_features(),
            &good_quality(),
        );

        assert_eq!(alert, AlertLevel::None);
    }

    #[test]
    fn accelerations_do_not_suppress_tachycardia_warning() {
        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &[],
            &["accelerations present".to_string()],
            &["tachycardia".to_string()],
            &base_features(),
            &good_quality(),
        );

        assert_eq!(alert, AlertLevel::Warning);
    }

    #[test]
    fn marked_variability_only_stays_non_alerting() {
        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &[],
            &[],
            &["marked variability".to_string()],
            &base_features(),
            &good_quality(),
        );

        assert_eq!(alert, AlertLevel::None);
    }

    #[test]
    fn concerning_marked_variability_triggers_warning() {
        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &["marked variability".to_string()],
            &["accelerations present".to_string()],
            &["marked variability".to_string()],
            &base_features(),
            &good_quality(),
        );

        assert_eq!(alert, AlertLevel::Warning);
    }

    #[test]
    fn recurrent_variables_with_deep_deceleration_escalate_to_urgent_review() {
        let mut features = base_features();
        features.deepest_deceleration_nadir_bpm = Some(79.0);

        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &["recurrent variable decelerations".to_string()],
            &[
                "moderate variability".to_string(),
                "accelerations present".to_string(),
            ],
            &["variable deceleration".to_string()],
            &features,
            &good_quality(),
        );

        assert_eq!(alert, AlertLevel::UrgentReview);
    }

    #[test]
    fn tachysystole_with_high_risk_category_ii_escalates_to_urgent_review() {
        let mut reasons = Vec::new();
        let mut high_risk = vec!["marked variability".to_string()];
        apply_tachysystole_context(
            CategoryClassification::CategoryII,
            &empty_toco(Some(true)),
            &mut reasons,
            &mut high_risk,
        );

        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &high_risk,
            &["accelerations present".to_string()],
            &reasons,
            &base_features(),
            &good_quality(),
        );

        assert!(reasons.contains(&"tachysystole".to_string()));
        assert!(
            high_risk.contains(&"tachysystole with high-risk Category II features".to_string())
        );
        assert_eq!(alert, AlertLevel::UrgentReview);
    }

    #[test]
    fn tachysystole_alone_triggers_warning() {
        let mut reasons = Vec::new();
        let mut high_risk = Vec::new();
        apply_tachysystole_context(
            CategoryClassification::CategoryI,
            &empty_toco(Some(true)),
            &mut reasons,
            &mut high_risk,
        );

        let alert = choose_alert_level(
            CategoryClassification::CategoryI,
            &high_risk,
            &["moderate variability".to_string()],
            &reasons,
            &base_features(),
            &good_quality(),
        );

        assert_eq!(high_risk, vec!["tachysystole".to_string()]);
        assert_eq!(alert, AlertLevel::Warning);
    }

    #[test]
    fn marginal_signal_without_high_risk_returns_data_quality() {
        let mut quality = good_quality();
        quality.fetal_usable_ratio = 0.65;

        let alert = choose_alert_level(
            CategoryClassification::CategoryII,
            &[],
            &["moderate variability".to_string()],
            &["variable deceleration".to_string()],
            &base_features(),
            &quality,
        );

        assert_eq!(alert, AlertLevel::DataQuality);
    }

    #[test]
    fn recurrent_decelerations_need_enough_context_and_events() {
        let decelerations = vec![
            variable_deceleration(60, 90),
            variable_deceleration(180, 210),
        ];
        let two_contractions = TocoSummary {
            contractions: vec![contraction(50, 100), contraction(170, 220)],
            contractions_per_10_min: 1.0,
            tachysystole: None,
        };
        let three_contractions = TocoSummary {
            contractions: vec![
                contraction(50, 100),
                contraction(170, 220),
                contraction(300, 350),
            ],
            contractions_per_10_min: 1.5,
            tachysystole: None,
        };

        assert!(!recurrent_deceleration_kind(
            &decelerations,
            &two_contractions,
            DecelerationKind::Variable,
            0,
            TWENTY_MIN_MS,
        ));
        assert!(!recurrent_deceleration_kind(
            &decelerations,
            &three_contractions,
            DecelerationKind::Variable,
            0,
            TWENTY_MIN_MS - 1,
        ));
        assert!(recurrent_deceleration_kind(
            &decelerations,
            &three_contractions,
            DecelerationKind::Variable,
            0,
            TWENTY_MIN_MS,
        ));
    }

    #[test]
    fn preterm_acceleration_uses_ten_by_ten_threshold() {
        let mut values = vec![130.0; 40];
        for value in values.iter_mut().take(20).skip(10) {
            *value = 140.0;
        }
        let seconds = second_samples(&values);

        let term = detect_accelerations(&seconds, 130.0, 0, 39_000, Some(32));
        let preterm = detect_accelerations(&seconds, 130.0, 0, 39_000, Some(31));

        assert!(term.is_empty());
        assert_eq!(preterm.len(), 1);
    }

    #[test]
    fn post_peak_gradual_deceleration_is_late_outside_small_tolerance() {
        let mut decelerations = vec![DecelerationEvent {
            start: "2026-01-01 00:00:00.000".to_string(),
            end: "2026-01-01 00:01:00.000".to_string(),
            duration_seconds: 60.0,
            nadir_bpm: 90.0,
            depth_bpm: 40.0,
            onset_to_nadir_seconds: 24.0,
            kind: DecelerationKind::GradualUnclassified,
        }];
        let contractions = vec![ContractionEvent {
            start: "2026-01-01 00:00:00.000".to_string(),
            peak: "2026-01-01 00:00:10.000".to_string(),
            end: "2026-01-01 00:00:30.000".to_string(),
            duration_seconds: 30.0,
            peak_toco: 60.0,
        }];

        associate_decelerations_with_contractions(&mut decelerations, &contractions);

        assert_eq!(decelerations[0].kind, DecelerationKind::Late);
    }

    #[test]
    fn gradual_deceleration_with_low_variability_is_high_risk() {
        let decelerations = vec![DecelerationEvent {
            start: "2026-01-01 00:00:00.000".to_string(),
            end: "2026-01-01 00:01:00.000".to_string(),
            duration_seconds: 60.0,
            nadir_bpm: 90.0,
            depth_bpm: 40.0,
            onset_to_nadir_seconds: 35.0,
            kind: DecelerationKind::GradualUnclassified,
        }];

        assert!(has_gradual_deceleration_with_low_variability(
            &decelerations,
            Some(VariabilityClass::Minimal)
        ));
        assert!(!has_gradual_deceleration_with_low_variability(
            &decelerations,
            Some(VariabilityClass::Moderate)
        ));
    }
}
