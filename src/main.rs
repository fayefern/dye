use std::path::PathBuf;

use dye_app::{
    Renderers,
    app::{AppOpt, start},
};
use dye_render::{Backend, BackendDesc, BackendError};
use dye_wgpu::renderer::WgpuRenderer;

fn main() -> anyhow::Result<()> {
    let renderer = "wgpu";
    let data_dir = PathBuf::from("./dye-out/data");
    let cache_dir = PathBuf::from("./dye-out/cache");
    let opt = AppOpt::builder()
        .with_renderer(renderer)
        .with_data_dir(&data_dir)
        .with_cache_dir(&cache_dir)
        .with_window_inner_size(1920, 1080)
        .build()?;
    start(opt, RENDERERS)?;
    Ok(())
}

const RENDERERS: Renderers = &[("wgpu", wgpu)];

fn wgpu(desc: &BackendDesc) -> Result<Box<dyn Backend>, BackendError> {
    Ok(Box::new(WgpuRenderer::new(desc)?))
}
