use std::collections::{BTreeMap, HashSet};

use super::model::*;
use super::time::{seconds_between, whole_seconds_between};

const TEN_MIN_MS: i64 = 10 * 60 * 1_000;
const TWENTY_MIN_MS: i64 = 20 * 60 * 1_000;
const THIRTY_MIN_MS: i64 = 30 * 60 * 1_000;

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
    let window_ms = config.window_minutes as i64 * 60 * 1_000;
    let step_ms = config.step_seconds.max(1) as i64 * 1_000;

    let mut windows = Vec::new();
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

    let current_start = (window_end_ms - TEN_MIN_MS).max(window_start_ms);
    let baseline_bpm = estimate_baseline(&seconds, current_start, window_end_ms);
    let baseline_class = baseline_bpm.map(classify_baseline);
    let variability_bpm = baseline_bpm.and_then(|baseline| {
        estimate_variability(&seconds, baseline as f64, current_start, window_end_ms)
    });
    let variability_class = variability_bpm.map(classify_variability);

    let eval_start = (window_end_ms - TWENTY_MIN_MS).max(window_start_ms);
    let accelerations = baseline_bpm
        .map(|baseline| detect_accelerations(&seconds, baseline as f64, eval_start, window_end_ms))
        .unwrap_or_default();
    let mut decelerations = baseline_bpm
        .map(|baseline| detect_decelerations(&seconds, baseline as f64, eval_start, window_end_ms))
        .unwrap_or_default();
    let contractions = detect_contractions(&seconds, eval_start, window_end_ms);
    associate_decelerations_with_contractions(&mut decelerations, &contractions);

    let toco = summarize_toco(&seconds, contractions, window_start_ms, window_end_ms);
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
        &mut reasons,
    );
    find_risk_features(
        &seconds,
        baseline_bpm,
        baseline_class,
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

    let alert_level = choose_alert_level(
        classification,
        &high_risk_features,
        &protective_features,
        &data_quality,
    );

    WindowAnalysis {
        window_start,
        window_end,
        duration_seconds,
        data_quality,
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
) -> Vec<AccelerationEvent> {
    let threshold = baseline + 15.0;
    detect_segments(seconds, start_ms, end_ms, |sample| {
        sample.fhr.is_some_and(|value| value >= threshold)
    })
    .into_iter()
    .filter_map(|segment| {
        let duration = seconds_between(segment.start_ms, segment.end_ms);
        if !(15.0..120.0).contains(&duration) {
            return None;
        }
        let peak = seconds
            .iter()
            .filter(|sample| {
                sample.timestamp_ms >= segment.start_ms && sample.timestamp_ms <= segment.end_ms
            })
            .filter_map(|sample| sample.fhr)
            .max_by(f64::total_cmp)?;
        Some(AccelerationEvent {
            start: label_for_time(seconds, segment.start_ms),
            end: label_for_time(seconds, segment.end_ms),
            duration_seconds: duration,
            peak_bpm: peak,
        })
    })
    .collect()
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
            if (decel_nadir - peak).abs() <= 15_000 {
                decel.kind = DecelerationKind::Early;
            } else if decel_nadir > peak + 5_000 {
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

    let recurrent_late = recurrent_deceleration_kind(decelerations, toco, DecelerationKind::Late);
    let recurrent_variable =
        recurrent_deceleration_kind(decelerations, toco, DecelerationKind::Variable);
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
    if persistent_minimal_variability(seconds, baseline_bpm, window_start_ms, window_end_ms) {
        high_risk_features
            .push("persistent minimal variability for at least 20 minutes".to_string());
    }
    if baseline_class == Some(BaselineClass::Tachycardia)
        && baseline_changed_from_normal(seconds, window_start_ms, window_end_ms)
    {
        high_risk_features.push("baseline changed from normal to tachycardia".to_string());
    }
    if recurrent_deceleration_kind(decelerations, toco, DecelerationKind::Late) {
        high_risk_features.push("recurrent late decelerations".to_string());
    }
    if recurrent_deceleration_kind(decelerations, toco, DecelerationKind::Variable) {
        high_risk_features.push("recurrent variable decelerations".to_string());
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
    if !high_risk_features.is_empty() {
        return AlertLevel::Warning;
    }
    if classification == CategoryClassification::CategoryII && protective_features.is_empty() {
        return AlertLevel::Info;
    }
    AlertLevel::None
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
) -> bool {
    if toco.contractions.len() < 2 {
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
    push_json_u32(
        &mut out,
        1,
        "window_minutes",
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
