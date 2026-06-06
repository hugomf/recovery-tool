use std::process::Command;

#[derive(Clone)]
pub struct DiskEntry {
    pub identifier: String,
    pub size: String,
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

    let mut disks = Vec::new();
    let mut current: Option<DiskEntry> = None;

    for line in text.lines() {
        if line.starts_with("/dev/disk") {
            if let Some(d) = current.take() {
                disks.push(d);
            }
            let is_internal = line.contains("internal");
            let identifier = line.split_whitespace().next().unwrap_or("").to_string();
            current = Some(DiskEntry {
                identifier,
                size: String::new(),
                internal: is_internal,
                details: vec![line.to_string()],
            });
        } else if let Some(ref mut d) = current {
            d.details.push(line.to_string());
            if !line.contains("Scheme") && !line.contains("Container") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[parts.len() - 1].starts_with("disk") {
                    // This is a partition line: extract size if present
                    let end = parts.len() - 1;
                    for i in (1..end).rev() {
                        if parts[i].contains('.') && (parts[i].ends_with('B') || parts[i].ends_with("KB") || parts[i].ends_with("MB") || parts[i].ends_with("GB") || parts[i].ends_with("TB")) {
                            d.size = parts[i].to_string();
                            break;
                        }
                    }
                }
            }
        }
    }
    if let Some(d) = current.take() {
        disks.push(d);
    }

    // Fetch sizes via diskutil info for each disk
    for disk in &mut disks {
        if let Ok(out) = Command::new("diskutil")
            .args(["info", "-plist", &disk.identifier])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(pos) = text.find("<key>TotalSize</key>") {
                let rest = &text[pos..];
                if let Some(val_start) = rest.find("<integer>") {
                    let val_start = val_start + "<integer>".len();
                    if let Some(val_end) = rest[val_start..].find("</integer>") {
                        let size_str = &rest[val_start..val_start + val_end];
                        if let Ok(bytes) = size_str.parse::<u64>() {
                            disk.size = format_size(bytes);
                        }
                    }
                }
            }
            if let Some(pos) = text.find("<key>SMARTStatus</key>") {
                let rest = &text[pos..];
                if let Some(val_start) = rest.find("<string>") {
                    let val_start = val_start + "<string>".len();
                    if let Some(val_end) = rest[val_start..].find("</string>") {
                        let smart = &rest[val_start..val_start + val_end];
                        disk.details.push(format!("SMART: {smart}"));
                    }
                }
            }
        }
    }

    Ok(disks)
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        format!("{:.1} TB", bytes as f64 / 1_000_000_000_000.0)
    } else if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{bytes} B")
    }
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
        ui.spinner();
        ui.label("Loading disk info...");
        return;
    }

    if let Some(ref err) = state.error {
        ui.colored_label(egui::Color32::RED, err);
        return;
    }

    if state.disks.is_empty() {
        ui.label("No disks found. Click Refresh.");
        return;
    }

    ui.horizontal(|ui| {
        ui.scope(|ui| {
            ui.set_min_width(200.0);
            ui.label("Devices:");
            ui.separator();

            egui::ScrollArea::vertical().max_height(ui.available_height()).show(ui, |ui| {
                for (i, disk) in state.disks.iter().enumerate() {
                    let label = format!(
                        "{} {} {}",
                        if disk.internal { "🖥" } else { "💾" },
                        disk.identifier,
                        disk.size
                    );
                    if ui.selectable_label(state.selected_index == i, &label).clicked() {
                        state.selected_index = i;
                    }
                }
            });
        });

        ui.separator();

        if let Some(disk) = state.disks.get(state.selected_index) {
            ui.vertical(|ui| {
                ui.strong(&disk.identifier);
                ui.label(format!("Size: {}", disk.size));
                ui.label(if disk.internal { "Internal" } else { "External" });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for line in &disk.details {
                        ui.monospace(line);
                    }
                });
            });
        }
    });
}
