use crate::utils;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

#[derive(Default, Clone)]
pub struct ImagingProgress {
    pub bytes_copied: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub sha256_digest: String,
    pub done: bool,
    pub error: Option<String>,
    pub verified: Option<bool>,
}

#[derive(Default)]
pub struct ImagingState {
    pub source: String,
    pub dest: String,
    pub running: bool,
    pub progress: ImagingProgress,
    pub rx: Option<mpsc::Receiver<ImagingProgress>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl ImagingState {
    pub fn start(&mut self) {
        let source = self.source.clone();
        let dest = self.dest.clone();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.rx = Some(rx);
        self.running = true;

        thread::spawn(move || {
            let mut src = match OpenOptions::new().read(true).open(&source) {
                Ok(f) => f,
                Err(e) => {
                    tx.send(ImagingProgress {
                        bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                        sha256_digest: String::new(), done: true,
                        error: Some(format!("Cannot open source: {e}")),
                        verified: None,
                    }).ok();
                    return;
                }
            };

            let mut dst = match File::create(&dest) {
                Ok(f) => f,
                Err(e) => {
                    tx.send(ImagingProgress {
                        bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                        sha256_digest: String::new(), done: true,
                        error: Some(format!("Cannot create dest: {e}")),
                        verified: None,
                    }).ok();
                    return;
                }
            };

            let total = src.seek(SeekFrom::End(0)).unwrap_or(0);
            let _ = src.seek(SeekFrom::Start(0));

            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 1024 * 1024];
            let mut copied: u64 = 0;
            let start_time = std::time::Instant::now();

            loop {
                if cancel.load(Ordering::Relaxed) {
                    tx.send(ImagingProgress {
                        bytes_copied: copied, total_bytes: total, speed_bps: 0.0,
                        sha256_digest: String::new(), done: true,
                        error: Some("Cancelled by user".into()),
                        verified: None,
                    }).ok();
                    return;
                }

                match src.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];
                        hasher.update(chunk);
                        if let Err(e) = dst.write_all(chunk) {
                            tx.send(ImagingProgress {
                                bytes_copied: copied, total_bytes: total, speed_bps: 0.0,
                                sha256_digest: String::new(), done: true,
                                error: Some(format!("Write error: {e}")),
                                verified: None,
                            }).ok();
                            return;
                        }
                        copied += n as u64;
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 { copied as f64 / elapsed } else { 0.0 };

                        tx.send(ImagingProgress {
                            bytes_copied: copied, total_bytes: total, speed_bps: speed,
                            sha256_digest: String::new(), done: false,
                            error: None, verified: None,
                        }).ok();
                    }
                    Err(e) => {
                        tx.send(ImagingProgress {
                            bytes_copied: copied, total_bytes: total, speed_bps: 0.0,
                            sha256_digest: String::new(), done: true,
                            error: Some(format!("Read error: {e}")),
                            verified: None,
                        }).ok();
                        return;
                    }
                }
            }

            let hash = format!("{:x}", hasher.finalize());
            tx.send(ImagingProgress {
                bytes_copied: copied, total_bytes: total, speed_bps: 0.0,
                sha256_digest: hash, done: true, error: None, verified: None,
            }).ok();
        });
    }

    pub fn verify(&mut self) {
        let source = self.source.clone();
        let dest = self.dest.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.running = true;

        thread::spawn(move || {
            let hash_file = |path: &str| -> Option<String> {
                let mut f = OpenOptions::new().read(true).open(path).ok()?;
                let mut h = Sha256::new();
                let mut buf = vec![0u8; 1024 * 1024];
                loop {
                    match f.read(&mut buf).ok()? {
                        0 => break,
                        n => h.update(&buf[..n]),
                    }
                }
                Some(format!("{:x}", h.finalize()))
            };

            let hs = hash_file(&source);
            let hd = hash_file(&dest);

            if let (Some(s), Some(d)) = (&hs, &hd) {
                tx.send(ImagingProgress {
                    bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                    sha256_digest: format!("Src: {s}\nDst: {d}"), done: true,
                    error: None, verified: Some(s == d),
                }).ok();
            } else {
                tx.send(ImagingProgress {
                    bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                    sha256_digest: String::new(), done: true,
                    error: Some("Verification failed — cannot read one or both files".into()),
                    verified: None,
                }).ok();
            }
        });
    }

    pub fn cancel(&self) {
        if let Some(ref c) = self.cancel {
            c.store(true, Ordering::Relaxed);
        }
    }
}

pub fn imaging_ui(state: &mut ImagingState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("💾 Disk Imaging");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Source device:");
        ui.text_edit_singleline(&mut state.source);
    });
    ui.horizontal(|ui| {
        ui.label("Dest image file:");
        ui.text_edit_singleline(&mut state.dest);
    });
    ui.separator();

    if !state.running {
        if ui.add_enabled(!state.source.is_empty() && !state.dest.is_empty(), egui::Button::new("▶ Start Imaging")).clicked() {
            state.start();
        }
    } else {
        if ui.button("⏹ Stop").clicked() {
            state.cancel();
            state.running = false;
        }
    }

    if let Some(rx) = &state.rx {
        while let Ok(p) = rx.try_recv() {
            state.progress = p;
            if state.progress.done {
                state.running = false;
            }
            ctx.request_repaint();
        }
    }

    let progress = state.progress.clone();
    if progress.total_bytes > 0 {
        let fraction = progress.bytes_copied as f64 / progress.total_bytes as f64;
        ui.add(egui::ProgressBar::new(fraction as f32).text(format!(
            "{:.1}% — {}/{}", fraction * 100.0,
            utils::format_size(progress.bytes_copied), utils::format_size(progress.total_bytes)
        )));

        if progress.speed_bps > 0.0 {
            let remaining = progress.total_bytes.saturating_sub(progress.bytes_copied);
            let eta_secs = (remaining as f64 / progress.speed_bps) as u64;
            ui.label(format!("Speed: {}/s  ETA: {}",
                utils::format_size(progress.speed_bps as u64), utils::format_duration(eta_secs)));
        }
    }

    if !progress.sha256_digest.is_empty() {
        ui.label(format!("SHA-256: {}", progress.sha256_digest));
    }

    if let Some(ref verified) = progress.verified {
        if *verified {
            ui.colored_label(egui::Color32::GREEN, "✓ SHA-256 verified — image is identical");
        } else {
            ui.colored_label(egui::Color32::RED, "✗ SHA-256 MISMATCH — image is corrupted");
        }
    }

    if !state.running && progress.done && progress.bytes_copied > 0 {
        if ui.button("🔍 Verify SHA-256").clicked() {
            state.verify();
        }
    }

    if let Some(ref e) = progress.error {
        ui.colored_label(egui::Color32::RED, e);
    }
}
