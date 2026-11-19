use std::{
    backtrace::Backtrace,
    env, fs,
    io::{self, IsTerminal},
    panic,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use dye_opt::launch::{FileLog, LogColor, LogFormat, LogLevel, LogOpt};
use tracing::error;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer, Registry, fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt,
};

type BoxLayer = Box<dyn Layer<Registry> + Send + Sync>;

const OWN_TARGETS: &[&str] = &[
    "dye_app",
    "dye_core",
    "dye_engine",
    "dye_net",
    "dye_opt",
    "dye_render",
    "dye_res",
    "dye_wgpu",
    "dye_bin",
];

#[must_use = "dropping the guard stops file logging"]
pub struct LogGuard(Option<WorkerGuard>);

pub fn init(opt: &LogOpt, dir: &Path) -> LogGuard {
    let mut layers: Vec<BoxLayer> = Vec::new();
    let mut guard = None;

    if opt.to_console.enabled {
        let ansi = match opt.to_console.color {
            LogColor::Auto => io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none(),
            LogColor::Always => true,
            LogColor::Never => false,
        };
        layers.push(layer(opt.to_console.format, io::stderr, ansi, opt));
    }

    if opt.to_file.enabled {
        match open_log_file(&opt.to_file, dir) {
            Ok(file) => {
                let (writer, g) = tracing_appender::non_blocking(file);
                guard = Some(g);
                layers.push(layer(opt.to_file.format, writer, false, opt));
            }
            Err(e) => eprintln!("warning: cannot open log file at {}: {e}", dir.display()),
        }
    }

    let _ = tracing_subscriber::registry()
        .with(layers)
        .with(build_filter(opt))
        .try_init();

    if opt.capture_panics {
        install_panic_hook(opt.to_console.enabled);
    }

    LogGuard(guard)
}

fn layer<W>(format: LogFormat, writer: W, ansi: bool, opt: &LogOpt) -> BoxLayer
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let base = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(ansi)
        .with_target(opt.show_target);

    match format {
        LogFormat::Compact => base.compact().boxed(),
        LogFormat::Pretty => base.pretty().boxed(),
        LogFormat::Json => base.json().boxed(),
    }
}

fn build_filter(opt: &LogOpt) -> EnvFilter {
    if let Some(s) = &opt.filter {
        match EnvFilter::try_new(s) {
            Ok(f) => return f,
            Err(e) => eprintln!("warning: invalid log filter {s:?} ({e}); defaulting to `level`"),
        }
    }
    let level = directive(opt.level);
    let deps = if matches!(opt.level, LogLevel::Error) {
        "error"
    } else {
        "warn"
    };
    let own: Vec<String> = OWN_TARGETS.iter().map(|t| format!("{t}={level}")).collect();

    EnvFilter::new(format!("{deps},{}", own.join(",")))
}

fn directive(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

fn open_log_file(opt: &FileLog, dir: &Path) -> io::Result<fs::File> {
    fs::create_dir_all(dir)?;
    let latest = dir.join("latest.log");
    if latest.exists() {
        let secs = fs::metadata(&latest)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now())
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = fs::rename(&latest, dir.join(format!("log-{secs}.log")));
    }
    prune(opt, dir);

    fs::File::create(latest)
}

fn prune(opt: &FileLog, dir: &Path) {
    let Ok(d) = fs::read_dir(dir) else {
        return;
    };
    let mut old: Vec<PathBuf> = d
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| s.starts_with("log-") && s.ends_with(".log"))
        })
        .collect();
    old.sort();
    let excess = old.len().saturating_sub(opt.keep);
    for path in old.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

fn install_panic_hook(console_enabled: bool) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let backtrace = Backtrace::force_capture();
        error!(target: "panic", "{info}\n{backtrace}");

        if !console_enabled {
            previous(info);
        }
    }));
}
