//! Background jobs.
//!
//! Compression, export and scan decoding take seconds and the GUI would
//! freeze meanwhile. Each job therefore runs in its own thread and sends the
//! result back over a channel. When done, `notify` is called (in the app
//! `ctx.request_repaint()`) so the GUI shows the result right away.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use pabaru_core::{CoreError, Decoder, Destination, EncodeOptions, PageScan, Prepared};

use crate::i18n::{self, tr};
use crate::model::{ExportSummary, RestoreSummary};

/// Where to restore (owned variant of `Destination`).
#[derive(Debug, Clone)]
pub enum RestoreTarget {
    Folder(PathBuf),
}

/// A job for a worker thread.
#[derive(Debug, Clone)]
pub enum Job {
    Prepare {
        generation: u64,
        inputs: Vec<PathBuf>,
    },
    Export {
        prepared: Arc<Prepared>,
        opts: Box<EncodeOptions>,
        path: PathBuf,
        /// `true` = pages as PNG files into the folder `path`.
        png: bool,
    },
    DecodeScan {
        path: PathBuf,
    },
    Restore {
        decoder: Box<Decoder>,
        password: Option<String>,
        target: RestoreTarget,
    },
}

/// Job result.
#[derive(Debug)]
pub enum JobResult {
    Prepared {
        generation: u64,
        result: Result<Prepared, String>,
    },
    Exported(Result<ExportSummary, String>),
    ScanDecoded {
        path: PathBuf,
        result: Result<Vec<PageScan>, String>,
    },
    Restored(Result<RestoreSummary, String>),
}

/// User-friendly error message.
pub fn friendly_error(e: &CoreError) -> String {
    match e {
        CoreError::Crypto(_) => tr(
            "Wrong password or damaged data.",
            "Špatné heslo nebo poškozená data.",
        )
        .into(),
        CoreError::PasswordRequired => tr(
            "The backup is encrypted – enter the password.",
            "Záloha je šifrovaná – zadejte heslo.",
        )
        .into(),
        CoreError::NotEnoughBlocks { have, need } => match i18n::lang() {
            i18n::Lang::En => {
                format!("Not enough readable blocks ({have} of {need}). Scan the missing pages.")
            }
            i18n::Lang::Cs => format!(
                "Nedostatek čitelných bloků ({have} z {need}). Naskenujte chybějící stránky."
            ),
        },
        other => other.to_string(),
    }
}

/// Runs a job (synchronously). Called from a thread; testable on its own.
pub fn run(job: Job) -> JobResult {
    match job {
        Job::Prepare { generation, inputs } => JobResult::Prepared {
            generation,
            result: pabaru_core::prepare(&inputs, 19).map_err(|e| friendly_error(&e)),
        },
        Job::Export {
            prepared,
            opts,
            path,
            png,
        } => JobResult::Exported(
            (|| {
                let backup = pabaru_core::encode_prepared(&prepared, &opts)?;
                if png {
                    backup.write_pngs(&path, "pabaru")?;
                } else {
                    backup.write_pdf(&path)?;
                }
                Ok(ExportSummary {
                    path,
                    pages: backup.page_count(),
                })
            })()
            .map_err(|e: CoreError| friendly_error(&e)),
        ),
        Job::DecodeScan { path } => {
            let result = pabaru_core::load_images(&path)
                .map_err(|e| e.to_string())
                .and_then(|images| {
                    images
                        .iter()
                        .map(|img| pabaru_core::decode_page(img).map_err(|e| e.to_string()))
                        .collect()
                });
            JobResult::ScanDecoded { path, result }
        }
        Job::Restore {
            decoder,
            password,
            target,
        } => {
            let (dest, label) = match &target {
                RestoreTarget::Folder(p) => (Destination::Folder(p), p.display().to_string()),
            };
            JobResult::Restored(
                decoder
                    .restore(password.as_deref(), dest)
                    .map(|r| RestoreSummary {
                        files: r.unpack.verified_files,
                        recovered_blocks: r.recovered_shards,
                        target: label,
                    })
                    .map_err(|e| friendly_error(&e)),
            )
        }
    }
}

/// An "error" result matching the kind of job.
fn failure_for(job: &Job, msg: &str) -> JobResult {
    let e = msg.to_string();
    match job {
        Job::Prepare { generation, .. } => JobResult::Prepared {
            generation: *generation,
            result: Err(e),
        },
        Job::Export { .. } => JobResult::Exported(Err(e)),
        Job::DecodeScan { path } => JobResult::ScanDecoded {
            path: path.clone(),
            result: Err(e),
        },
        Job::Restore { .. } => JobResult::Restored(Err(e)),
    }
}

/// Background job runner.
pub struct Worker {
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    notify: Arc<dyn Fn() + Send + Sync>,
    pending: Arc<AtomicUsize>,
}

impl Worker {
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            notify,
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Starts a job in a thread.
    pub fn submit(&self, job: Job) {
        let tx = self.tx.clone();
        let notify = self.notify.clone();
        let pending = self.pending.clone();
        pending.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            // If the job panicked the GUI would wait forever – send an error instead.
            let fallback = failure_for(
                &job,
                tr(
                    "Unexpected internal error during processing.",
                    "Neočekávaná vnitřní chyba při zpracování.",
                ),
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job)))
                .unwrap_or(fallback);
            // Send first, then decrement – whoever sees 0 finds the result in the queue.
            let _ = tx.send(result);
            pending.fetch_sub(1, Ordering::SeqCst);
            notify();
        });
    }

    /// Collects finished results.
    pub fn poll(&self) -> Vec<JobResult> {
        self.rx.try_iter().collect()
    }

    /// Number of running jobs.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_returns_result() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "data").unwrap();
        let w = Worker::new(Arc::new(|| {}));
        w.submit(Job::Prepare {
            generation: 7,
            inputs: vec![dir.path().to_path_buf()],
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut results = Vec::new();
        while results.is_empty() && std::time::Instant::now() < deadline {
            results = w.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(matches!(
            results.as_slice(),
            [JobResult::Prepared {
                generation: 7,
                result: Ok(_)
            }]
        ));
        assert_eq!(w.pending(), 0);
    }

    #[test]
    fn bad_scan_returns_error() {
        let r = run(Job::DecodeScan {
            path: PathBuf::from("does-not-exist.png"),
        });
        assert!(matches!(r, JobResult::ScanDecoded { result: Err(_), .. }));
    }
}
