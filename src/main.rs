mod carver;
mod disk_info;
mod git_recovery;
mod hex_viewer;
mod imager;
mod partition_scanner;
mod utils;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    DiskInfo,
    Imaging,
    PartitionScan,
    FileCarver,
    HexViewer,
    GitRecovery,
}

struct DiskDoctor {
    tab: Tab,
    disk_info: disk_info::DiskInfoState,
    imaging: imager::ImagingState,
    partition_scan: partition_scanner::PartitionScanState,
    carver: carver::CarverState,
    hex_viewer: hex_viewer::HexViewerState,
    git_recovery: git_recovery::GitRecoveryState,
}

impl Default for DiskDoctor {
    fn default() -> Self {
        Self {
            tab: Tab::DiskInfo,
            disk_info: disk_info::DiskInfoState::default(),
            imaging: imager::ImagingState::default(),
            partition_scan: partition_scanner::PartitionScanState::default(),
            carver: carver::CarverState::new(),
            hex_viewer: hex_viewer::HexViewerState::default(),
            git_recovery: git_recovery::GitRecoveryState::default(),
        }
    }
}

impl eframe::App for DiskDoctor {
    fn ui(&mut self, _: &mut egui::Ui, _: &mut eframe::Frame) {}
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🩺 Disk Doctor");
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::DiskInfo,      "💻 Disk Info");
                ui.selectable_value(&mut self.tab, Tab::Imaging,       "💾 Imaging");
                ui.selectable_value(&mut self.tab, Tab::PartitionScan, "🔍 Partition Scan");
                ui.selectable_value(&mut self.tab, Tab::FileCarver,    "🔧 File Carver");
                ui.selectable_value(&mut self.tab, Tab::HexViewer,     "📝 Hex Viewer");
                ui.selectable_value(&mut self.tab, Tab::GitRecovery,   "🔗 Git Recovery");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::DiskInfo      => disk_info::disk_info_ui(&mut self.disk_info, ui),
                Tab::Imaging       => imager::imaging_ui(&mut self.imaging, ctx, ui),
                Tab::PartitionScan => partition_scanner::partition_scan_ui(&mut self.partition_scan, ctx, ui),
                Tab::FileCarver    => carver::carver_ui(&mut self.carver, ctx, ui),
                Tab::HexViewer     => hex_viewer::hex_viewer_ui(&mut self.hex_viewer, ctx, ui),
                Tab::GitRecovery   => git_recovery::git_recovery_ui(&mut self.git_recovery, ctx, ui),
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    println!("Disk Doctor — Forensic Recovery Toolkit");

    eframe::run_native(
        "Disk Doctor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1000.0, 720.0])
                .with_min_inner_size([700.0, 500.0]),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::<DiskDoctor>::default())),
    )
}
