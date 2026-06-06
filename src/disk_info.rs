use std::process::Command;
use crate::utils::format_size;

#[derive(Clone)]
pub struct DiskEntry {
    pub path: String,
    pub short_name: String,
    pub size_bytes: u64,
    pub internal: bool,
    pub details: Vec<String>,
}

#[derive(Default)]
pub struct DiskInfoState {
    pub disks: Vec<DiskEntry>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected_index: usize,
}

fn parse_diskutil_list() -> Result<Vec<DiskEntry>, String> {
    let out = Command::new("diskutil").arg("list").output()
        .map_err(|e| format!("diskutil failed: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);

    let mut disks: Vec<DiskEntry> = Vec::new();
    let mut current: Option<DiskEntry> = None;

    for line in text.lines() {
        if line.starts_with("/dev/disk") {
            if let Some(d) = current.take() {
                disks.push(d);
            }
            let is_internal = line.contains("internal");
            let path = line.split_whitespace().next().unwrap_or("").to_string();
            let short = path.trim_start_matches("/dev/").to_string();
            current = Some(DiskEntry {
                path,
                short_name: short,
                size_bytes: 0,
                internal: is_internal,
                details: Vec::new(),
            });
        } else if let Some(ref mut d) = current {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                d.details.push(trimmed.to_string());
            }
        }
    }
    if let Some(d) = current.take() {
        disks.push(d);
    }

    for disk in &mut disks {
        if let Some(bytes) = fetch_disk_size(&disk.path) {
            disk.size_bytes = bytes;
        }
    }

    Ok(disks)
}

fn fetch_disk_size(path: &str) -> Option<u64> {
    let out = Command::new("diskutil").args(["info", path]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.trim_start().starts_with("Disk Size:") {
            if let Some(start) = line.find('(') {
                if let Some(end) = line[start..].find(" Bytes") {
                    let num_str = line[start + 1..start + end].replace(',', "");
                    return num_str.trim().parse::<u64>().ok();
                }
            }
        }
    }
    None
}

pub fn fetch_disk_info(state: &mut DiskInfoState) {
    state.loading = true;
    state.error = None;

    match parse_diskutil_list() {
        Ok(disks) => {
            state.disks = disks;
            state.loading = false;
        }
        Err(e) => {
            state.error = Some(e);
            state.loading = false;
        }
    }
}

pub fn disk_info_ui(state: &mut DiskInfoState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("💻 Disk Info");
        if ui.button("⟳ Refresh").clicked() {
            fetch_disk_info(state);
        }
    });
    ui.separator();

    if state.loading {
        ui.horizontal(|ui| { ui.spinner(); ui.label("Loading disk info..."); });
        return;
    }

    if let Some(ref err) = state.error {
        ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
        return;
    }

    if state.disks.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label("No disks found. Click ⟳ Refresh to scan.");
        });
        return;
    }

    egui::SidePanel::left("disk_tree")
        .resizable(true)
        .min_width(160.0)
        .default_width(200.0)
        .show_inside(ui, |ui| {
            ui.label("Devices:");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, disk) in state.disks.iter().enumerate() {
                    let size_label = if disk.size_bytes > 0 {
                        format_size(disk.size_bytes)
                    } else {
                        "?".to_string()
                    };
                    let label = format!("{} {} {}",
                        if disk.internal { "🖥" } else { "💾" },
                        disk.short_name, size_label,
                    );
                    if ui.selectable_label(state.selected_index == i, &label).clicked() {
                        state.selected_index = i;
                    }
                }
            });
        });

    if let Some(disk) = state.disks.get(state.selected_index) {
        ui.vertical(|ui| {
            ui.strong(&disk.path);
            if disk.size_bytes > 0 {
                ui.label(format!("Size: {}", format_size(disk.size_bytes)));
            }
            ui.label(if disk.internal { "Internal" } else { "External" });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for line in &disk.details {
                    ui.monospace(line);
                }
            });

            ui.separator();
            if ui.button("📋 Copy disk info").clicked() {
                let mut info = format!("Device: {}\nSize: {}\n", disk.path, format_size(disk.size_bytes));
                for line in &disk.details {
                    info.push_str(line);
                    info.push('\n');
                }
                copy_to_clipboard(&info);
            }
        });
    }
}

fn copy_to_clipboard(text: &str) {
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
    }
}
