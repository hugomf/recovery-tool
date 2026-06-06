use crate::utils;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};

pub struct HexViewerState {
    pub path: String,
    pub data: Vec<u8>,
    pub offset: u64,
    pub file_size: u64,
    pub loaded: bool,
    pub error: Option<String>,
    pub go_to_offset: String,
    pub bytes_per_row: usize,
}

impl Default for HexViewerState {
    fn default() -> Self {
        Self {
            path: String::new(),
            data: Vec::new(),
            offset: 0,
            file_size: 0,
            loaded: false,
            error: None,
            go_to_offset: String::new(),
            bytes_per_row: 16,
        }
    }
}

impl HexViewerState {
    pub fn load(&mut self) {
        if self.path.is_empty() {
            self.error = Some("No file specified".into());
            return;
        }

        let mut file = match OpenOptions::new().read(true).open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(format!("Cannot open: {e}"));
                return;
            }
        };

        self.file_size = file.seek(SeekFrom::End(0)).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(self.offset));

        // Round up to 64KB but don't exceed file size
        let read_size = (64 * 1024).min(self.file_size.saturating_sub(self.offset)) as usize;
        if read_size == 0 {
            self.data = Vec::new();
            self.loaded = true;
            self.error = None;
            return;
        }

        let mut buffer = vec![0u8; read_size];
        match file.read(&mut buffer) {
            Ok(n) => {
                buffer.truncate(n);
                self.data = buffer;
                self.loaded = true;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Read error: {e}"));
            }
        }
    }

    pub fn go_offset(&mut self, offset_str: &str) {
        let trimmed = offset_str.trim();
        let offset = if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
            u64::from_str_radix(&trimmed[2..], 16).unwrap_or(0)
        } else {
            trimmed.parse::<u64>().unwrap_or(0)
        };
        self.offset = offset;
        self.load();
    }
}

pub fn hex_viewer_ui(state: &mut HexViewerState, _ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("📝 Hex Viewer");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("File:");
        ui.text_edit_singleline(&mut state.path);
        if ui.button("Open").clicked() {
            state.offset = 0;
            state.loaded = false;
            state.go_to_offset.clear();
            state.load();
        }
    });

    ui.horizontal(|ui| {
        ui.label("Go to:");
        if ui.text_edit_singleline(&mut state.go_to_offset).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            state.go_offset(&state.go_to_offset.clone());
        }
        if ui.button("Go").clicked() {
            state.go_offset(&state.go_to_offset.clone());
        }
        ui.label(format!("(file size: {})", utils::format_size(state.file_size)));
    });

    if let Some(ref e) = state.error {
        ui.colored_label(egui::Color32::RED, e);
        return;
    }

    if !state.loaded || state.data.is_empty() {
        ui.label("Open a file or device to view hex.");
        return;
    }

    let bpr = state.bytes_per_row.max(1);
    let total_rows = state.data.len().div_ceil(bpr);

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .max_height(ui.available_height())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.monospace("  Offset    ");
                for b in 0..bpr {
                    ui.monospace(format!("{b:02X} "));
                }
                ui.monospace("  ASCII");
            });

            ui.separator();

            for row in 0..total_rows {
                let start = row * bpr;
                let end = (start + bpr).min(state.data.len());
                let abs_offset = state.offset + start as u64;

                let ascii: String = state.data[start..end].iter()
                    .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                    .collect();

                ui.horizontal(|ui| {
                    ui.monospace(format!("  0x{abs_offset:08X}  "));
                    for &b in &state.data[start..end] {
                        ui.monospace(format!("{b:02X} "));
                    }
                    let remaining = bpr - (end - start);
                    for _ in 0..remaining {
                        ui.monospace("   ");
                    }
                    ui.monospace(format!("  {ascii}"));
                });
            }
        });

    // Navigation
    ui.separator();
    ui.horizontal(|ui| {
        if state.offset > 0 {
            if ui.button("⬅ Prev Page").clicked() {
                state.offset = state.offset.saturating_sub(64 * 1024);
                state.load();
            }
        }
        if state.offset + 64 * 1024 < state.file_size {
            if ui.button("Next Page ➡").clicked() {
                state.offset = (state.offset + 64 * 1024).min(state.file_size.saturating_sub(1));
                state.load();
            }
        }
    });
}
