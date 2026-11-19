use std::{
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread::{self, JoinHandle, available_parallelism},
};

use crossbeam_channel::{Sender, unbounded};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct JobPool {
    tx: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl JobPool {
    pub fn default_threads() -> usize {
        available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1))
            .max(1)
    }

    pub fn new(name: &str, threads: usize) -> Self {
        let (tx, rx) = unbounded::<Job>();
        let workers = (0..threads.max(1))
            .map(|idx| {
                let rx = rx.clone();
                thread::Builder::new()
                    .name(format!("{name}-{idx}"))
                    .spawn(move || {
                        while let Ok(job) = rx.recv() {
                            let _ = panic::catch_unwind(AssertUnwindSafe(job));
                        }
                    })
                    .expect("failed to spawn worker thread")
            })
            .collect();

        Self {
            tx: Some(tx),
            workers,
        }
    }

    pub fn spawn(&self, job: impl FnOnce() + Send + 'static) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Box::new(job));
        }
    }
}

impl Drop for JobPool {
    fn drop(&mut self) {
        drop(self.tx.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
