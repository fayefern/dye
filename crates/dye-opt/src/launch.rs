use std::{convert::Infallible, fmt::Display, path::PathBuf, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{
    dirs::{DirSlot, RootDir},
    file::OptFile,
};

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LaunchOpt {
    pub paths: PathsOpt,
    pub window: WindowOpt,
    pub gpu: GpuOpt,
    pub log: LogOpt,
}

impl OptFile for LaunchOpt {
    const FILE_NAME: &'static str = "launch.toml";
    const VERSION: u32 = 1;
    const TEMPLATE: Option<&'static str> = Some(include_str!("../assets/launch.template.toml"));

    fn validate(&mut self, issues: &mut crate::warning::Issues) {
        if !(1..8192).contains(&self.window.size.width)
            || !(1..8192).contains(&self.window.size.height)
        {
            let d = WindowOpt::default();
            issues.invalid(
                "window width and height",
                format!(
                    "must be non-zero, defaulted to {}x{}",
                    d.size.width, d.size.height
                ),
            );
            self.window.size = d.size;
        }
        if !(1..=3).contains(&self.gpu.desired_max_frame_latency) {
            issues.invalid("gpu.desired_max_frame_latency", "must be 1..=3. clamped");
            self.gpu.desired_max_frame_latency = self.gpu.desired_max_frame_latency.clamp(1, 3);
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PathsOpt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<PathBuf>,
    pub logs: Option<PathBuf>,
    pub saves: Option<PathBuf>,
    pub cache: Option<PathBuf>,
}

impl PathsOpt {
    pub fn apply(&self, dirs: &mut RootDir) {
        for (slot, value) in [
            (DirSlot::Options, &self.options),
            (DirSlot::Logs, &self.logs),
            (DirSlot::Saves, &self.saves),
            (DirSlot::Cache, &self.cache),
        ] {
            if let Some(p) = value
                && !dirs.is_overridden(slot)
            {
                dirs.set_override(slot, p);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WindowOpt {
    pub size: WindowSize,
    pub mode: WindowMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

impl Display for WindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}*{}", self.width, self.height)
    }
}

impl Serialize for WindowSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for WindowSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl FromStr for WindowSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s
            .split(|c: char| c.is_whitespace() || matches!(c, 'x' | 'X' | ',' | '*'))
            .filter(|p| !p.is_empty());
        let (Some(w), Some(h), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("expected two numbers like `1280 720`, got {s:?}"));
        };
        let num = |v: &str| {
            v.parse::<u32>()
                .map_err(|e| format!("invalid number {v:?}: {e}"))
        };

        Ok(Self {
            width: num(w)?,
            height: num(h)?,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Borderless,
    Fullscreen,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct GpuOpt {
    pub backend: WgpuBackend,
    pub adapter: AdapterSelection,
    pub power_preference: PowerPreference,
    pub present_mode: PresentMode,
    pub desired_max_frame_latency: u32,
    pub validation: Validation,
    pub error_policy: GpuErrorPolicy,
}

impl Default for GpuOpt {
    fn default() -> Self {
        Self {
            backend: WgpuBackend::All,
            adapter: AdapterSelection::Auto,
            power_preference: PowerPreference::HighPerformance,
            present_mode: PresentMode::AutoVsync,
            desired_max_frame_latency: 2,
            validation: Validation::Auto,
            error_policy: GpuErrorPolicy::Log,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WgpuBackend {
    #[default]
    All,
    Vulkan,
    Metal,
    Dx12,
    Gl,
    Noop,
}

#[derive(Clone, Debug, Default)]
pub enum AdapterSelection {
    #[default]
    Auto,
    Name(String),
    Index(usize),
    Software,
}

impl Serialize for AdapterSelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Name(n) => serializer.serialize_str(n),
            Self::Index(i) => serializer.serialize_u64(*i as u64),
            Self::Software => serializer.serialize_str("software"),
        }
    }
}

impl<'de> Deserialize<'de> for AdapterSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(usize),
            Text(String),
        }

        Ok(match Raw::deserialize(deserializer)? {
            Raw::Number(i) => Self::Index(i),
            Raw::Text(s) => s.parse().unwrap_or(Self::Auto),
        })
    }
}

impl FromStr for AdapterSelection {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        Ok(match t.to_lowercase().as_str() {
            "" | "auto" => Self::Auto,
            "software" => Self::Software,
            _ => t
                .parse()
                .map_or_else(|_| Self::Name(t.to_owned()), Self::Index),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PowerPreference {
    #[default]
    None,
    HighPerformance,
    LowPower,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PresentMode {
    #[default]
    AutoVsync,
    AutoNoVsync,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Validation {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GpuErrorPolicy {
    #[default]
    Log,
    Panic,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct LogOpt {
    pub level: LogLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    pub to_console: ConsoleLog,
    pub to_file: FileLog,
    pub timestamps: Timestamps,
    pub show_target: bool,
    pub show_thread: bool,
    pub show_source_location: bool,
    pub capture_panics: bool,
}

impl Default for LogOpt {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            filter: None,
            to_console: ConsoleLog::default(),
            to_file: FileLog::default(),
            timestamps: Timestamps::Utc,
            show_target: true,
            show_thread: false,
            show_source_location: false,
            capture_panics: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn shift(self, delta: i8) -> Self {
        const ORDER: [LogLevel; 5] = [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ];
        let curr = ORDER.iter().position(|l| *l == self).unwrap_or(2) as i8;
        ORDER[(curr + delta).clamp(0, 4) as usize]
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ConsoleLog {
    pub enabled: bool,
    pub format: LogFormat,
    pub color: LogColor,
}

impl Default for ConsoleLog {
    fn default() -> Self {
        Self {
            enabled: true,
            format: LogFormat::Compact,
            color: LogColor::Auto,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    Pretty,
    #[default]
    Compact,
    Json,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LogColor {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct FileLog {
    pub enabled: bool,
    pub format: LogFormat,
    pub rotation: FileRotation,
    pub keep: usize,
}

impl Default for FileLog {
    fn default() -> Self {
        Self {
            enabled: true,
            format: LogFormat::Compact,
            rotation: FileRotation::Launch,
            keep: 8,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FileRotation {
    #[default]
    Launch,
    Daily,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Timestamps {
    None,
    Relative,
    #[default]
    Utc,
    Local,
}
