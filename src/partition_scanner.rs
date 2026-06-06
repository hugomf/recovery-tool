use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::sync::mpsc;
use std::thread;

const SECTOR_SIZE: u64 = 512;
const MAX_SCAN_SECTORS: u64 = 100_000; // ~50MB — covers all common partition layouts

struct PartSig {
    name: &'static str,
    sig: &'static [u8],
    offset_in_sector: usize,
}

static PART_SIGS: &[PartSig] = &[
    PartSig { name: "GPT Header", sig: b"EFI PART", offset_in_sector: 0 },
    PartSig { name: "APFS Container (NXSB)", sig: b"NXSB", offset_in_sector: 0 },
    PartSig { name: "APFS Volume (APSB)", sig: b"APSB", offset_in_sector: 0 },
    PartSig { name: "HFS+ (H+)", sig: b"H+", offset_in_sector: 0 },
    PartSig { name: "HFS+ (HX)", sig: b"HX", offset_in_sector: 0 },
    PartSig { name: "exFAT", sig: b"EXFAT", offset_in_sector: 0 },
    PartSig { name: "FAT32", sig: b"MSDOS5.0", offset_in_sector: 0 },
    PartSig { name: "NTFS", sig: b"NTFS    ", offset_in_sector: 3 },
];

#[derive(Clone)]
pub struct FoundPartition {
    pub offset: u64,
    pub sector: u64,
    pub sig_name: String,
}

#[derive(Clone, Default)]
pub struct PartitionScanProgress {
    pub sectors_scanned: u64,
    pub total_sectors: u64,
    pub found: Vec<FoundPartition>,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct PartitionScanState {
    pub device: String,
    pub scanning: bool,
    pub progress: PartitionScanProgress,
    pub rx: Option<mpsc::Receiver<PartitionScanProgress>>,
}

impl PartitionScanState {
    pub fn start(&mut self) {
        let device = self.device.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.scanning = true;
        self.progress = PartitionScanProgress {
            sectors_scanned: 0, total_sectors: 0,
            found: Vec::new(), done: false, error: None,
        };

        thread::spawn(move || {
            let mut file = match OpenOptions::new().read(true).open(&device) {
                Ok(f) => f,
                Err(e) => {
                    tx.send(PartitionScanProgress {
                        sectors_scanned: 0, total_sectors: 0,
                        found: vec![], done: true,
                        error: Some(format!("Cannot open {device}: {e}")),
                    }).ok();
                    return;
                }
            };

            let total_size = file.seek(SeekFrom::End(0)).unwrap_or(0);
            let total_sectors = total_size / SECTOR_SIZE;
            let _ = file.seek(SeekFrom::Start(0));

            let mut chunk = vec![0u8; 4096 * SECTOR_SIZE as usize]; // 2MB chunks
            let mut sector: u64 = 0;
            let mut found = Vec::new();

            loop {
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk_sectors = n as u64 / SECTOR_SIZE;
                        for s in 0..chunk_sectors {
                            let offset = (sector + s) * SECTOR_SIZE;
                            let start = (s * SECTOR_SIZE) as usize;
                            if start + 512 > n { break; }
                            let sector_data = &chunk[start..start + 512];

                            for sig in PART_SIGS {
                                if sig.offset_in_sector + sig.sig.len() <= 512 {
                                    let check_start = sig.offset_in_sector;
                                    let check_end = check_start + sig.sig.len();
                                    if &sector_data[check_start..check_end] == sig.sig {
                                        found.push(FoundPartition {
                                            offset,
                                            sector: sector + s,
                                            sig_name: sig.name.to_string(),

                                        });
                                    }
                                }
                            }
                        }

                        let scanned = sector + chunk_sectors;
                        sector = scanned;

                        tx.send(PartitionScanProgress {
                            sectors_scanned: scanned,
                            total_sectors,
                            found: found.clone(),
                            done: false,
                            error: None,
                        }).ok();

                        if scanned >= MAX_SCAN_SECTORS {
                            break;
                        }
                    }
                    Err(e) => {
                        tx.send(PartitionScanProgress {
                            sectors_scanned: sector, total_sectors,
                            found: found.clone(), done: true,
                            error: Some(format!("Read error at sector {sector}: {e}")),
                        }).ok();
                        return;
                    }
                }
            }

            tx.send(PartitionScanProgress {
                sectors_scanned: sector, total_sectors,
                found, done: true, error: None,
            }).ok();
        });
    }
}

pub fn partition_scan_ui(state: &mut PartitionScanState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("🔍 Partition Scanner");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Device:");
        ui.text_edit_singleline(&mut state.device);
        if !state.scanning {
            if ui.add_enabled(!state.device.is_empty(), egui::Button::new("Scan")).clicked() {
                state.start();
            }
        } else {
            if ui.button("Stop").clicked() {
                state.scanning = false;
            }
        }
    });

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
    if state.scanning || p.total_sectors > 0 {
        let limit = p.total_sectors.min(MAX_SCAN_SECTORS);
        if limit > 0 {
            let frac = (p.sectors_scanned as f64 / limit as f64).min(1.0) as f32;
            ui.add(egui::ProgressBar::new(frac).text(format!(
                "Scanning... {} sectors / {} (limit: {}M)",
                p.sectors_scanned, limit, limit / 1_000_000
            )));
        } else {
            ui.spinner();
            ui.label("Scanning...");
        }

        if !p.found.is_empty() {
            ui.separator();
            ui.strong(format!("Found {} partition signatures:", p.found.len()));
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                egui::Grid::new("partitions").striped(true).show(ui, |ui| {
                    ui.strong("Offset"); ui.strong("Sector"); ui.strong("Type");
                    ui.end_row();
                    for fp in &p.found {
                        ui.monospace(format!("0x{:X}", fp.offset));
                        ui.monospace(fp.sector.to_string());
                        ui.label(&fp.sig_name);
                        ui.end_row();
                    }
                });
            });
        }
    }

    if let Some(ref e) = p.error {
        ui.colored_label(egui::Color32::RED, e);
    }
}
