use std::path::PathBuf;

use dye_app::app::{AppOpt, start};

fn main() -> anyhow::Result<()> {
    let data_dir = PathBuf::from("./dye-out/data");
    let cache_dir = PathBuf::from("./dye-out/cache");
    let opt = AppOpt::builder()
        .with_data_dir(&data_dir)
        .with_cache_dir(&cache_dir)
        .with_window_inner_size(1920, 1080)
        .build()?;
    start(opt)?;
    Ok(())
}
