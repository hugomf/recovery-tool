use std::process::Command;

#[derive(Clone)]
pub struct DiskEntry {
    pub name: String,
    pub identifier: String,
    pub size: String,
    pub internal: bool,
    pub details: Vec<(String, String)>,
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
            let parts: Vec<&str> = line.split_whitespace().collect();
            let name = parts.first().unwrap_or(&"").trim_start_matches("/dev/");
            current = Some(DiskEntry {
                name: name.to_string(),
                identifier: parts.first().unwrap_or(&"").to_string(),
                size: String::new(),
                internal: is_internal,
                details: Vec::new(),
            });
        } else if let Some(ref mut d) = current {
            if line.contains("GUID_partition_scheme") || line.contains("FDisk_partition_scheme") || line.contains("Apple_partition_scheme") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    d.size = parts[parts.len() - 2].to_string();
                }
            }
            if !line.trim().is_empty() {
                d.details.push(("".into(), line.trim().to_string()));
            }
        }
    }
    if let Some(d) = current.take() {
        disks.push(d);
    }

    Ok(disks)
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

    egui::Panel::left("disk_tree")
        .resizable(true)
        .default_width(200.0)
        .show_inside(ui, |ui| {
            ui.label("Devices:");
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, disk) in state.disks.iter().enumerate() {
                    let label = format!(
                        "{} {} {}",
                        if disk.internal { "🖥" } else { "💾" },
                        disk.name,
                        disk.size
                    );
                    if ui.selectable_label(state.selected_index == i, &label).clicked() {
                        state.selected_index = i;
                    }
                }
            });
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(disk) = state.disks.get(state.selected_index) {
            ui.strong(&disk.identifier);
            ui.label(format!("Size: {}", disk.size));
            ui.label(if disk.internal { "Internal" } else { "External" });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (_, line) in &disk.details {
                    ui.monospace(line);
                }
            });

            if ui.button("📋 Copy disk info").clicked() {
                let mut info = format!("Device: {}\nSize: {}\n", disk.identifier, disk.size);
                for (_, line) in &disk.details {
                    info.push_str(line);
                    info.push('\n');
                }
                if let Ok(mut child) = std::process::Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        use std::io::Write;
                        let _ = stdin.write_all(info.as_bytes());
                    }
                }
            }
        }
    });
}
