use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};

use anyhow::Result;
use bytesize::ByteSize;
use eframe::egui::{self, ComboBox, ProgressBar};

use crate::backend::{
    FillRequest, ProgressUpdate, StorageInfo, list_devices_safe, list_storages_safe, run_fill,
};
use crate::shared::validate_desired_free_space;

enum WorkerMessage {
    Progress(ProgressUpdate),
    Completed(Result<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FreeSpacePreset {
    MiB20,
    MiB50,
    MiB75,
    MiB100,
    Custom,
}

pub struct MtpFillerApp {
    devices: Vec<String>,
    storages_by_device: Vec<Vec<StorageInfo>>,
    selected_pair: Option<(usize, usize)>,
    desired_free_input: String,
    free_space_preset: FreeSpacePreset,
    delete_local_file: bool,
    status: String,
    error: Option<String>,
    is_busy: bool,
    local_file_progress: (u64, u64),
    transfer_progress: (u64, u64),
    worker_rx: Option<Receiver<WorkerMessage>>,
    success_popup_message: Option<String>,
    confirm_fill_popup_open: bool,
    pending_fill_request: Option<FillRequest>,
}

impl Default for MtpFillerApp {
    fn default() -> Self {
        let mut app = Self {
            devices: Vec::new(),
            storages_by_device: Vec::new(),
            selected_pair: None,
            desired_free_input: "10MiB".to_string(),
            free_space_preset: FreeSpacePreset::Custom,
            delete_local_file: true,
            status: "Ready".to_string(),
            error: None,
            is_busy: false,
            local_file_progress: (0, 0),
            transfer_progress: (0, 0),
            worker_rx: None,
            success_popup_message: None,
            confirm_fill_popup_open: false,
            pending_fill_request: None,
        };

        app.refresh_all(true);
        app
    }
}

impl MtpFillerApp {
    fn refresh_all(&mut self, update_status: bool) {
        let previous_selection = self.selected_pair;
        self.error = None;

        match list_devices_safe() {
            Ok(devices) => {
                self.devices = devices.into_iter().map(|d| d.label).collect();
                self.storages_by_device.clear();

                for (device_index, _) in self.devices.iter().enumerate() {
                    match list_storages_safe(device_index) {
                        Ok(storages) => {
                            self.storages_by_device.push(storages);
                        }
                        Err(err) => {
                            self.storages_by_device.push(Vec::new());
                            self.error = Some(err.to_string());
                        }
                    }
                }

                self.selected_pair = previous_selection.filter(|(device_idx, storage_idx)| {
                    self.storages_by_device
                        .get(*device_idx)
                        .and_then(|storages| storages.get(*storage_idx))
                        .is_some()
                });

                if update_status {
                    self.status = if self.devices.is_empty() {
                        "No devices found".to_string()
                    } else {
                        "Devices and storages refreshed".to_string()
                    };
                }
            }
            Err(err) => {
                self.devices.clear();
                self.storages_by_device.clear();
                self.selected_pair = None;
                self.error = Some(err.to_string());
            }
        }
    }

    fn selected_pair_label(&self) -> String {
        let Some((device_idx, storage_idx)) = self.selected_pair else {
            return "Select device\nSelect storage".to_string();
        };

        let Some(device_label) = self.devices.get(device_idx) else {
            return "Select device\nSelect storage".to_string();
        };

        let Some(storage_label) = self
            .storages_by_device
            .get(device_idx)
            .and_then(|storages| storages.get(storage_idx))
            .map(|storage| storage.label.clone())
        else {
            return "Select device\nSelect storage".to_string();
        };

        format!("{}\n{}", device_label, storage_label)
    }

    fn combo_entries(&self) -> Vec<(usize, usize, String)> {
        let mut entries = Vec::new();
        for (device_idx, device_label) in self.devices.iter().enumerate() {
            if let Some(storages) = self.storages_by_device.get(device_idx) {
                for (storage_idx, storage) in storages.iter().enumerate() {
                    entries.push((
                        device_idx,
                        storage_idx,
                        format!("{}\n{}", device_label, storage.label),
                    ));
                }
            }
        }
        entries
    }

    fn selected_storage_info(&self) -> Option<&StorageInfo> {
        let (device_idx, storage_idx) = self.selected_pair?;
        self.storages_by_device
            .get(device_idx)
            .and_then(|storages| storages.get(storage_idx))
    }

    fn selected_desired_free_bytes(&self) -> std::result::Result<ByteSize, String> {
        match self.free_space_preset {
            FreeSpacePreset::MiB20 => {
                ByteSize::from_str("20MiB").map_err(|e| format!("Invalid preset value: {e}"))
            }
            FreeSpacePreset::MiB50 => {
                ByteSize::from_str("50MiB").map_err(|e| format!("Invalid preset value: {e}"))
            }
            FreeSpacePreset::MiB75 => {
                ByteSize::from_str("75MiB").map_err(|e| format!("Invalid preset value: {e}"))
            }
            FreeSpacePreset::MiB100 => {
                ByteSize::from_str("100MiB").map_err(|e| format!("Invalid preset value: {e}"))
            }
            FreeSpacePreset::Custom => ByteSize::from_str(&self.desired_free_input)
                .map_err(|e| format!("Invalid free space value: {e}")),
        }
    }

    fn prepare_fill_request(&mut self) -> Option<FillRequest> {
        self.error = None;

        let Some((device_index, storage_index)) = self.selected_pair else {
            self.error = Some("Please select a device and storage".to_string());
            return None;
        };

        let desired_free_bytes = match self.selected_desired_free_bytes() {
            Ok(size) => size,
            Err(message) => {
                self.error = Some(message);
                return None;
            }
        };

        let current_free_bytes = match self.selected_storage_info().and_then(|s| s.free_bytes) {
            Some(bytes) => ByteSize::b(bytes),
            None => {
                self.error = Some(
                    "Current free space is unknown. Please refresh devices and storages."
                        .to_string(),
                );
                return None;
            }
        };

        if let Err(err) = validate_desired_free_space(current_free_bytes, desired_free_bytes) {
            self.error = Some(err.to_string());
            return None;
        }

        Some(FillRequest {
            device_index,
            storage_index,
            desired_free_bytes,
            delete_local_file: self.delete_local_file,
        })
    }

    fn start_fill_with_request(&mut self, request: FillRequest) {
        if self.is_busy {
            return;
        }

        self.error = None;

        self.is_busy = true;
        self.status = "Starting transfer".to_string();
        self.local_file_progress = (0, 0);
        self.transfer_progress = (0, 0);

        let (tx, rx) = mpsc::channel::<WorkerMessage>();
        self.worker_rx = Some(rx);

        std::thread::spawn(move || {
            let tx_progress = tx.clone();
            let result = run_fill(request, |update| {
                let _ = tx_progress.send(WorkerMessage::Progress(update));
            })
            .map(|res| {
                format!(
                    "Done. Remaining free space: {}",
                    res.remaining_free_space.display()
                )
            });
            let _ = tx.send(WorkerMessage::Completed(result));
        });
    }

    fn poll_worker(&mut self) {
        let mut completed = false;
        let Some(rx) = self.worker_rx.take() else {
            return;
        };

        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMessage::Progress(update) => match update {
                    ProgressUpdate::Status(status) => {
                        self.status = status.as_str().to_string();
                    }
                    ProgressUpdate::LocalFileProgress { written, total } => {
                        self.local_file_progress = (written, total);
                    }
                    ProgressUpdate::TransferProgress { sent, total } => {
                        self.transfer_progress = (sent, total);
                    }
                },
                WorkerMessage::Completed(result) => {
                    self.is_busy = false;
                    match result {
                        Ok(message) => {
                            self.status = message;
                            self.error = None;
                            self.success_popup_message = Some(self.status.clone());
                            self.refresh_all(false);
                        }
                        Err(err) => {
                            self.error = Some(err.to_string());
                        }
                    }
                    completed = true;
                }
            }
        }

        if !completed {
            self.worker_rx = Some(rx);
        }
    }
}

impl eframe::App for MtpFillerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        let modal_open = self.success_popup_message.is_some() || self.confirm_fill_popup_open;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!modal_open, |ui| {
                ui.heading("MTP Filler");
                ui.label("Fill an MTP storage to leave a specific amount of free space.");
                ui.separator();

                let target_label = ui.label("Target");
                ui.horizontal(|ui| {
                    let target_combo_response = ui
                        .scope(|ui| {
                            let row_height = ui.text_style_height(&egui::TextStyle::Body);
                            ui.spacing_mut().interact_size.y =
                                (row_height * 2.0) + (ui.spacing().button_padding.y * 2.0);
                            ComboBox::from_id_salt("device_storage_select")
                                .selected_text(self.selected_pair_label())
                                .width(500.0)
                                .show_ui(ui, |ui| {
                                    for (device_idx, storage_idx, label) in self.combo_entries() {
                                        ui.selectable_value(
                                            &mut self.selected_pair,
                                            Some((device_idx, storage_idx)),
                                            label,
                                        );
                                    }
                                })
                                .response
                        })
                        .inner;
                    target_combo_response.labelled_by(target_label.id);

                    if ui
                        .add_enabled(!self.is_busy, egui::Button::new("\u{1F504} Refresh"))
                        .clicked()
                    {
                        self.refresh_all(true);
                    }
                });
                ui.add_space(10.0);

                let free_space_label = ui.label("Free space to leave");
                ui.add_enabled_ui(!self.is_busy, |ui| {
                    ui.vertical(|ui| {
                        ui.radio_value(
                            &mut self.free_space_preset,
                            FreeSpacePreset::MiB20,
                            "20MiB",
                        );
                        ui.radio_value(
                            &mut self.free_space_preset,
                            FreeSpacePreset::MiB50,
                            "50MiB",
                        );
                        ui.radio_value(
                            &mut self.free_space_preset,
                            FreeSpacePreset::MiB75,
                            "75MiB",
                        );
                        ui.radio_value(
                            &mut self.free_space_preset,
                            FreeSpacePreset::MiB100,
                            "100MiB",
                        );
                        ui.radio_value(
                            &mut self.free_space_preset,
                            FreeSpacePreset::Custom,
                            "Custom",
                        );

                        if self.free_space_preset == FreeSpacePreset::Custom {
                            let custom_input = ui.add(
                                egui::TextEdit::singleline(&mut self.desired_free_input)
                                    .hint_text("e.g. 10MiB"),
                            );
                            custom_input.labelled_by(free_space_label.id);
                        }
                    });
                });
                ui.add_space(10.0);

                ui.add_enabled_ui(!self.is_busy, |ui| {
                    ui.checkbox(
                        &mut self.delete_local_file,
                        "Delete local filler file after transfer",
                    );
                });
                ui.add_space(10.0);

                if ui
                    .add_enabled(
                        !self.is_busy && self.selected_pair.is_some(),
                        egui::Button::new("Fill storage"),
                    )
                    .clicked()
                {
                    if let Some(request) = self.prepare_fill_request() {
                        self.pending_fill_request = Some(request);
                        self.confirm_fill_popup_open = true;
                    }
                }

                ui.separator();
                ui.label(format!("Status: {}", self.status));

                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
                }

                if self.local_file_progress.1 > 0 {
                    let progress =
                        self.local_file_progress.0 as f32 / self.local_file_progress.1 as f32;
                    ui.label("Creating local filler file");
                    ui.add(ProgressBar::new(progress).show_percentage());
                }

                if self.transfer_progress.1 > 0 {
                    let progress = self.transfer_progress.0 as f32 / self.transfer_progress.1 as f32;
                    ui.label("Sending file to device");
                    ui.add(ProgressBar::new(progress).show_percentage());
                }
            });
        });

        if modal_open {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("modal_backdrop"),
            ));
            painter.rect_filled(
                ctx.content_rect(),
                0.0,
                egui::Color32::from_black_alpha(120),
            );
        }

        if let Some(message) = self.success_popup_message.clone() {
            egui::Window::new("Success")
                .order(egui::Order::Foreground)
                .collapsible(false)
                .movable(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(message);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        self.success_popup_message = None;
                    }
                });
        }

        if self.confirm_fill_popup_open {
            egui::Window::new("Confirm Fill")
                .order(egui::Order::Foreground)
                .collapsible(false)
                .movable(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Start filling the selected storage now?");
                    ui.add_space(8.0);

                    let desired = self.selected_desired_free_bytes().ok();
                    let current_free_space = self
                        .selected_storage_info()
                        .and_then(|s| s.free_bytes)
                        .map(|v| ByteSize::b(v).to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    let remaining_text = desired
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    ui.label(format!("Current free space: {}", current_free_space));
                    ui.label(format!("Remaining after send: {}", remaining_text));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.confirm_fill_popup_open = false;
                        }
                        if ui.button("Start").clicked() {
                            self.confirm_fill_popup_open = false;
                            if let Some(request) = self.pending_fill_request.take() {
                                self.start_fill_with_request(request);
                            }
                        }
                    });
                });
        }

        if self.is_busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
