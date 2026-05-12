use std::env;
use std::process;

use fhr_monitor::fhr_core::report_as_json;
use fhr_monitor::{AnalysisConfig, FetalChannel, analyze_rolling_windows, read_monitor_csv};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage();
        return Ok(());
    }

    let mut csv_path = None;
    let mut config = AnalysisConfig::default();
    let mut json = false;
    let mut last_only = false;

    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--channel" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| "--channel requires HR1, HR2, or HR3".to_string())?;
                config.fetal_channel = FetalChannel::parse(value)?;
            }
            "--window-min" => {
                idx += 1;
                config.window_minutes = Some(parse_u32_arg(&args, idx, "--window-min")?);
            }
            "--step-sec" => {
                idx += 1;
                config.step_seconds = parse_u32_arg(&args, idx, "--step-sec")?;
            }
            "--ga-weeks" => {
                idx += 1;
                config.gestational_age_weeks =
                    Some(parse_u32_arg(&args, idx, "--ga-weeks")?.min(u8::MAX as u32) as u8);
            }
            "--json" => json = true,
            "--last-only" => last_only = true,
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => {
                if csv_path.is_some() {
                    return Err(format!("unexpected extra path: {value}"));
                }
                csv_path = Some(value.to_string());
            }
        }
        idx += 1;
    }

    if let Some(window_minutes) = config.window_minutes {
        if !(10..=30).contains(&window_minutes) {
            return Err("--window-min must be between 10 and 30 for this prototype".to_string());
        }
    }
    let csv_path = csv_path.ok_or_else(|| "missing CSV path".to_string())?;
    let input = read_monitor_csv(&csv_path)?;
    let mut report = analyze_rolling_windows(&input, config);
    if last_only && report.windows.len() > 1 {
        if let Some(last) = report.windows.last().cloned() {
            report.windows.clear();
            report.windows.push(last);
        }
    }

    if json {
        print!("{}", report_as_json(&report));
    } else {
        print_text_report(&csv_path, &report);
    }
    Ok(())
}

fn parse_u32_arg(args: &[String], idx: usize, name: &str) -> Result<u32, String> {
    args.get(idx)
        .ok_or_else(|| format!("{name} requires a value"))?
        .parse::<u32>()
        .map_err(|_| format!("{name} must be a positive integer"))
}

fn print_usage() {
    println!(
        "Usage: fhr-cli <csv-path> [--channel HR1|HR2|HR3] [--window-min 10..30] [--step-sec N] [--last-only] [--json]

By default, the CLI analyzes the available chunk and infers its duration. Use --window-min only for rolling-window replay."
    );
}

fn print_text_report(path: &str, report: &fhr_monitor::AnalysisReport) {
    println!("file: {path}");
    println!(
        "input: rows={} start={} end={} duration={:.1} min out_of_order={} duplicate_ts={}",
        report.input.rows,
        report.input.start_timestamp.as_deref().unwrap_or("unknown"),
        report.input.end_timestamp.as_deref().unwrap_or("unknown"),
        report.input.duration_seconds / 60.0,
        report.input.out_of_order_rows,
        report.input.duplicate_timestamps
    );
    let window_label = report
        .config
        .window_minutes
        .map(|minutes| format!("{minutes} min rolling"))
        .unwrap_or_else(|| "inferred chunk, capped at latest 30 min".to_string());
    println!(
        "config: channel={} window={} step={} sec",
        report.config.fetal_channel.as_str(),
        window_label,
        report.config.step_seconds
    );
    println!();

    for window in &report.windows {
        println!(
            "{} -> {} | {} | alert={} | baseline={} {:?} | variability={} {:?} | usable={:.0}% | ctx={:.1}/10min",
            window.window_start,
            window.window_end,
            window.classification.as_str(),
            window.alert_level.as_str(),
            window
                .baseline_bpm
                .map(|value| value.to_string())
                .unwrap_or_else(|| "NA".to_string()),
            window.baseline_class.map(|value| value.as_str()),
            window
                .variability_bpm
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "NA".to_string()),
            window.variability_class.map(|value| value.as_str()),
            window.data_quality.fetal_usable_ratio * 100.0,
            window.toco.contractions_per_10_min
        );
        if !window.high_risk_features.is_empty() {
            println!("  high-risk: {}", window.high_risk_features.join("; "));
        }
        if !window.protective_features.is_empty() {
            println!("  protective: {}", window.protective_features.join("; "));
        }
        if !window.reasons.is_empty() {
            println!("  reasons: {}", window.reasons.join("; "));
        }
        if !window.limitations.is_empty() {
            println!("  limitations: {}", window.limitations.join("; "));
        }
    }
}
