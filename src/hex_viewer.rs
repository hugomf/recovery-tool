use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};

use crate::utils::format_size;

const PAGE_SIZE: u64 = 64 * 1024;

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
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            self.error = Some("Seek failed".into());
            return;
        }

        let read_size = PAGE_SIZE.min(self.file_size.saturating_sub(self.offset)) as usize;
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
        self.offset = offset.min(self.file_size.saturating_sub(1));
        self.load();
    }
}

pub fn hex_viewer_ui(state: &mut HexViewerState, _ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("📝 Hex Viewer");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("File:");
        let response = ui.text_edit_singleline(&mut state.path);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Open").clicked() || enter {
            state.offset = 0;
            state.loaded = false;
            state.go_to_offset.clear();
            state.load();
        }
    });

    ui.horizontal(|ui| {
        ui.label("Go to offset:");
        let response = ui.text_edit_singleline(&mut state.go_to_offset);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Go").clicked() || enter {
            let s = state.go_to_offset.clone();
            state.go_offset(&s);
        }

        if state.file_size > 0 {
            ui.separator();
            ui.label(format!("File size: {}", format_size(state.file_size)));
            ui.label(format!(
                "  Page: 0x{:X}–0x{:X}",
                state.offset,
                (state.offset + state.data.len() as u64).min(state.file_size)
            ));
        }

        ui.separator();
        ui.label("Width:");
        for &w in &[8usize, 16, 32] {
            if ui.selectable_label(state.bytes_per_row == w, w.to_string()).clicked() {
                state.bytes_per_row = w;
            }
        }
    });

    if let Some(ref e) = state.error.clone() {
        ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        return;
    }

    if !state.loaded || state.data.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label("Open a file or device path (e.g. /dev/disk0) to view hex.");
        });
        return;
    }

    let bpr = state.bytes_per_row.max(1);
    let total_rows = (state.data.len() + bpr - 1) / bpr;

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.monospace(format!("{:12}", "Offset"));
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
                let row_bytes = &state.data[start..end];

                let ascii: String = row_bytes
                    .iter()
                    .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                    .collect();

                ui.horizontal(|ui| {
                    ui.monospace(format!("0x{abs_offset:08X}  "));
                    for b in row_bytes {
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

    ui.separator();
    ui.horizontal(|ui| {
        let at_start = state.offset == 0;
        if ui.add_enabled(!at_start, egui::Button::new("⬅ Prev")).clicked() {
            state.offset = state.offset.saturating_sub(PAGE_SIZE);
            state.load();
        }

        let at_end = state.offset + PAGE_SIZE >= state.file_size;
        if ui.add_enabled(!at_end, egui::Button::new("Next ➡")).clicked() {
            state.offset = (state.offset + PAGE_SIZE).min(state.file_size.saturating_sub(1));
            state.load();
        }

        if state.file_size > 0 {
            let page = state.offset / PAGE_SIZE + 1;
            let total_pages = (state.file_size + PAGE_SIZE - 1) / PAGE_SIZE;
            ui.label(format!("Page {page}/{total_pages}"));
        }
    });
}
