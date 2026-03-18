use std::str::FromStr;

use anyhow::{Result, anyhow};
use bytesize::ByteSize;
use dialoguer::{Confirm, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};

use crate::backend::{FillRequest, FillStatus, ProgressUpdate, list_devices_safe, list_storages_safe, run_fill};
use crate::shared::validate_desired_free_space;

fn make_progress_bar(total: u64, message: &str) -> Result<ProgressBar> {
    let bar = ProgressBar::new(total).with_message(message.to_string()).with_style(
        ProgressStyle::with_template(
            "{msg:30} [{wide_bar}] {percent}% ({binary_bytes}/{binary_total_bytes})",
        )?
        .progress_chars("## "),
    );
    Ok(bar)
}

pub fn run_cli() -> Result<()> {
    let devices = list_devices_safe()?;
    if devices.is_empty() {
        return Err(anyhow!("No attached MTP devices detected"));
    }

    let device_idx = Select::new()
        .with_prompt("Select the device to use")
        .items(devices.iter().map(|d| d.label.as_str()).collect::<Vec<_>>())
        .default(0)
        .interact()?;

    let storages = list_storages_safe(device_idx)?;
    if storages.is_empty() {
        return Err(anyhow!("No storage detected on selected device"));
    }

    let storage_idx = Select::new()
        .with_prompt("Select storage to use")
        .items(storages.iter().map(|s| s.label.as_str()).collect::<Vec<_>>())
        .default(0)
        .interact()?;

    let selected_storage = &storages[storage_idx];
    let input_size = Input::new()
        .with_prompt("How much space should be left on device?")
        .default("10MiB".to_string())
        .validate_with(|input: &String| -> Result<(), String> {
            let desired = ByteSize::from_str(input).map_err(|e| e.to_string())?;

            if let Some(free) = selected_storage.free_bytes {
                validate_desired_free_space(ByteSize::b(free), desired)
                    .map_err(|e| e.to_string())?;
            }

            Ok(())
        })
        .interact_text()?;

    let desired_free_bytes = ByteSize::from_str(&input_size).map_err(|e| anyhow!(e.to_string()))?;
    let delete_local_file = Confirm::new()
        .with_prompt("Delete local filler file after transfer?")
        .default(true)
        .interact()?;

    let request = FillRequest {
        device_index: device_idx,
        storage_index: storage_idx,
        desired_free_bytes,
        delete_local_file,
    };

    let mut local_bar: Option<ProgressBar> = None;
    let mut transfer_bar: Option<ProgressBar> = None;

    let result = run_fill(request, |update| match update {
        ProgressUpdate::Status(status) => match status {
            FillStatus::CreatingLocalFillerFile
            | FillStatus::SendingFileToDevice
            | FillStatus::FileWrittenToDevice
            | FillStatus::FinalizingTransfer => {}
        },
        ProgressUpdate::LocalFileProgress { written, total } => {
            if local_bar.is_none() {
                local_bar = make_progress_bar(total, "Creating filler file").ok();
            }
            if let Some(bar) = &local_bar {
                bar.set_length(total);
                bar.set_position(written);
            }
        }
        ProgressUpdate::TransferProgress { sent, total } => {
            if transfer_bar.is_none() {
                transfer_bar = make_progress_bar(total, "Sending to device").ok();
            }
            if let Some(bar) = &transfer_bar {
                bar.set_length(total);
                bar.set_position(sent);
            }
        }
    })?;

    if let Some(bar) = &local_bar {
        bar.finish_and_clear();
    }
    if let Some(bar) = &transfer_bar {
        bar.finish_and_clear();
    }

    println!(
        "Successfully filled MTP storage, remaining free space is: {}",
        result.remaining_free_space.display()
    );

    Ok(())
}
