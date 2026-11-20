use std::{fs, process::ExitCode, sync::Arc, time::Duration};

use dye_app::{Renderers, app::start};
use dye_assets::{
    asset::KindTable,
    discovery::{discover, scan_packs_dir},
    kinds::Image,
    manifest::NetworkSide,
    reload::Reloader,
    source::PackSource,
    store::Assets,
};
use dye_core::JobPool;
use dye_opts::{
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

    let pool = Arc::new(JobPool::new("workers", JobPool::default_threads()));
    let mut kinds = KindTable::default();
    kinds.register::<Image>();

    let builtin: Arc<dyn PackSource> = Arc::from(builtin::source());
    let (others, scan_diags) = scan_packs_dir(&dirs.packs());
    let mut found = discover(&kinds, builtin.clone(), others, NetworkSide::Both);
    let diags: Vec<_> = scan_diags
        .into_iter()
        .chain(found.diags.drain(..))
        .collect();
    for d in &diags {
        warn!("{d}");
    }
    for p in &found.packs {
        info!(id = %p.id, version = %p.version, from = %p.label, "pack");
    }
    let assets = Assets::new(kinds, Arc::new(found.index), pool.clone());
    let assets_opts = &loaded.value.assets;
    let reloader = assets_opts.hot_reload.then(|| {
        Reloader::new(
            &assets,
            pool,
            dirs.packs(),
            builtin,
            NetworkSide::Both,
            Duration::from_millis(assets_opts.reload_interval_ms),
            &diags,
        )
    });

    match start(loaded.value, RENDERERS, assets, reloader) {
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
