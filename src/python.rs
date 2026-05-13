use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::fhr_core::{
    AnalysisConfig, FetalChannel, analyze_rolling_windows, read_analysis_request_json,
    read_monitor_csv_str, report_as_json,
};

#[pyfunction]
fn analyze_json_string(request_json: &str) -> PyResult<String> {
    let (input, config) =
        read_analysis_request_json(request_json).map_err(PyValueError::new_err)?;
    let report = analyze_rolling_windows(&input, config);
    Ok(report_as_json(&report))
}

#[pyfunction]
#[pyo3(signature = (
    csv_text,
    channel = "HR1",
    ga_weeks = None,
    window_min = None,
    step_sec = 60,
    last_only = false
))]
fn analyze_csv_string(
    csv_text: &str,
    channel: &str,
    ga_weeks: Option<u8>,
    window_min: Option<u32>,
    step_sec: u32,
    last_only: bool,
) -> PyResult<String> {
    let input = read_monitor_csv_str(csv_text).map_err(PyValueError::new_err)?;
    let config =
        csv_config(channel, ga_weeks, window_min, step_sec).map_err(PyValueError::new_err)?;
    let mut report = analyze_rolling_windows(&input, config);
    if last_only && report.windows.len() > 1 {
        if let Some(last) = report.windows.last().cloned() {
            report.windows.clear();
            report.windows.push(last);
        }
    }
    Ok(report_as_json(&report))
}

fn csv_config(
    channel: &str,
    ga_weeks: Option<u8>,
    window_min: Option<u32>,
    step_sec: u32,
) -> Result<AnalysisConfig, String> {
    if let Some(window_min) = window_min {
        if !(10..=30).contains(&window_min) {
            return Err("window_min must be between 10 and 30".to_string());
        }
    }
    Ok(AnalysisConfig {
        fetal_channel: FetalChannel::parse(channel)?,
        window_minutes: window_min,
        step_seconds: step_sec.max(1),
        gestational_age_weeks: ga_weeks,
    })
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(analyze_json_string, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_csv_string, m)?)?;
    Ok(())
}
