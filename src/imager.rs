use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::utils::{format_duration, format_size};

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
    cancel: Arc<AtomicBool>,
}

impl ImagingState {
    pub fn start(&mut self) {
        let source = self.source.clone();
        let dest = self.dest.clone();
        let (tx, rx) = mpsc::channel::<ImagingProgress>();
        self.rx = Some(rx);
        self.running = true;

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();

        thread::spawn(move || {
            let mut src = match OpenOptions::new().read(true).open(&source) {
                Ok(f) => f,
                Err(e) => {
                    send_error(&tx, 0, format!("Cannot open source: {e}"));
                    return;
                }
            };

            let total = src.seek(SeekFrom::End(0)).unwrap_or(0);
            if src.seek(SeekFrom::Start(0)).is_err() {
                send_error(&tx, total, "Cannot seek source to start".into());
                return;
            }

            let dst_file = match File::create(&dest) {
                Ok(f) => f,
                Err(e) => {
                    send_error(&tx, total, format!("Cannot create dest: {e}"));
                    return;
                }
            };
            let mut dst = BufWriter::new(dst_file);

            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 1024 * 1024];
            let mut copied: u64 = 0;
            let start_time = std::time::Instant::now();

            loop {
                if cancel.load(Ordering::Relaxed) {
                    send_error(&tx, total, "Imaging cancelled by user".into());
                    return;
                }

                match src.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];
                        hasher.update(chunk);
                        if let Err(e) = dst.write_all(chunk) {
                            send_error(&tx, total, format!("Write error: {e}"));
                            return;
                        }
                        copied += n as u64;
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 { copied as f64 / elapsed } else { 0.0 };

                        tx.send(ImagingProgress {
                            bytes_copied: copied,
                            total_bytes: total,
                            speed_bps: speed,
                            sha256_digest: String::new(),
                            done: false,
                            error: None,
                            verified: None,
                        }).ok();
                    }
                    Err(e) => {
                        send_error(&tx, total, format!("Read error: {e}"));
                        return;
                    }
                }
            }

            let _ = dst.flush();
            let hash = format!("{:x}", hasher.finalize());
            tx.send(ImagingProgress {
                bytes_copied: copied,
                total_bytes: total,
                speed_bps: 0.0,
                sha256_digest: hash,
                done: true,
                error: None,
                verified: None,
            }).ok();
        });
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.running = false;
    }

    pub fn verify(&mut self) {
        let source = self.source.clone();
        let dest = self.dest.clone();
        let (tx, rx) = mpsc::channel::<ImagingProgress>();
        self.rx = Some(rx);
        self.running = true;

        thread::spawn(move || {
            let hash_file = |path: &str| -> Option<String> {
                let mut f = OpenOptions::new().read(true).open(path).ok()?;
                let mut h = Sha256::new();
                let mut buf = vec![0u8; 1024 * 1024];
                loop {
                    match f.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => h.update(&buf[..n]),
                        Err(_) => return None,
                    }
                }
                Some(format!("{:x}", h.finalize()))
            };

            let hs = hash_file(&source);
            let hd = hash_file(&dest);

            match (&hs, &hd) {
                (Some(s), Some(d)) => {
                    tx.send(ImagingProgress {
                        bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                        sha256_digest: format!("Src: {s}\nDst: {d}"),
                        done: true, error: None, verified: Some(s == d),
                    }).ok();
                }
                _ => {
                    tx.send(ImagingProgress {
                        bytes_copied: 0, total_bytes: 0, speed_bps: 0.0,
                        sha256_digest: String::new(), done: true,
                        error: Some("Verification failed — cannot read one or both files".into()),
                        verified: None,
                    }).ok();
                }
            }
        });
    }
}

fn send_error(tx: &mpsc::Sender<ImagingProgress>, total: u64, msg: String) {
    tx.send(ImagingProgress {
        bytes_copied: 0, total_bytes: total, speed_bps: 0.0,
        sha256_digest: String::new(), done: true,
        error: Some(msg), verified: None,
    }).ok();
}

pub fn imaging_ui(state: &mut ImagingState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("💾 Disk Imaging");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Source device:");
        ui.text_edit_singleline(&mut state.source);
    });
    ui.horizontal(|ui| {
        ui.label("Dest image file: ");
        ui.text_edit_singleline(&mut state.dest);
    });
    ui.separator();

    if !state.running {
        if ui.add_enabled(!state.source.is_empty() && !state.dest.is_empty(), egui::Button::new("▶ Start Imaging")).clicked() {
            state.start();
        }
    } else {
        if ui.button("⏹ Stop").clicked() {
            state.stop();
        }
    }

    if let Some(rx) = &state.rx {
        while let Ok(p) = rx.try_recv() {
            state.progress = p;
            if state.progress.done { state.running = false; }
            ctx.request_repaint();
        }
    }

    let can_verify = state.progress.done && state.progress.bytes_copied > 0 && state.progress.error.is_none();

    if state.progress.total_bytes > 0 {
        let fraction = (state.progress.bytes_copied as f64 / state.progress.total_bytes as f64).min(1.0);
        ui.add(egui::ProgressBar::new(fraction as f32).text(format!(
            "{:.1}% — {}/{}", fraction * 100.0,
            format_size(state.progress.bytes_copied), format_size(state.progress.total_bytes),
        )));

        if state.progress.speed_bps > 0.0 {
            let remaining = state.progress.total_bytes.saturating_sub(state.progress.bytes_copied);
            let eta_secs = remaining as f64 / state.progress.speed_bps;
            ui.label(format!("Speed: {}/s   ETA: {}",
                format_size(state.progress.speed_bps as u64),
                format_duration(eta_secs as u64),
            ));
        }
    }

    if !state.progress.sha256_digest.is_empty() {
        ui.separator();
        ui.label("SHA-256:");
        ui.monospace(&state.progress.sha256_digest);
    }

    if let Some(verified) = state.progress.verified {
        if verified {
            ui.colored_label(egui::Color32::GREEN, "✓ SHA-256 verified — image is identical");
        } else {
            ui.colored_label(egui::Color32::RED, "✗ SHA-256 MISMATCH — image may be corrupted");
        }
    }

    if !state.running && can_verify {
        if ui.button("🔍 Verify SHA-256").clicked() {
            state.verify();
        }
    }

    if let Some(ref e) = &state.progress.error {
        ui.separator();
        ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
    }
}
