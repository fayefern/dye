use std::{
    collections::HashSet,
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use dye_core::JobPool;

use crate::{
    discovery::{discover, scan_packs_dir},
    error::AssetDiag,
    index::AssetIndex,
    manifest::NetworkSide,
    parse::Shared,
    source::PackSource,
    store::Assets,
};

struct Scan {
    index: AssetIndex,
    diags: Vec<AssetDiag>,
}

pub struct Reload {
    pub changed: usize,
    pub diags: Vec<AssetDiag>,
}

pub struct Reloader {
    pool: Arc<JobPool>,
    shared: Arc<Shared>,
    dir: PathBuf,
    builtin: Arc<dyn PackSource>,
    side: NetworkSide,
    interval: Duration,
    tx: Sender<Option<Scan>>,
    rx: Receiver<Option<Scan>>,
    last: Instant,
    scanning: bool,
    reported: HashSet<String>,
}

impl Reloader {
    pub fn new(
        assets: &Assets,
        pool: Arc<JobPool>,
        dir: PathBuf,
        builtin: Arc<dyn PackSource>,
        side: NetworkSide,
        interval: Duration,
        known: &[AssetDiag],
    ) -> Self {
        let (tx, rx) = unbounded();

        Self {
            pool,
            shared: assets.shared(),
            dir,
            builtin,
            side,
            interval,
            tx,
            rx,
            last: Instant::now(),
            scanning: false,
            reported: known.iter().map(|d| d.to_string()).collect(),
        }
    }

    pub fn poll(&mut self, assets: &mut Assets) -> Option<Reload> {
        let mut report = None;

        if let Ok(result) = self.rx.try_recv() {
            self.scanning = false;
            self.last = Instant::now();
            if let Some(scan) = result {
                let changed = assets.replace_index(Arc::new(scan.index));
                let current: HashSet<String> = scan.diags.iter().map(|d| d.to_string()).collect();
                let fresh: Vec<AssetDiag> = scan
                    .diags
                    .into_iter()
                    .filter(|d| !self.reported.contains(&d.to_string()))
                    .collect();
                self.reported = current;
                if changed > 0 || !fresh.is_empty() {
                    report = Some(Reload {
                        changed,
                        diags: fresh,
                    });
                }
            }
        }

        if !self.scanning && self.last.elapsed() >= self.interval {
            self.scanning = true;
            let (shared, tx, dir) = (self.shared.clone(), self.tx.clone(), self.dir.clone());
            let (builtin, side) = (self.builtin.clone(), self.side);
            self.pool.spawn(move || {
                let scan = panic::catch_unwind(AssertUnwindSafe(|| {
                    let (others, mut diags) = scan_packs_dir(&dir);
                    let found = discover(&shared.kinds, builtin, others, side);
                    diags.extend(found.diags);
                    Scan {
                        index: found.index,
                        diags,
                    }
                }));
                let _ = tx.send(scan.ok());
            });
        }

        report
    }
}
