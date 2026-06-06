use std::process::Command;
use std::sync::mpsc;
use std::thread;

#[derive(Clone, Debug)]
pub struct RecoveredRepo {
    pub name: String,
    pub remote: String,
    pub source: String,
    pub local_path: Option<String>,
}

#[derive(Clone)]
pub enum ScanEvent {
    Progress(String),
    FoundRepo(RecoveredRepo),
    Done,
}

fn run_scan(tx: mpsc::Sender<ScanEvent>) {
    let home = std::env::var("HOME").unwrap_or_default();
    let projects = format!("{home}/Projects");
    let mut seen_remotes: Vec<String> = vec![];

    tx.send(ScanEvent::Progress("Scanning for surviving .git dirs...".into())).ok();
    if let Some(out) = Command::new("find")
        .args([&projects, "-maxdepth", "4", "-name", "config", "-path", "*.git/config", "-type", "f"])
        .output().ok()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Some(cfg) = std::fs::read_to_string(line).ok() {
                for line2 in cfg.lines() {
                    let line2 = line2.trim();
                    if line2.starts_with("url = ") {
                        let url = line2.trim_start_matches("url = ");
                        let name = url.rsplit('/').next().unwrap_or(url)
                            .trim_end_matches(".git");
                        if !seen_remotes.contains(&url.to_string()) {
                            seen_remotes.push(url.to_string());
                            let path = line.trim_end_matches("/.git/config")
                                .trim_end_matches("/config")
                                .to_string();
                            tx.send(ScanEvent::FoundRepo(RecoveredRepo {
                                name: name.to_string(),
                                remote: url.to_string(),
                                source: "surviving .git".into(),
                                local_path: Some(path),
                            })).ok();
                        }
                    }
                }
            }
        }
    }

    tx.send(ScanEvent::Progress("Scanning shell history...".into())).ok();
    for hist_file in &[
        format!("{home}/.zhistory"),
        format!("{home}/.bash_history"),
        format!("{home}/.zsh_history"),
    ] {
        if let Ok(content) = std::fs::read_to_string(hist_file) {
            for raw_line in content.lines() {
                // Strip zsh extended history prefix: ": 1700000000:0;command"
                let line = if raw_line.starts_with(": ") {
                    raw_line.find(";").map_or(raw_line, |semi| &raw_line[semi + 1..])
                } else {
                    raw_line
                };

                if let Some(url_start) = line.find("git clone ") {
                    let rest = &line[url_start + 10..];
                    let url = rest.split_whitespace()
                        .skip_while(|t| t.starts_with('-'))
                        .next()
                        .unwrap_or("")
                        .trim_matches('\'')
                        .trim_matches('"')
                        .to_string();
                    if !url.is_empty() && !seen_remotes.contains(&url) {
                        seen_remotes.push(url.clone());
                        let name = url.rsplit('/').next().unwrap_or(&url)
                            .trim_end_matches(".git")
                            .to_string();
                        tx.send(ScanEvent::FoundRepo(RecoveredRepo {
                            name,
                            remote: url,
                            source: "shell history".into(),
                            local_path: None,
                        })).ok();
                    }
                }
            }
        }
    }

    tx.send(ScanEvent::Progress("Checking gh CLI...".into())).ok();
    if let Ok(out) = Command::new("gh").args(["repo", "list", "--limit", "100"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(full_name) = parts.first() {
                    let name = full_name.split('/').last().unwrap_or(full_name);
                    let url = format!("https://github.com/{full_name}.git");
                    if !seen_remotes.contains(&url) {
                        seen_remotes.push(url.clone());
                        tx.send(ScanEvent::FoundRepo(RecoveredRepo {
                            name: name.to_string(),
                            remote: url,
                            source: "gh CLI".into(),
                            local_path: None,
                        })).ok();
                    }
                }
            }
        }
    }

    tx.send(ScanEvent::Done).ok();
}

#[derive(Default)]
pub struct GitRecoveryState {
    pub repos: Vec<RecoveredRepo>,
    pub scanning: bool,
    pub scan_progress: String,
    pub done: bool,
    pub rx: Option<mpsc::Receiver<ScanEvent>>,
    pub script: Option<String>,
    pub show_script: bool,
}

impl GitRecoveryState {
    pub fn start_scan(&mut self) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || run_scan(tx));
        self.rx = Some(rx);
        self.scanning = true;
        self.done = false;
        self.repos.clear();
        self.scan_progress = "Starting scan...".into();
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ScanEvent::Progress(msg) => self.scan_progress = msg,
                    ScanEvent::FoundRepo(r) => self.repos.push(r),
                    ScanEvent::Done => {
                        self.scanning = false;
                        self.done = true;
                        self.scan_progress = format!("Found {} repos", self.repos.len());
                    }
                }
                ctx.request_repaint();
            }
        }
    }
}

const PHOTOREC_INSTRUCTIONS: &str = r#"=== Deep File Recovery with PhotoRec ===

PhotoRec recovers files by scanning raw disk blocks for file signatures.
It does NOT preserve filenames or folder structure.

STEPS:
1. Install testdisk:
   brew install testdisk

2. Boot from a separate drive or USB (recovery on same disk reduces odds).

3. Run photorec as root:
   sudo photorec

4. Select the disk (usually "APPLE SSD" or similar)
5. Select partition type (EFI GPT or Intel)
6. Select the APFS partition
7. Choose "Other" for filesystem type
8. Select [File Opt] → check source code types: .rs, .ts, .js, .py, .java, .kt, .swift, .go, .c, .h, .cpp, .toml, .json, .yaml, .md, .dart, .gradle, .xml, .properties
9. Choose destination (an EXTERNAL drive — NOT the same disk)
10. Start recovery

After recovery, you'll get thousands of files named like f1234567.rs.
Use `file` command or grep through content to identify what you need.
"#;

pub fn git_recovery_ui(state: &mut GitRecoveryState, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading("🔗 Git Remote Recovery");
    ui.separator();

    ui.horizontal(|ui| {
        if !state.scanning {
            if ui.button("🔍 Scan for recoverable repos").clicked() {
                state.start_scan();
            }
        } else {
            ui.label("⏳");
            ui.label(&state.scan_progress);
            ui.spinner();
        }
    });

    state.update(ctx);

    if state.scanning && state.repos.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("Scanning for recoverable projects...");
            ui.spinner();
            ui.label(&state.scan_progress);
        });
        return;
    }

    if state.done && state.repos.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("No recoverable repos found via git/gh.");
            ui.label("Try photorec for deep file recovery (see bottom of this tab).");
        });
        if ui.button("📖 Show photorec instructions").clicked() {
            state.script = Some(PHOTOREC_INSTRUCTIONS.to_string());
            state.show_script = true;
        }
        return;
    }

    if !state.repos.is_empty() {
        let survivors = state.repos.iter().filter(|r| r.local_path.is_some()).count();
        let from_history = state.repos.iter().filter(|r| r.source == "shell history").count();
        let from_gh = state.repos.iter().filter(|r| r.source == "gh CLI").count();

        ui.horizontal(|ui| {
            ui.label(format!("📁 with local data: {survivors}"));
            ui.label(format!("📜 from shell history: {from_history}"));
            ui.label(format!("🐙 from GitHub: {from_gh}"));
        });
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.label("Recoverable repositories:");
            ui.separator();
            for repo in &state.repos {
                ui.horizontal(|ui| {
                    if repo.local_path.is_some() {
                        ui.label("✅");
                    } else {
                        ui.label("🔶");
                    }
                    ui.strong(&repo.name);
                    ui.label(format!("({})", repo.source));
                    if let Some(ref path) = repo.local_path {
                        ui.label(format!("→ {path}"));
                    }
                });
                ui.monospace(format!("   {}", repo.remote));
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(!state.repos.is_empty(), egui::Button::new("📋 Generate Re-clone Script")).clicked() {
                let mut s = String::from("#!/bin/zsh\n# Recovery script — run to re-clone projects\n# Generated by Disk Doctor\n\n");
                for repo in &state.repos {
                    s.push_str(&format!("# {} ({})\n", repo.remote, repo.source));
                    if repo.local_path.is_some() {
                        s.push_str(&format!("# Already at: {}\n", repo.local_path.as_ref().unwrap()));
                    } else {
                        s.push_str(&format!("git clone {} {}\n", repo.remote, repo.name));
                    }
                    s.push('\n');
                }
                state.script = Some(s);
                state.show_script = true;
            }
            if ui.button("📖 Photorec guide").clicked() {
                state.script = Some(PHOTOREC_INSTRUCTIONS.to_string());
                state.show_script = true;
            }
        });
    }

    if state.show_script {
        let title = if state.script.as_ref().map_or(false, |s| s.contains("photorec")) {
            "📖 Photorec Recovery Guide"
        } else {
            "📋 Re-clone Script"
        };
        let content = state.script.clone().unwrap_or_default();
        let n_lines = content.lines().count() as f32;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_size([700.0, (n_lines * 16.0 + 80.0).min(500.0)])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(&content);
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        state.show_script = false;
                        state.script = None;
                    }
                    if !content.contains("photorec") {
                        if ui.button("💾 Save to ~/recovery.sh").clicked() {
                            let path = format!("{}/recovery.sh", std::env::var("HOME").unwrap_or_default());
                            let _ = std::fs::write(&path, &content);
                            let _ = Command::new("chmod").args(["+x", &path]).output();
                        }
                    }
                });
            });
    }
}
