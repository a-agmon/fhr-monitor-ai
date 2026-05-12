use std::fs;
use std::path::Path;

use super::model::{InputData, MonitorSample};
use super::time::parse_monitor_timestamp;

pub fn read_monitor_csv(path: impl AsRef<Path>) -> Result<InputData, String> {
    let content = fs::read_to_string(path.as_ref())
        .map_err(|err| format!("failed reading {}: {err}", path.as_ref().display()))?;
    let mut lines = content.lines();
    let header = lines
        .next()
        .ok_or_else(|| "CSV is empty; expected a header row".to_string())?;
    let columns: Vec<String> = header.split(',').map(|v| v.trim().to_string()).collect();

    let date_idx = find_column(&columns, "Date")?;
    let hr1_idx = find_optional_column(&columns, "HR1");
    let hr2_idx = find_optional_column(&columns, "HR2");
    let hr3_idx = find_optional_column(&columns, "HR3");
    let hrm_idx = find_optional_column(&columns, "HRM");
    let toco_idx = find_optional_column(&columns, "TOCO");

    let mut samples = Vec::new();
    let mut previous_ts = None;
    let mut out_of_order_rows = 0;

    // Real device exports are not perfectly ordered. Preserve disorder metrics
    // for metadata, then sort before analysis so feature extraction is stable.
    for (line_idx, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let timestamp = fields
            .get(date_idx)
            .ok_or_else(|| format!("row {} is missing Date", line_idx + 2))?
            .trim()
            .to_string();
        let timestamp_ms = parse_monitor_timestamp(&timestamp)
            .map_err(|err| format!("row {} has invalid Date: {err}", line_idx + 2))?;
        if let Some(previous) = previous_ts {
            if timestamp_ms < previous {
                out_of_order_rows += 1;
            }
        }
        previous_ts = Some(timestamp_ms);

        samples.push(MonitorSample {
            timestamp_ms,
            timestamp,
            hr1: parse_optional_number(&fields, hr1_idx),
            hr2: parse_optional_number(&fields, hr2_idx),
            hr3: parse_optional_number(&fields, hr3_idx),
            hrm: parse_optional_number(&fields, hrm_idx),
            toco: parse_optional_number(&fields, toco_idx),
        });
    }

    samples.sort_by_key(|sample| sample.timestamp_ms);
    let duplicate_timestamps = samples
        .windows(2)
        .filter(|pair| pair[0].timestamp_ms == pair[1].timestamp_ms)
        .count();

    Ok(InputData {
        samples,
        columns,
        out_of_order_rows,
        duplicate_timestamps,
    })
}

fn find_column(columns: &[String], name: &str) -> Result<usize, String> {
    find_optional_column(columns, name)
        .ok_or_else(|| format!("CSV is missing required {name} column"))
}

fn find_optional_column(columns: &[String], name: &str) -> Option<usize> {
    columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case(name))
}

fn parse_optional_number(fields: &[&str], idx: Option<usize>) -> Option<f64> {
    let idx = idx?;
    let raw = fields.get(idx)?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.parse::<f64>().ok()
}
