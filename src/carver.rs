use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use crate::utils::{format_duration, format_size};

const MAX_CANDIDATES: usize = 512;
const NO_FOOTER_CAP: u64 = 50 * 1024 * 1024;

pub struct FileSig {
    pub name: &'static str,
    pub exts: &'static [&'static str],
    pub header: &'static [u8],
    pub footer: Option<&'static [u8]>,
    pub min_size: u64,
    pub enabled: bool,
}

static ALL_SIGS: &[FileSig] = &[
    FileSig { name: "ZIP/JAR/APK/AAR",   exts: &["zip","jar","apk","aar","war","docx","xlsx","pptx","ipa"], header: &[0x50, 0x4B, 0x03, 0x04], footer: Some(&[0x50, 0x4B, 0x05, 0x06]), min_size: 256, enabled: true },
    FileSig { name: "GZip",              exts: &["gz","tgz"],            header: &[0x1F, 0x8B],                              footer: None,                                               min_size: 32,   enabled: true },
    FileSig { name: "TAR",               exts: &["tar"],                 header: &[],                                         footer: None,                                               min_size: 1024, enabled: false },
    FileSig { name: "7z",                exts: &["7z"],                  header: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],      footer: None,                                               min_size: 128,  enabled: true },
    FileSig { name: "RAR",               exts: &["rar"],                 header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07],      footer: None,                                               min_size: 128,  enabled: true },
    FileSig { name: "Java Class",        exts: &["class"],               header: &[0xCA, 0xFE, 0xBA, 0xBE],                  footer: None,                                               min_size: 64,   enabled: true },
    FileSig { name: "DEX (Dalvik)",      exts: &["dex"],                 header: b"dex\n",                                    footer: None,                                               min_size: 64,   enabled: true },
    FileSig { name: "JPEG",              exts: &["jpg","jpeg"],          header: &[0xFF, 0xD8, 0xFF],                         footer: Some(&[0xFF, 0xD9]),                                min_size: 1024, enabled: true },
    FileSig { name: "PNG",               exts: &["png"],                 header: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], footer: Some(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]), min_size: 128, enabled: true },
    FileSig { name: "GIF",               exts: &["gif"],                 header: &[0x47, 0x49, 0x46, 0x38],                  footer: Some(&[0x00, 0x3B]),                                min_size: 64,   enabled: true },
    FileSig { name: "BMP",               exts: &["bmp"],                 header: &[0x42, 0x4D],                               footer: None,                                               min_size: 128,  enabled: true },
    FileSig { name: "PDF",               exts: &["pdf"],                 header: &[0x25, 0x50, 0x44, 0x46],                  footer: Some(&[0x25, 0x25, 0x45, 0x4F, 0x46]),             min_size: 256,  enabled: true },
    FileSig { name: "RIFF (AVI/WAV)",    exts: &["avi","wav"],           header: &[0x52, 0x49, 0x46, 0x46],                  footer: None,                                               min_size: 128,  enabled: true },
    FileSig { name: "Mach-O (ARM64)",    exts: &["bin"],                 header: &[0xCF, 0xFA, 0xED, 0xFE],                  footer: None,                                               min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_64)",   exts: &["bin"],                 header: &[0xFE, 0xED, 0xFA, 0xCE],                  footer: None,                                               min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (ARM32)",    exts: &["bin"],                 header: &[0xFE, 0xED, 0xFA, 0xCF],                  footer: None,                                               min_size: 4096, enabled: true },
    FileSig { name: "Mach-O (x86_32)",   exts: &["bin"],                 header: &[0xCE, 0xFA, 0xED, 0xFE],                  footer: None,                                               min_size: 4096, enabled: true },
    FileSig { name: "ELF",               exts: &["elf","so"],            header: &[0x7F, 0x45, 0x4C, 0x46],                  footer: None,                                               min_size: 4096, enabled: true },
    FileSig { name: "WebAssembly",        exts: &["wasm"],                header: &[0x00, 0x61, 0x73, 0x6D],                  footer: None,                                               min_size: 64,   enabled: true },
    FileSig { name: "SQLite",            exts: &["sqlite","db","sqlite3"], header: b"SQLite format 3\0",                      footer: None,                                               min_size: 512,  enabled: true },
    FileSig { name: "LevelDB/LMDB",      exts: &["ldb"],                 header: &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], footer: None,                                      min_size: 4096, enabled: false },
    FileSig { name: "MP4/M4V",           exts: &["mp4","m4v","mov"],     header: &[0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70], footer: None,                                      min_size: 4096, enabled: true },
    FileSig { name: "MP3",               exts: &["mp3"],                 header: &[0x49, 0x44, 0x33],                         footer: None,                                               min_size: 1024, enabled: true },
];

#[derive(Clone)]
pub struct CarvedFileInfo {
    pub offset: u64,
    pub size: u64,
    pub sig_name: String,
    pub ext: String,
    pub filename: String,
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
    pub candidates_dropped: u64,
}

struct SigInfo {
    name: String,
    exts: Vec<String>,
    header: Vec<u8>,
    footer: Option<Vec<u8>>,
    min_size: u64,
}

pub struct CarverState {
    pub source: String,
    pub output_dir: String,
    pub sig_enabled: Vec<bool>,
    pub scanning: bool,
    pub progress: CarverProgress,
    pub rx: Option<mpsc::Receiver<CarverProgress>>,
}

impl CarverState {
    pub fn new() -> Self {
        Self {
            sig_enabled: ALL_SIGS.iter().map(|s| s.enabled).collect(),
            scanning: false,
            source: "/dev/disk3s5".into(),
            output_dir: format!("{}/carved_files", std::env::var("HOME").unwrap_or_default()),
            progress: CarverProgress { status: "Ready".into(), ..Default::default() },
            rx: None,
        }
    }

    fn active_sigs(&self) -> Vec<SigInfo> {
        ALL_SIGS.iter().zip(&self.sig_enabled)
            .filter(|(_, &en)| en)
            .map(|(s, _)| SigInfo {
                name: s.name.to_string(),
                exts: s.exts.iter().map(|e| e.to_string()).collect(),
                header: s.header.to_vec(),
                footer: s.footer.map(|f| f.to_vec()),
                min_size: s.min_size,
            })
            .collect()
    }

    pub fn start(&mut self) {
        let source = self.source.clone();
        let output_dir = self.output_dir.clone();
        let active_sigs = self.active_sigs();

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.scanning = true;
        self.progress = CarverProgress { status: "Starting...".into(), ..Default::default() };

        thread::spawn(move || {
            let _ = fs::create_dir_all(&output_dir);

            let mut file = match OpenOptions::new().read(true).open(&source) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.send(CarverProgress {
                        done: true,
                        error: Some(format!("Cannot open {source}: {e}\nTry: sudo disk-doctor")),
                        status: "Error".into(),
                        ..Default::default()
                    });
                    return;
                }
            };

            let total = file.seek(SeekFrom::End(0)).unwrap_or(0);
            let _ = file.seek(SeekFrom::Start(0));

            let _ = tx.send(CarverProgress {
                total_bytes: total,
                status: format!("Scanning {}...", format_size(total)),
                ..Default::default()
            });

            let mut candidates: Vec<Vec<u64>> = vec![Vec::new(); active_sigs.len()];
            let mut buffer = vec![0u8; 4 * 1024 * 1024];
            let mut scanned: u64 = 0;
            let start_time = std::time::Instant::now();
            let mut files_found: u64 = 0;
            let mut recent: VecDeque<CarvedFileInfo> = VecDeque::new();
            let mut candidates_dropped: u64 = 0;

            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];

                        for (si, sig) in active_sigs.iter().enumerate() {
                            if sig.header.is_empty() { continue; }
                            let first_byte = sig.header[0];
                            let hdr = &sig.header;
                            let mut pos = 0;

                            while pos + hdr.len() <= chunk.len() {
                                if chunk[pos] == first_byte
                                    && &chunk[pos..pos + hdr.len()] == hdr.as_slice()
                                {
                                    if candidates[si].len() < MAX_CANDIDATES {
                                        candidates[si].push(scanned + pos as u64);
                                    } else {
                                        candidates_dropped += 1;
                                    }
                                    pos += hdr.len();
                                } else {
                                    pos += 1;
                                }
                            }
                        }

                        for (si, sig) in active_sigs.iter().enumerate() {
                            let mut settled: Vec<usize> = Vec::new();

                            for (ci, &start_offset) in candidates[si].iter().enumerate() {
                                let current_size = scanned + n as u64 - start_offset;

                                if let Some(footer) = &sig.footer {
                                    if !footer.is_empty() && chunk.len() >= footer.len() {
                                        for fpos in 0..=chunk.len() - footer.len() {
                                            if &chunk[fpos..fpos + footer.len()] == footer.as_slice() {
                                                let size = scanned + fpos as u64 + footer.len() as u64 - start_offset;
                                                if size >= sig.min_size {
                                                    extract_file_streaming(
                                                        &source, start_offset, size, &output_dir,
                                                        files_found, sig, &mut recent,
                                                    );
                                                    files_found += 1;
                                                }
                                                settled.push(ci);
                                                break;
                                            }
                                        }
                                    }
                                }

                                if sig.footer.is_none() && current_size >= NO_FOOTER_CAP {
                                    if current_size >= sig.min_size {
                                        extract_file_streaming(
                                            &source, start_offset, NO_FOOTER_CAP, &output_dir,
                                            files_found, sig, &mut recent,
                                        );
                                        files_found += 1;
                                    }
                                    settled.push(ci);
                                }
                            }

                            for ci in settled.into_iter().rev() {
                                if ci < candidates[si].len() {
                                    candidates[si].remove(ci);
                                }
                            }
                        }

                        scanned += n as u64;

                        let elapsed = start_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 { scanned as f64 / elapsed } else { 0.0 };
                        let _ = tx.send(CarverProgress {
                            bytes_scanned: scanned,
                            total_bytes: total,
                            speed_bps: speed,
                            files_found,
                            recent: recent.iter().cloned().collect(),
                            status: format!("Scanning... {:.1}%", scanned as f64 / total.max(1) as f64 * 100.0),
                            candidates_dropped,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(CarverProgress {
                            bytes_scanned: scanned, total_bytes: total,
                            files_found, recent: recent.iter().cloned().collect(),
                            done: true,
                            error: Some(format!("Read error: {e}")),
                            status: "Error".into(),
                            candidates_dropped,
                            ..Default::default()
                        });
                        return;
                    }
                }
            }

            for (si, sig) in active_sigs.iter().enumerate() {
                for &start_offset in &candidates[si] {
                    let size = scanned - start_offset;
                    if size >= sig.min_size {
                        extract_file_streaming(
                            &source, start_offset, size, &output_dir,
                            files_found, sig, &mut recent,
                        );
                        files_found += 1;
                    }
                }
            }

            let _ = tx.send(CarverProgress {
                bytes_scanned: scanned, total_bytes: total,
                files_found, recent: recent.iter().cloned().collect(),
                done: true,
                status: format!("Done. Recovered {files_found} files."),
                candidates_dropped,
                ..Default::default()
            });
        });
    }
}

fn extract_file_streaming(
    source: &str,
    offset: u64,
    size: u64,
    output_dir: &str,
    file_idx: u64,
    sig: &SigInfo,
    recent: &mut VecDeque<CarvedFileInfo>,
) {
    let ext = sig.exts.first().map(|s| s.as_str()).unwrap_or("bin");
    let filename = format!("f{:09}_{:x}.{}", file_idx + 1, offset, ext);
    let path = Path::new(output_dir).join(&filename);

    let mut src = match OpenOptions::new().read(true).open(source) {
        Ok(f) => f,
        Err(_) => return,
    };
    if src.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }

    let dst_file = match fs::File::create(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut dst = std::io::BufWriter::new(dst_file);
    let mut buf = vec![0u8; 64 * 1024];
    let mut remaining = size;

    while remaining > 0 {
        let to_read = buf.len().min(remaining as usize);
        match src.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(n) => {
                if dst.write_all(&buf[..n]).is_err() { return; }
                remaining -= n as u64;
            }
            Err(_) => return,
        }
    }
    let _ = dst.flush();

    recent.push_back(CarvedFileInfo {
        offset, size,
        sig_name: sig.name.clone(),
        ext: ext.to_string(),
        filename: filename.clone(),
    });
    if recent.len() > 50 {
        recent.pop_front();
    }
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

    let enabled_count = state.sig_enabled.iter().filter(|&&e| e).count();
    let total_count = state.sig_enabled.len();
    ui.horizontal(|ui| {
        ui.label(format!("File types: {enabled_count}/{total_count} enabled"));
        if ui.small_button("All").clicked() {
            state.sig_enabled.iter_mut().for_each(|e| *e = true);
        }
        if ui.small_button("None").clicked() {
            state.sig_enabled.iter_mut().for_each(|e| *e = false);
        }
    });

    egui::ScrollArea::vertical().max_height(150.0).id_source("sig_list").show(ui, |ui| {
        for (i, sig) in ALL_SIGS.iter().enumerate() {
            ui.checkbox(&mut state.sig_enabled[i], format!("{} ({})", sig.name, sig.exts.join(", ")));
        }
    });

    ui.separator();

    if !state.scanning {
        if ui.add_enabled(!state.source.is_empty() && enabled_count > 0, egui::Button::new("▶ Start Carving")).clicked() {
            state.start();
        }
    } else if ui.button("⏹ Stop").clicked() {
        state.scanning = false;
    }

    if let Some(rx) = &state.rx {
        while let Ok(p) = rx.try_recv() {
            state.progress = p;
            if state.progress.done { state.scanning = false; }
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
            let eta = remaining as f64 / p.speed_bps;
            ui.label(format!("Speed: {}/s   ETA: {}", format_size(p.speed_bps as u64), format_duration(eta as u64)));
        }
    }

    ui.horizontal(|ui| {
        ui.label(format!("Files recovered: {}", p.files_found));
        if p.candidates_dropped > 0 {
            ui.colored_label(egui::Color32::YELLOW, format!("⚠ {} dropped (cap)", p.candidates_dropped));
        }
    });

    if !p.recent.is_empty() {
        ui.separator();
        ui.strong("Recent recovered files:");
        egui::ScrollArea::vertical().max_height(200.0).id_source("recent_scroll").show(ui, |ui| {
            for f in p.recent.iter().rev() {
                ui.horizontal(|ui| {
                    ui.monospace(format_size(f.size));
                    ui.label(format!("[{}]", f.sig_name));
                    ui.monospace(&f.filename);
                });
            }
        });
    }

    if p.done {
        ui.label(&p.status);
        if p.files_found > 0 && ui.button("📂 Open output folder").clicked() {
            let _ = std::process::Command::new("open").arg(&state.output_dir).output();
        }
    }

    if let Some(ref e) = p.error {
        ui.separator();
        ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
    }
}
