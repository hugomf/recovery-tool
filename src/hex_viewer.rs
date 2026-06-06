use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};

#[derive(Default)]
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

        let read_size = (64 * 1024).min(self.file_size.saturating_sub(self.offset)) as usize;
        let mut buffer = vec![0u8; read_size];
        match file.read_exact(&mut buffer) {
            Ok(_) => {
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

pub fn hex_viewer_ui(state: &mut HexViewerState, ctx: &egui::Context, ui: &mut egui::Ui) {
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
        ui.label(format!("(file size: {})", format_size(state.file_size)));
    });

    if let Some(ref e) = state.error {
        ui.colored_label(egui::Color32::RED, e);
        return;
    }

    if !state.loaded || state.data.is_empty() {
        ui.label("Open a file or device to view hex.");
        return;
    }

    // Pagination
    let row_height = 18.0;
    let viewport_height = ui.available_height() - 40.0;
    let visible_rows = (viewport_height / row_height).max(10.0) as usize;
    let total_rows = state.data.len() / state.bytes_per_row + 1;
    let scroll = egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .max_height(ui.available_height());

    scroll.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.monospace("  Offset    ");
            for b in 0..state.bytes_per_row {
                ui.monospace(format!("{b:02X} "));
            }
            ui.monospace("  ASCII");
        });

        let ascii_offset = 12 + state.bytes_per_row * 3;
        ui.separator();

        egui::ScrollArea::vertical().id_source("hex_scroll").show(ui, |ui| {
            for row in 0..total_rows {
                let start = row * state.bytes_per_row;
                let end = (start + state.bytes_per_row).min(state.data.len());
                let abs_offset = state.offset + start as u64;

                let ascii: String = state.data[start..end].iter()
                    .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                    .collect();

                ui.horizontal(|ui| {
                    ui.monospace(format!("  0x{abs_offset:08X}  "));
                    for b in &state.data[start..end] {
                        ui.monospace(format!("{b:02X} "));
                    }
                    // Pad remaining space
                    let remaining = state.bytes_per_row - (end - start);
                    for _ in 0..remaining {
                        ui.monospace("   ");
                    }
                    ui.monospace(format!("  {ascii}"));
                });
            }
        });
    });

    // Navigation controls
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
