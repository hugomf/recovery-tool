use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

// File signatures for carving (magic bytes)
pub struct FileSig {
    pub name: &'static str,
    pub exts: &'static [&'static str],
    pub header: &'static [u8],
    pub footer: Option<&'static [u8]>,
    pub min_size: u64,
    pub enabled: bool,
}

static ALL_SIGS: &[FileSig] = &[
    // Archives / packages
    FileSig { name: "ZIP/JAR/APK/AAR", exts: &["zip","jar","apk","aar","war","docx","xlsx","pptx","ipa"], header: &[0x50, 0x4B, 0x03, 0x04], footer: Some(&[0x50, 0x4B, 0x05, 0x06]), min_size: 256, enabled: true },
    FileSig { name: "GZip", exts: &["gz","tgz"], header: &[0x1F, 0x8B], footer: None, min_size: 32, enabled: true },
    FileSig { name: "TAR", exts: &["tar"], header: &[], footer: None, min_size: 1024, enabled: false },
    FileSig { name: "7z", exts: &["7z"], header: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C], footer: None, min_size: 128, enabled: true },
    FileSig { name: "RAR", exts: &["rar"], header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07], footer: None, min_size: 128, enabled: true },

    // Java / JVM
    FileSig { name: "Java Class", exts: &["class"], header: &[0xCA, 0xFE, 0xBA, 0xBE], footer: None, min_size: 64, enabled: true },
    FileSig { name: "DEX (Dalvik)", exts: &["dex"], header: b"dex\n", footer: None, min_size: 64, enabled: true },

    // Images
    FileSig { name: "JPEG", exts: &["jpg","jpeg"], header: &[0xFF, 0xD8, 0xFF], footer: Some(&[0xFF, 0xD9]), min_size: 1024, enabled: true },
    FileSig { name: "PNG", exts: &["png"], header: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], footer: Some(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]), min_size: 128, enabled: true },
    FileSig { name: "GIF", exts: &["gif"], header: &[0x47, 0x49, 0x46, 0x38], footer: Some(&[0x00, 0x3B]), min_size: 64, enabled: true },
    FileSig { name: "BMP", exts: &["bmp"], header: &[0x42, 0x4D], footer: None, min_size: 128, enabled: true },

    // Documents
    FileSig { name: "PDF", exts: &["pdf"], header: &[0x25, 0x50, 0x44, 0x46], footer: Some(&[0x25, 0x25, 0x45, 0x4F, 0x46]), min_size: 256, enabled: true },
    FileSig { name: "RIFF (AVI/WAV)", exts: &["avi","wav"], header: &[0x52, 0x49, 0x46, 0x46], footer: None, min_size: 128, enabled: true },

    // Binaries
    FileSig { name: "Mach-O (ARM64)", exts: &["bin"], header: &[0xCF, 0xFA, 0xED, 0xFE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_64)", exts: &["bin"], header: &[0xFE, 0xED, 0xFA, 0xCE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (ARM32)", exts: &["bin"], header: &[0xFE, 0xED, 0xFA, 0xCF], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_32)", exts: &["bin"], header: &[0xCE, 0xFA, 0xED, 0xFE], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "ELF", exts: &["elf","so"], header: &[0x7F, 0x45, 0x4C, 0x46], footer: None, min_size: 4096, enabled: true },
    FileSig { name: "WebAssembly", exts: &["wasm"], header: &[0x00, 0x61, 0x73, 0x6D], footer: None, min_size: 64, enabled: true },

    // Database
    FileSig { name: "SQLite", exts: &["sqlite","db","sqlite3"], header: b"SQLite format 3\0", footer: None, min_size: 512, enabled: true },
    FileSig { name: "LevelDB/LMDB", exts: &["ldb"], header: &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], footer: None, min_size: 4096, enabled: false },

    // Media
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

#[derive(Default)]
pub struct CarverState {
    pub source: String,
    pub output_dir: String,
    pub sigs: Vec<FileSig>,
    pub scanning: bool,
    pub progress: CarverProgress,
    pub rx: Option<mpsc::Receiver<CarverProgress>>,
}

impl CarverState {
    pub fn new() -> Self {
        Self {
            sigs: ALL_SIGS.iter().map(|s| FileSig {
                name: s.name, exts: s.exts, header: s.header,
                footer: s.footer, min_size: s.min_size, enabled: s.enabled,
            }).collect(),
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
        let active_sigs: Vec<(String, Vec<u8>, Option<Vec<u8>>, u64, Vec<String>)> = self.sigs.iter()
            .filter(|s| s.enabled)
            .map(|s| (s.name.to_string(), s.header.to_vec(), s.footer.map(|f| f.to_vec()), s.min_size, s.exts.iter().map(|e| e.to_string()).collect()))
            .collect();

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

            let total = match file.seek(SeekFrom::End(0)) {
                Ok(len) => {
                    let _ = file.seek(SeekFrom::Start(0));
                    len
                }
                Err(_) => 0,
            };

            tx.send(CarverProgress {
                bytes_scanned: 0, total_bytes: total, speed_bps: 0.0,
                files_found: 0, recent: vec![], done: false,
                error: None, status: format!("Scanning {}...", format_size(total)),
            }).ok();

            let mut buffer = vec![0u8; 4 * 1024 * 1024]; // 4MB chunks
            let mut scanned: u64 = 0;
            let start_time = std::time::Instant::now();
            let mut files_found: u64 = 0;
            let mut recent: Vec<CarvedFileInfo> = Vec::new();

            // Track candidate buffer for multi-chunk headers
            let mut candidates: Vec<(u64, usize, Vec<u8>, Option<Vec<u8>>, u64, Vec<String>)> = Vec::new();

            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];
                        let sig_offset = scanned;

                        // Check for header matches in this chunk
                        for (si, (sname, sheader, sfooter, smin, sexts)) in active_sigs.iter().enumerate() {
                            if sheader.is_empty() { continue; }
                            // Slide through the chunk looking for header
                            let hdr = sheader.as_slice();
                            if chunk.len() < hdr.len() { continue; }

                            let mut pos = 0;
                            while pos + hdr.len() <= chunk.len() {
                                if &chunk[pos..pos + hdr.len()] == hdr {
                                    candidates.push((
                                        sig_offset + pos as u64,
                                        si,
                                        sheader.clone(),
                                        sfooter.clone(),
                                        *smin,
                                        sexts.clone(),
                                    ));
                                }
                                pos += 1;
                            }
                        }

                        // For each candidate, check if we passed its max reasonable size (100MB default)
                        // or found a footer
                        let mut settled: Vec<usize> = Vec::new();
                        for (ci, (start_offset, si, sheader, sfooter, smin, sexts)) in candidates.iter().enumerate() {
                            let file_size = scanned + n as u64 - start_offset;

                            // Check for footer match in this chunk
                            let mut footer_found = false;
                            if let Some(footer) = sfooter {
                                if !footer.is_empty() && chunk.len() >= footer.len() {
                                    for pos in 0..=chunk.len() - footer.len() {
                                        if &chunk[pos..pos + footer.len()] == footer.as_slice() {
                                            footer_found = true;
                                            let total_size = scanned + pos as u64 + footer.len() as u64 - start_offset;
                                            extract_file(&source, *start_offset, total_size,
                                                &output_dir, files_found, si, sexts, &mut recent, &tx);
                                            files_found += 1;
                                            settled.push(ci);
                                            break;
                                        }
                                    }
                                }
                            }

                            // If no footer, cap at 50MB or next header
                            if !footer_found && file_size > 50 * 1024 * 1024 {
                                extract_file(&source, *start_offset, file_size,
                                    &output_dir, files_found, si, sexts, &mut recent, &tx);
                                files_found += 1;
                                settled.push(ci);
                            }
                        }

                        // Remove settled candidates (in reverse order)
                        for ci in settled.into_iter().rev() {
                            if ci < candidates.len() {
                                candidates.remove(ci);
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
                            recent: recent.clone(),
                            done: false,
                            error: None,
                            status: format!("Scanning... {:.1}%", scanned as f64 / total.max(1) as f64 * 100.0),
                        }).ok();
                    }
                    Err(e) => {
                        tx.send(CarverProgress {
                            bytes_scanned: scanned, total_bytes: total, speed_bps: 0.0,
                            files_found, recent: recent.clone(), done: true,
                            error: Some(format!("Read error: {e}")),
                            status: "Error".into(),
                        }).ok();
                        return;
                    }
                }
            }

            // Final: extract any remaining candidates
            for (start_offset, si, _sh, _sf, _sm, sexts) in &candidates {
                let file_size = scanned - start_offset;
                if file_size > 0 {
                    extract_file(&source, *start_offset, file_size,
                        &output_dir, files_found, si, sexts, &mut recent, &tx);
                    files_found += 1;
                }
            }

            tx.send(CarverProgress {
                bytes_scanned: scanned, total_bytes: total, speed_bps: 0.0,
                files_found, recent, done: true, error: None,
                status: format!("Done. Recovered {files_found} files."),
            }).ok();
        });
    }

}

fn extract_file(
    source: &str, offset: u64, size: u64,
    output_dir: &str, file_idx: u64, sig_idx: &usize,
    exts: &[String], recent: &mut Vec<CarvedFileInfo>,
    tx: &mpsc::Sender<CarverProgress>,
) {
    let ext = exts.first().map(|s| s.as_str()).unwrap_or("bin");
    let filename = format!("f{:09}_{:x}.{}", file_idx + 1, offset, ext);
    let path = Path::new(output_dir).join(&filename);

    let mut src = match OpenOptions::new().read(true).open(source) {
        Ok(f) => f,
        Err(_) => return,
    };

    if let Err(_) = src.seek(SeekFrom::Start(offset)) {
        return;
    }

    let mut buffer = vec![0u8; size as usize];
    if let Err(_) = src.read_exact(&mut buffer) {
        return;
    }

    if let Err(_) = fs::write(&path, &buffer) {
        return;
    }

    let info = CarvedFileInfo {
        offset,
        size,
        sig_name: "carved".into(),
        ext: ext.to_string(),
    };
    recent.push(info);
    if recent.len() > 50 {
        recent.remove(0);
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
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
    ui.label("File types to scan for:");
    let sig_names: Vec<String> = state.sigs.iter()
        .map(|s| format!("{} ({})", s.name, s.exts.join(", ")))
        .collect();
    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
        for i in 0..state.sigs.len() {
            let mut enabled = state.sigs[i].enabled;
            ui.checkbox(&mut enabled, &sig_names[i]);
            if enabled != state.sigs[i].enabled {
                state.sigs[i].enabled = enabled;
            }
        }
    });

    ui.separator();
    if !state.scanning {
        if ui.button("▶ Start Carving").clicked() {
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
            "{} / {}", format_size(p.bytes_scanned), format_size(p.total_bytes)
        )));

        if p.speed_bps > 0.0 {
            let remaining = p.total_bytes.saturating_sub(p.bytes_scanned);
            let eta = if p.speed_bps > 0.0 { remaining as f64 / p.speed_bps } else { 0.0 };
            ui.label(format!("Speed: {}/s  ETA: {}",
                format_size(p.speed_bps as u64), format_eta(eta as u64)));
        }
    }

    ui.label(format!("Files recovered: {}", p.files_found));

    if !p.recent.is_empty() {
        ui.separator();
        ui.strong("Recent recovered files:");
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for f in p.recent.iter().rev() {
                ui.monospace(format!(
                    "  {} ({})", format_size(f.size),
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

fn format_eta(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") }
    else if secs < 3600 { format!("{}m {}s", secs / 60, secs % 60) }
    else { format!("{}h {}m", secs / 3600, (secs % 3600) / 60) }
}
