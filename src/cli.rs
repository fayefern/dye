use std::path::{self, PathBuf};

use clap::{ArgAction, Args, CommandFactory, FromArgMatches, Parser, parser::ValueSource};
use dye_opt::{
    dirs::{DirSlot, RootDir},
    file::parse_value,
    launch::{
        AdapterSelection, GpuErrorPolicy, LaunchOpt, LogLevel, PowerPreference, PresentMode,
        Validation, WgpuBackend, WindowMode, WindowSize,
    },
};

#[derive(Parser, Debug)]
#[command(name = "dye", version, about = "dye voxel engine")]
pub struct Cli {
    #[command(flatten)]
    pub paths: PathArgs,
    #[command(flatten)]
    pub overrides: LaunchOverrides,
    #[command(flatten)]
    pub actions: Actions,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Paths")]
pub struct PathArgs {
    #[arg(long, env = "DYE_DIR_ROOT", value_name = "DIR")]
    root_dir: Option<PathBuf>,
    #[arg(long, env = "DYE_DIR_OPTIONS", value_name = "DIR")]
    options_dir: Option<PathBuf>,
    #[arg(long, env = "DYE_DIR_LOGS", value_name = "DIR")]
    logs_dir: Option<PathBuf>,
    #[arg(long, env = "DYE_DIR_SAVES", value_name = "DIR")]
    saves_dir: Option<PathBuf>,
    #[arg(long, env = "DYE_DIR_CACHE", value_name = "DIR")]
    cache_dir: Option<PathBuf>,
}

impl PathArgs {
    pub fn root_dir(&self) -> RootDir {
        let mut dirs = match &self.root_dir {
            Some(p) => RootDir::new(p),
            None => RootDir::platform_default(),
        };
        for (slot, value) in [
            (DirSlot::Options, &self.options_dir),
            (DirSlot::Logs, &self.logs_dir),
            (DirSlot::Saves, &self.saves_dir),
            (DirSlot::Cache, &self.cache_dir),
        ] {
            if let Some(p) = value {
                let abs = path::absolute(p).unwrap_or_else(|_| p.clone());
                dirs.set_override(slot, abs);
            }
        }

        dirs
    }
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Commands")]
pub struct Actions {
    /// Lists available GPU adapters and exits. Exits.
    #[arg(long)]
    pub list_adapters: bool,
    /// Prints the fully resolved launch options. Exits.
    #[arg(long)]
    pub print_launch_opt: bool,
    /// Backs up launch.toml and replaces it with the default file. Exits.
    #[arg(long)]
    pub reset_launch: bool,
}

#[derive(Args, Debug, Default)]
#[command(next_help_heading = "Launch overrides")]
pub struct LaunchOverrides {
    #[arg(long, env = "DYE_WINDOW_MODE", value_name = "WINDOW_MODE", value_parser = parse_value::<WindowMode>)]
    window_mode: Option<WindowMode>,
    #[arg(long, env = "DYE_WINDOW_SIZE", value_name = "W*H", value_parser = str::parse::<WindowSize>)]
    window_size: Option<WindowSize>,

    #[arg(long, env = "DYE_GPU_BACKEND", value_name = "BACKEND", value_parser = parse_value::<WgpuBackend>)]
    backend: Option<WgpuBackend>,
    #[arg(long, env = "DYE_GPU_ADAPTER", value_name = "ADAPTER", value_parser = str::parse::<AdapterSelection>)]
    adapter: Option<AdapterSelection>,
    #[arg(long, env = "DYE_GPU_POWER", value_name = "POWER", value_parser = parse_value::<PowerPreference>)]
    power_preference: Option<PowerPreference>,
    #[arg(long, env = "DYE_GPU_PRESENT", value_name = "PRESENT", value_parser = parse_value::<PresentMode>)]
    present_mode: Option<PresentMode>,
    #[arg(long, env = "DYE_GPU_LATENCY", value_name = "N")]
    desired_max_frame_latency: Option<u32>,
    #[arg(long, env = "DYE_GPU_VALIDATION", value_name = "VALIDATION", value_parser = parse_value::<Validation>)]
    validation: Option<Validation>,
    #[arg(long, env = "DYE_GPU_POLICY", value_name = "POLICY", value_parser = parse_value::<GpuErrorPolicy>)]
    error_policy: Option<GpuErrorPolicy>,

    #[arg(long, env = "DYE_LOG_LEVEL", value_name = "LEVEL", value_parser = parse_value::<LogLevel>)]
    log_level: Option<LogLevel>,
    #[arg(long, env = "DYE_LOG_FILTER", value_name = "FILTER")]
    log_filter: Option<String>,
    #[arg(short = 'l', long, action = ArgAction::Count)]
    louder: u8,
    #[arg(short = 'q', long, action = ArgAction::Count)]
    quieter: u8,
}

impl LaunchOverrides {
    pub fn apply(&self, s: &mut LaunchOpt) {
        if let Some(v) = self.window_mode {
            s.window.mode = v;
        }
        if let Some(v) = self.window_size {
            s.window.size = v;
        }

        if let Some(v) = self.backend {
            s.gpu.backend = v;
        }
        if let Some(v) = &self.adapter {
            s.gpu.adapter = v.clone();
        }
        if let Some(v) = self.power_preference {
            s.gpu.power_preference = v;
        }
        if let Some(v) = self.present_mode {
            s.gpu.present_mode = v;
        }
        if let Some(v) = self.desired_max_frame_latency {
            s.gpu.desired_max_frame_latency = v;
        }
        if let Some(v) = self.validation {
            s.gpu.validation = v;
        }
        if let Some(v) = self.error_policy {
            s.gpu.error_policy = v;
        }

        if let Some(v) = self.log_level {
            s.log.level = v;
        }

        if let Some(v) = &self.log_filter {
            s.log.filter = Some(v.clone());
        }

        s.log.level = s.log.level.shift(self.louder as i8 - self.quieter as i8);
    }
}

pub fn parse() -> (Cli, Vec<(String, &'static str)>) {
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let sources = matches
        .ids()
        .filter_map(|id| {
            let src = match matches.value_source(id.as_str())? {
                ValueSource::CommandLine => "cli",
                ValueSource::EnvVariable => "env",
                _ => return None,
            };

            Some((id.as_str().to_string(), src))
        })
        .collect();

    (cli, sources)
}
