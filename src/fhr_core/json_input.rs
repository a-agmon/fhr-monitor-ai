use std::collections::HashSet;

use serde::Deserialize;

use super::model::{AnalysisConfig, FetalChannel, InputData, MonitorSample};
use super::time::parse_monitor_timestamp;

#[derive(Deserialize)]
struct AnalysisRequest {
    episode_id: String,
    sent_at: String,
    #[serde(default)]
    analysis_options: Option<AnalysisOptions>,
    samples: Vec<RequestSample>,
    #[serde(default)]
    metadata: Option<RequestMetadata>,
}

#[derive(Deserialize)]
struct AnalysisOptions {
    #[serde(default)]
    fetal_channel: Option<String>,
    #[serde(default)]
    max_analysis_minutes: Option<u32>,
}

#[derive(Deserialize)]
struct RequestMetadata {
    #[serde(default)]
    gestational_age_weeks: Option<u8>,
}

#[derive(Deserialize)]
struct RequestSample {
    t: String,
    #[serde(default)]
    hr1: Option<f64>,
    #[serde(default)]
    hr2: Option<f64>,
    #[serde(default)]
    hr3: Option<f64>,
    #[serde(default)]
    hrm: Option<f64>,
    #[serde(default)]
    toco: Option<f64>,
}

pub fn read_analysis_request_json(content: &str) -> Result<(InputData, AnalysisConfig), String> {
    let request: AnalysisRequest = serde_json::from_str(content)
        .map_err(|err| format!("invalid analysis request JSON: {err}"))?;
    if request.episode_id.trim().is_empty() {
        return Err("analysis request episode_id must not be empty".to_string());
    }
    if request.sent_at.trim().is_empty() {
        return Err("analysis request sent_at must not be empty".to_string());
    }
    if request.samples.is_empty() {
        return Err("analysis request samples must not be empty".to_string());
    }

    let mut config = AnalysisConfig::default();
    if let Some(options) = request.analysis_options {
        if let Some(channel) = options.fetal_channel {
            config.fetal_channel = FetalChannel::parse(&channel)?;
        }
        if let Some(max_minutes) = options.max_analysis_minutes {
            validate_max_analysis_minutes(max_minutes)?;
        }
    }
    if let Some(metadata) = request.metadata {
        config.gestational_age_weeks = metadata.gestational_age_weeks;
    }

    let mut samples = Vec::with_capacity(request.samples.len());
    let mut previous_ts = None;
    let mut out_of_order_rows = 0;
    for (idx, sample) in request.samples.into_iter().enumerate() {
        let timestamp_ms = parse_monitor_timestamp(&sample.t)
            .map_err(|err| format!("sample {} has invalid t: {err}", idx + 1))?;
        if let Some(previous) = previous_ts {
            if timestamp_ms < previous {
                out_of_order_rows += 1;
            }
        }
        previous_ts = Some(timestamp_ms);
        samples.push(MonitorSample {
            timestamp_ms,
            timestamp: normalize_timestamp_label(&sample.t),
            hr1: sample.hr1,
            hr2: sample.hr2,
            hr3: sample.hr3,
            hrm: sample.hrm,
            toco: sample.toco,
        });
    }

    samples.sort_by_key(|sample| sample.timestamp_ms);
    let duplicate_timestamps = samples
        .windows(2)
        .filter(|pair| pair[0].timestamp_ms == pair[1].timestamp_ms)
        .count();
    let columns = infer_columns(&samples);

    Ok((
        InputData {
            samples,
            columns,
            out_of_order_rows,
            duplicate_timestamps,
        },
        config,
    ))
}

fn validate_max_analysis_minutes(max_minutes: u32) -> Result<(), String> {
    if !(10..=30).contains(&max_minutes) {
        return Err("analysis_options.max_analysis_minutes must be between 10 and 30".to_string());
    }
    Ok(())
}

fn normalize_timestamp_label(value: &str) -> String {
    value.trim().trim_end_matches('Z').replace('T', " ")
}

fn infer_columns(samples: &[MonitorSample]) -> Vec<String> {
    let mut columns = vec!["Date".to_string()];
    let mut seen = HashSet::new();
    for sample in samples {
        if sample.hr1.is_some() && seen.insert("HR1") {
            columns.push("HR1".to_string());
        }
        if sample.hr2.is_some() && seen.insert("HR2") {
            columns.push("HR2".to_string());
        }
        if sample.hr3.is_some() && seen.insert("HR3") {
            columns.push("HR3".to_string());
        }
        if sample.hrm.is_some() && seen.insert("HRM") {
            columns.push("HRM".to_string());
        }
        if sample.toco.is_some() && seen.insert("TOCO") {
            columns.push("TOCO".to_string());
        }
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_analysis_request_json() {
        let content = r#"
{
  "episode_id": "episode-1",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "analysis_options": {
    "fetal_channel": "HR2",
    "max_analysis_minutes": 30
  },
  "metadata": {
    "gestational_age_weeks": 31
  },
  "samples": [
    {"t": "2026-05-12T11:52:36.052Z", "hr2": 132, "hrm": 99, "toco": 14},
    {"t": "2026-05-12T11:52:35.052Z", "hr2": 130, "hrm": 98, "toco": 12}
  ]
}
"#;

        let (input, config) = read_analysis_request_json(content).expect("valid request");

        assert_eq!(config.fetal_channel, FetalChannel::Hr2);
        assert_eq!(config.gestational_age_weeks, Some(31));
        assert_eq!(input.samples.len(), 2);
        assert_eq!(input.out_of_order_rows, 1);
        assert_eq!(input.samples[0].hr2, Some(130.0));
    }

    #[test]
    fn rejects_empty_sample_list() {
        let content = r#"{
  "episode_id": "episode-1",
  "sent_at": "2026-05-12T12:22:35.052Z",
  "samples": []
}"#;

        let err = read_analysis_request_json(content).expect_err("invalid request");

        assert!(err.contains("samples must not be empty"));
    }
}
