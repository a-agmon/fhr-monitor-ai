#[derive(Clone, Debug)]
pub struct MonitorSample {
    pub timestamp_ms: i64,
    pub timestamp: String,
    pub hr1: Option<f64>,
    pub hr2: Option<f64>,
    pub hr3: Option<f64>,
    pub hrm: Option<f64>,
    pub toco: Option<f64>,
}

impl MonitorSample {
    pub fn fetal_value(&self, channel: FetalChannel) -> Option<f64> {
        match channel {
            FetalChannel::Hr1 => self.hr1,
            FetalChannel::Hr2 => self.hr2,
            FetalChannel::Hr3 => self.hr3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetalChannel {
    Hr1,
    Hr2,
    Hr3,
}

impl FetalChannel {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_uppercase().as_str() {
            "HR1" => Ok(Self::Hr1),
            "HR2" => Ok(Self::Hr2),
            "HR3" => Ok(Self::Hr3),
            other => Err(format!("unsupported fetal channel: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hr1 => "HR1",
            Self::Hr2 => "HR2",
            Self::Hr3 => "HR3",
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputData {
    pub samples: Vec<MonitorSample>,
    pub columns: Vec<String>,
    pub out_of_order_rows: usize,
    pub duplicate_timestamps: usize,
}

#[derive(Clone, Debug)]
pub struct AnalysisConfig {
    pub fetal_channel: FetalChannel,
    pub window_minutes: u32,
    pub step_seconds: u32,
    pub gestational_age_weeks: Option<u8>,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            fetal_channel: FetalChannel::Hr1,
            window_minutes: 30,
            step_seconds: 60,
            gestational_age_weeks: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisReport {
    pub config: AnalysisConfig,
    pub input: InputSummary,
    pub windows: Vec<WindowAnalysis>,
}

#[derive(Clone, Debug)]
pub struct InputSummary {
    pub rows: usize,
    pub start_timestamp: Option<String>,
    pub end_timestamp: Option<String>,
    pub duration_seconds: f64,
    pub out_of_order_rows: usize,
    pub duplicate_timestamps: usize,
}

#[derive(Clone, Debug)]
pub struct WindowAnalysis {
    pub window_start: String,
    pub window_end: String,
    pub duration_seconds: f64,
    pub data_quality: DataQuality,
    pub baseline_bpm: Option<i32>,
    pub baseline_class: Option<BaselineClass>,
    pub variability_bpm: Option<f64>,
    pub variability_class: Option<VariabilityClass>,
    pub accelerations: Vec<AccelerationEvent>,
    pub decelerations: Vec<DecelerationEvent>,
    pub toco: TocoSummary,
    pub classification: CategoryClassification,
    pub alert_level: AlertLevel,
    pub reasons: Vec<String>,
    pub high_risk_features: Vec<String>,
    pub protective_features: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DataQuality {
    pub expected_seconds: usize,
    pub raw_samples: usize,
    pub fetal_usable_seconds: usize,
    pub fetal_usable_ratio: f64,
    pub maternal_usable_ratio: f64,
    pub toco_usable_ratio: f64,
    pub suspected_maternal_capture_ratio: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselineClass {
    Bradycardia,
    Normal,
    Tachycardia,
}

impl BaselineClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bradycardia => "bradycardia",
            Self::Normal => "normal",
            Self::Tachycardia => "tachycardia",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VariabilityClass {
    Absent,
    Minimal,
    Moderate,
    Marked,
}

impl VariabilityClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Minimal => "minimal",
            Self::Moderate => "moderate",
            Self::Marked => "marked",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AccelerationEvent {
    pub start: String,
    pub end: String,
    pub duration_seconds: f64,
    pub peak_bpm: f64,
}

#[derive(Clone, Debug)]
pub struct DecelerationEvent {
    pub start: String,
    pub end: String,
    pub duration_seconds: f64,
    pub nadir_bpm: f64,
    pub depth_bpm: f64,
    pub onset_to_nadir_seconds: f64,
    pub kind: DecelerationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecelerationKind {
    Variable,
    Early,
    Late,
    Prolonged,
    GradualUnclassified,
}

impl DecelerationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Early => "early",
            Self::Late => "late",
            Self::Prolonged => "prolonged",
            Self::GradualUnclassified => "gradual_unclassified",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContractionEvent {
    pub start: String,
    pub peak: String,
    pub end: String,
    pub duration_seconds: f64,
    pub peak_toco: f64,
}

#[derive(Clone, Debug)]
pub struct TocoSummary {
    pub contractions: Vec<ContractionEvent>,
    pub contractions_per_10_min: f64,
    pub tachysystole: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CategoryClassification {
    CategoryI,
    CategoryII,
    CategoryIII,
    Unclassified,
}

impl CategoryClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CategoryI => "category_i",
            Self::CategoryII => "category_ii",
            Self::CategoryIII => "category_iii",
            Self::Unclassified => "unclassified",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertLevel {
    None,
    Info,
    Warning,
    Critical,
    DataQuality,
}

impl AlertLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
            Self::DataQuality => "data_quality",
        }
    }
}
