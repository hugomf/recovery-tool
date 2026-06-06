use crate::utils;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

const CHUNK_SIZE: usize = 4 * 1024 * 1024;
const MAX_CANDIDATES: usize = 1024;
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;
const STREAM_CHUNK: usize = 64 * 1024;

pub struct FileSig {
    pub name: &'static str,
    pub exts: &'static [&'static str],
    pub header: &'static [u8],
    pub footer: Option<&'static [u8]>,
    pub min_size: u64,
    pub enabled: bool,
}

static ALL_SIGS: &[FileSig] = &[
    FileSig { name: "ZIP/JAR/APK/AAR", exts: &["zip","jar","apk","aar","war","docx","xlsx","pptx","ipa"], header: &[0x50, 0x4B, 0x03, 0x04], footer: Some(&[0x50, 0x4B, 0x05, 0x06]), min_size: 256, enabled: true },
    FileSig { name: "GZip", exts: &["gz","tgz"], header: &[0x1F, 0x8B], footer: None, min_size: 32, enabled: true },
    FileSig { name: "7z", exts: &["7z"], header: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C], footer: None, min_size: 128, enabled: true },
    FileSig { name: "RAR", exts: &["rar"], header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07], footer: None, min_size: 128, enabled: true },
    FileSig { name: "Java Class", exts: &["class"], header: &[0xCA, 0xFE, 0xBA, 0xBE], footer: None, min_size: 64, enabled: true },
    FileSig { name: "DEX (Dalvik)", exts: &["dex"], header: b"dex\n", footer: None, min_size: 64, enabled: true },
    FileSig { name: "JPEG", exts: &["jpg","jpeg"], header: &[0xFF, 0xD8, 0xFF], footer: Some(&[0xFF, 0xD9]), min_size: 1024, enabled: true },
    FileSig { name: "PNG", exts: &["png"], header: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], footer: Some(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]), min_size: 128, enabled: true },
    FileSig { name: "GIF", exts: &["gif"], header: &[0x47, 0x49, 0x46, 0x38], footer: Some(&[0x00, 0x3B]), min_size: 64, enabled: true },
    FileSig { name: "BMP", exts: &["bmp"], header: &[0x42, 0x4D], footer: None, min_size: 128, enabled: true },
    FileSig { name: "PDF", exts: &["pdf"], header: &[0x25, 0x50, 0x44, 0x46], footer: Some(&[0x25, 0x25, 0x45, 0x4F, 0x46]), min_size: 256, enabled: true },
    FileSig { name: "RIFF (AVI/WAV)", exts: &["avi","wav"], header: &[0x52, 0x49, 0x46, 0x46], footer: None, min_size: 128, enabled: true },
    FileSig { name: "Mach-O (ARM64)", exts: &["bin"], header: &[0xCF, 0xFA, 0xED, 0xFE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_64)", exts: &["bin"], header: &[0xFE, 0xED, 0xFA, 0xCE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (ARM32)", exts: &["bin"], header: &[0xFE, 0xED, 0xFA, 0xCF], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_32)", exts: &["bin"], header: &[0xCE, 0xFA, 0xED, 0xFE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "ELF", exts: &["elf","so"], header: &[0x7F, 0x45, 0x4C, 0x46], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "WebAssembly", exts: &["wasm"], header: &[0x00, 0x61, 0x73, 0x6D], footer: None, min_size: 64, enabled: true },
    FileSig { name: "SQLite", exts: &["sqlite","db","sqlite3"], header: b"SQLite format 3\0", footer: None, min_size: 512, enabled: true },
    FileSig { name: "MP4/M4V", exts: &["mp4","m4v","mov"], header: &[0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "MP3", exts: &["mp3"], header: &[0x49, 0x44, 0x33], footer: None, min_size: 1024, enabled: true },
];

#[derive(Clone)]
pub struct CarvedFileInfo {
    pub offset: u64,
    pub size: u64,
    pub sig_name: String,
    pub ext: String,
}

#[derive(Clone, Default)]
pub struct CarverProgress {
    pub bytes_scanned: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub files_found: u64,
    pub recent: Vec<CarvedFileInfo>,
    pub done: bool,
    pub error: Option<String>,
    pub status: String,
}

struct ActiveSig {
    name: String,
    header: Vec<u8>,
    footer: Option<Vec<u8>>,
    min_size: u64,
    exts: Vec<String>,
    first_byte: u8,
}

pub struct CarverState {
    pub source: String,
    pub output_dir: String,
    pub sig_flags: Vec<bool>,
    pub scanning: bool,
    pub progress: CarverProgress,
    pub rx: Option<mpsc::Receiver<CarverProgress>>,
}

impl CarverState {
    pub fn new() -> Self {
        Self {
            sig_flags: ALL_SIGS.iter().map(|s| s.enabled).collect(),
            scanning: false,
            source: "/dev/disk3s5".into(),
            output_dir: format!("{}/carved_files", std::env::var("HOME").unwrap_or_default()),
            progress: CarverProgress {
                bytes_scanned: 0, total_bytes: 0, speed_bps: 0.0,
                files_found: 0, recent: vec![], done: false,
                error: None, status: "Ready".into(),
            },
            rx: None,
        }
    }

    pub fn start(&mut self) {
        let source = self.source.clone();
        let output_dir = self.output_dir.clone();
        let active: Vec<ActiveSig> = ALL_SIGS.iter().zip(&self.sig_flags)
            .filter(|(_, &enabled)| enabled)
            .map(|(s, _)| ActiveSig {
                name: s.name.to_string(),
                header: s.header.to_vec(),
                footer: s.footer.map(|f| f.to_vec()),
                min_size: s.min_size,
                exts: s.exts.iter().map(|e| e.to_string()).collect(),
                first_byte: s.header[0],
            })
            .collect();

        if active.is_empty() {
            self.progress.error = Some("No file types selected".into());
            self.progress.done = true;
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.scanning = true;
        self.progress = CarverProgress {
            bytes_scanned: 0, total_bytes: 0, speed_bps: 0.0,
            files_found: 0, recent: vec![], done: false,
            error: None, status: "Starting...".into(),
        };

        thread::spawn(move || {
            let _ = fs::create_dir_all(&output_dir);

            let mut file = match OpenOptions::new().read(true).open(&source) {
                Ok(f) => f,
                Err(e) => {
                    tx.send(CarverProgress {
                        bytes_scanned: 0, total_bytes: 0, speed_bps: 0.0,
                        files_found: 0, recent: vec![], done: true,
                        error: Some(format!("Cannot open {source}: {e}\nTry: sudo disk-doctor")),
                        status: "Error".into(),
                    }).ok();
                    return;
                }
            };

            let total = file.seek(SeekFrom::End(0)).unwrap_or(0);
            let _ = file.seek(SeekFrom::Start(0));

            tx.send(CarverProgress {
                bytes_scanned: 0, total_bytes: total, speed_bps: 0.0,
                files_found: 0, recent: vec![], done: false,
                error: None, status: format!("Scanning {}...", utils::format_size(total)),
            }).ok();

            let mut buffer = vec![0u8; CHUNK_SIZE];
            let mut scanned: u64 = 0;
            let start_time = std::time::Instant::now();
            let mut files_found: u64 = 0;
            let mut recent: VecDeque<CarvedFileInfo> = VecDeque::with_capacity(50);
            let mut candidates: Vec<(u64, usize)> = Vec::with_capacity(MAX_CANDIDATES);

            // Build first-byte lookup table
            let mut first_bytes = [false; 256];
            for sig in &active {
                first_bytes[sig.first_byte as usize] = true;
            }

            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];

                        // Single-pass scan using first-byte lookup
                        for (pos, &byte) in chunk.iter().enumerate() {
                            if !first_bytes[byte as usize] {
                                continue;
                            }
                            let abs_offset = scanned + pos as u64;
                            for (si, sig) in active.iter().enumerate() {
                                if byte != sig.first_byte {
                                    continue;
                                }
                                if chunk[pos..].starts_with(&sig.header) {
                                    if candidates.len() < MAX_CANDIDATES {
                                        candidates.push((abs_offset, si));
                                    }
                                    break; // only one sig can match this offset
                                }
                            }
                        }

                        // Check existing candidates for file boundaries
                        let mut i = 0;
                        while i < candidates.len() {
                            let (start_offset, si) = candidates[i];
                            let sig = &active[si];
                            let file_end = scanned + n as u64;
                            let file_size = file_end - start_offset;

                            let mut extracted = false;

                            // Check for footer match
                            if let Some(ref footer) = sig.footer {
                                if !footer.is_empty() && chunk.len() >= footer.len() {
                                    for pos in 0..=chunk.len() - footer.len() {
                                        if &chunk[pos..pos + footer.len()] == footer.as_slice() {
                                            let total_size = scanned + pos as u64 + footer.len() as u64 - start_offset;
                                            stream_extract(&source, start_offset, total_size,
                                                &output_dir, files_found, &sig.name, &sig.exts, &mut recent);
                                            files_found += 1;
                                            extracted = true;
                                            break;
                                        }
                                    }
                                }
                            }

                            if !extracted && file_size >= MAX_FILE_SIZE {
                                stream_extract(&source, start_offset, MAX_FILE_SIZE,
                                    &output_dir, files_found, &sig.name, &sig.exts, &mut recent);
                                files_found += 1;
                                extracted = true;
                            }

                            if extracted {
                                candidates.swap_remove(i);
                            } else {
                                i += 1;
                            }
                        }

                        scanned += n as u64;
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 { scanned as f64 / elapsed } else { 0.0 };

                        tx.send(CarverProgress {
                            bytes_scanned: scanned,
                            total_bytes: total,
                            speed_bps: speed,
                            files_found,
                            recent: recent.iter().cloned().collect(),
                            done: false,
                            error: None,
                            status: format!("Scanning... {:.1}%", scanned as f64 / total.max(1) as f64 * 100.0),
                        }).ok();
                    }
                    Err(e) => {
                        tx.send(CarverProgress {
                            bytes_scanned: scanned, total_bytes: total, speed_bps: 0.0,
                            files_found, recent: recent.iter().cloned().collect(), done: true,
                            error: Some(format!("Read error: {e}")),
                            status: "Error".into(),
                        }).ok();
                        return;
                    }
                }
            }

            // Extract remaining candidates
            for &(start_offset, si) in &candidates {
                let sig = &active[si];
                let file_size = scanned - start_offset;
                if file_size > sig.min_size {
                    stream_extract(&source, start_offset, file_size,
                        &output_dir, files_found, &sig.name, &sig.exts, &mut recent);
                    files_found += 1;
                }
            }

            tx.send(CarverProgress {
                bytes_scanned: scanned, total_bytes: total, speed_bps: 0.0,
                files_found, recent: recent.iter().cloned().collect(), done: true,
                error: None, status: format!("Done. Recovered {files_found} files."),
            }).ok();
        });
    }

    pub fn sig_enabled_count(&self) -> usize {
        self.sig_flags.iter().filter(|&&f| f).count()
    }
}

fn stream_extract(
    source: &str, offset: u64, size: u64,
    output_dir: &str, file_idx: u64,
    sig_name: &str, exts: &[String],
    recent: &mut VecDeque<CarvedFileInfo>,
) {
    let ext = exts.first().map(|s| s.as_str()).unwrap_or("bin");
    let filename = format!("f{:09}_{:x}.{}", file_idx + 1, offset, ext);
    let path = Path::new(output_dir).join(&filename);

    let mut src = match OpenOptions::new().read(true).open(source) {
        Ok(f) => f,
        Err(_) => return,
    };

    if src.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }

    let mut dst = match fs::File::create(&path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut remaining = size;
    let mut buf = [0u8; STREAM_CHUNK];
    while remaining > 0 {
        let to_read = (remaining as usize).min(STREAM_CHUNK);
        match src.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(n) => {
                if dst.write_all(&buf[..n]).is_err() {
                    return;
                }
                remaining -= n as u64;
            }
            Err(_) => return,
        }
    }

    if recent.len() >= 50 {
        recent.pop_front();
    }
    recent.push_back(CarvedFileInfo {
        offset, size,
        sig_name: sig_name.to_string(),
        ext: ext.to_string(),
    });
}

pub fn carver_ui(state: &mut CarverState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("🔧 File Carver");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Source:");
        ui.text_edit_singleline(&mut state.source);
    });
    ui.horizontal(|ui| {
        ui.label("Output:");
        ui.text_edit_singleline(&mut state.output_dir);
    });

    ui.separator();
    ui.label(format!("File types ({} enabled):", state.sig_enabled_count()));
    ui.horizontal(|ui| {
        if ui.button("Select All").clicked() {
            for f in &mut state.sig_flags { *f = true; }
        }
        if ui.button("Deselect All").clicked() {
            for f in &mut state.sig_flags { *f = false; }
        }
    });
    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
        for (i, sig) in ALL_SIGS.iter().enumerate() {
            ui.checkbox(&mut state.sig_flags[i], format!("{} ({})", sig.name, sig.exts.join(", ")));
        }
    });

    ui.separator();
    if !state.scanning {
        if ui.add_enabled(state.sig_enabled_count() > 0, egui::Button::new("▶ Start Carving")).clicked() {
            state.start();
        }
    } else {
        if ui.button("⏹ Stop").clicked() {
            state.scanning = false;
        }
    }

    if let Some(rx) = &state.rx {
        while let Ok(p) = rx.try_recv() {
            state.progress = p;
            if state.progress.done {
                state.scanning = false;
            }
            ctx.request_repaint();
        }
    }

    let p = &state.progress;
    if p.total_bytes > 0 {
        let frac = (p.bytes_scanned as f64 / p.total_bytes as f64).min(1.0) as f32;
        ui.add(egui::ProgressBar::new(frac).text(format!(
            "{} / {}", utils::format_size(p.bytes_scanned), utils::format_size(p.total_bytes)
        )));

        if p.speed_bps > 0.0 {
            let remaining = p.total_bytes.saturating_sub(p.bytes_scanned);
            let eta = (remaining as f64 / p.speed_bps) as u64;
            ui.label(format!("Speed: {}/s  ETA: {}",
                utils::format_size(p.speed_bps as u64), utils::format_duration(eta)));
        }
    }

    ui.label(format!("Files recovered: {}", p.files_found));

    if !p.recent.is_empty() {
        ui.separator();
        ui.strong("Recent recovered files:");
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for f in p.recent.iter().rev() {
                ui.monospace(format!(
                    "  {} {} ({})", f.sig_name, utils::format_size(f.size),
                    Path::new(&state.output_dir)
                        .join(format!("f{:09}_{:x}.{}", 0, f.offset, f.ext))
                        .display()
                ));
            }
        });
    }

    if p.done {
        ui.label(&p.status);
        if p.files_found > 0 {
            if ui.button("📂 Open output folder").clicked() {
                let _ = std::process::Command::new("open")
                    .arg(&state.output_dir)
                    .output();
            }
        }
    }

    if let Some(ref e) = p.error {
        ui.colored_label(egui::Color32::RED, e);
    }
}
