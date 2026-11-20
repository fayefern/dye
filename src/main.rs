use std::{fs, process::ExitCode};

use dye_app::{Renderers, app::start};
use dye_opt::{
    dirs::RootDir,
    file::{OptFile, load, render, write_default},
    launch::LaunchOpt,
    warning::OptWarning,
};
use dye_render::{Backend, BackendDesc, BackendError};
use dye_wgpu::renderer::{WgpuRenderer, describe_adapters};
use tracing::{error, info, warn};

pub mod builtin;
pub mod cli;
pub mod logging;

fn main() -> ExitCode {
    let (cli, sources) = cli::parse();
    let mut dirs = cli.paths.root_dir();

    if cli.actions.reset_launch {
        return reset_launch(&dirs);
    }

    let mut loaded = load::<LaunchOpt>(&dirs.options());
    if !loaded.existed
        && let Err(e) = write_default::<LaunchOpt>(&dirs.options())
    {
        warn!("could not write launch.default.toml: {e}");
    }
    cli.overrides.apply(&mut loaded.value);
    loaded.revalidate();
    loaded.value.paths.apply(&mut dirs);

    if cli.actions.list_adapters || cli.actions.print_launch_opt {
        eprint_warnings(&loaded.warnings);
        if cli.actions.list_adapters {
            for line in describe_adapters(loaded.value.gpu.backend) {
                println!("{line}");
            }
        } else {
            print!("{}", render(&loaded.value));
        }

        return ExitCode::SUCCESS;
    }

    let _log = logging::init(&loaded.value.log, &dirs.logs());
    for warning in &loaded.warnings {
        warn!("{}: {}", warning.path.display(), warning.kind);
    }
    for (slot, path, overridden) in dirs.entries() {
        info!(?slot, path = %path.display(), overridden, "directory");
    }
    info!(
        file = %loaded.path.display(),
        file_loaded = loaded.existed,
        overrides = ?sources,
        "launch settings resolved"
    );

    match start(loaded.value, RENDERERS) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("exiting with: {e}");
            ExitCode::FAILURE
        }
    }
}

fn reset_launch(dirs: &RootDir) -> ExitCode {
    let path = dirs.options().join(LaunchOpt::FILE_NAME);
    if path.exists() {
        let _ = fs::rename(&path, path.with_extension("toml.bak"));
    }
    match write_default::<LaunchOpt>(&dirs.options()) {
        Ok(()) => {
            eprintln!("wrote default {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("could not write default: {e}");
            ExitCode::FAILURE
        }
    }
}

fn eprint_warnings(warnings: &[OptWarning]) {
    for warning in warnings {
        eprintln!("warning: {}: {}", warning.path.display(), warning.kind);
    }
}

const RENDERERS: Renderers = &[("wgpu", wgpu)];

fn wgpu(desc: &BackendDesc) -> Result<Box<dyn Backend>, BackendError> {
    Ok(Box::new(WgpuRenderer::new(desc)?))
}
