use std::fs::metadata;
use std::path::Path;

use anyhow::{Context, Result};
use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use libmtp_rs::device::raw::detect_raw_devices;
use libmtp_rs::device::{MtpDevice, StorageSort};
use libmtp_rs::object::filetypes::Filetype;
use libmtp_rs::storage::Parent;
use libmtp_rs::storage::files::FileMetadata;
use libmtp_rs::util::CallbackReturn;

use crate::backend::{DeviceInfo, FillRequest, FillResult, FillStatus, ProgressUpdate, StorageInfo};
use crate::shared::{create_filler_file, maybe_delete_filler_file, validate_desired_free_space};

fn open_device_by_index(index: usize) -> Result<MtpDevice> {
    let raw_devices = detect_raw_devices()?;
    raw_devices
        .get(index)
        .context("Failed to select device")?
        .open_uncached()
        .context("Failed to open device")
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let raw_devices = detect_raw_devices()?;

    Ok(raw_devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let entry = dev.device_entry();
            DeviceInfo {
                label: format!(
                    "ID {}: {} {} (VID: {}, PID: {})",
                    i, entry.vendor, entry.product, entry.product_id, entry.vendor_id
                ),
            }
        })
        .collect())
}

pub fn list_storages(device_index: usize) -> Result<Vec<StorageInfo>> {
    let device = open_device_by_index(device_index)?;
    let storage_pools = device.storage_pool();

    Ok(storage_pools
        .iter()
        .map(|(_, storage)| StorageInfo {
            label: format!(
                "ID {}: {} (capacity: {}, free space: {})",
                storage.id(),
                storage.description().unwrap_or("-"),
                ByteSize::b(storage.maximum_capacity()).display(),
                ByteSize::b(storage.free_space_in_bytes()).display()
            ),
            free_bytes: Some(storage.free_space_in_bytes()),
        })
        .collect())
}

fn get_free_space(device: &MtpDevice, storage_id: u32) -> Result<ByteSize> {
    let pool = device.storage_pool();
    let storage = pool.by_id(storage_id).context("Could not select storage")?;
    Ok(ByteSize::b(storage.free_space_in_bytes()))
}

fn get_metadata(path: &Path) -> Result<FileMetadata> {
    let meta = metadata(path)?;
    let modification_date: DateTime<Utc> = meta.modified()?.into();
    let file_name = path
        .file_name()
        .context("Path terminates in ..")?
        .to_str()
        .context("File name is not valid unicode")?;

    Ok(FileMetadata {
        file_name,
        file_size: meta.len(),
        file_type: Filetype::Unknown,
        modification_date,
    })
}

fn send_file_to_device(
    device: &MtpDevice,
    storage_id: u32,
    filler_file_path: impl AsRef<Path>,
    metadata: FileMetadata,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let pool = device.storage_pool();
    let storage = pool.by_id(storage_id).context("Could not select storage")?;

    storage.send_file_from_path_with_callback(
        filler_file_path,
        Parent::Root,
        metadata,
        |sent, total| {
            on_progress(sent, total);
            CallbackReturn::Continue
        },
    )?;

    Ok(())
}

pub fn run_fill(
    request: FillRequest,
    mut on_progress: impl FnMut(ProgressUpdate),
) -> Result<FillResult> {
    let mut device = open_device_by_index(request.device_index)?;

    let storage_ids = device
        .storage_pool()
        .iter()
        .map(|(_, storage)| storage.id())
        .collect::<Vec<_>>();
    let storage_id = *storage_ids
        .get(request.storage_index)
        .context("Failed to select storage")?;

    let free_space = get_free_space(&device, storage_id)?;
    validate_desired_free_space(free_space, request.desired_free_bytes)?;

    on_progress(ProgressUpdate::Status(FillStatus::CreatingLocalFillerFile));
    let filler_file_path =
        create_filler_file(free_space, request.desired_free_bytes, |written, total| {
            on_progress(ProgressUpdate::LocalFileProgress { written, total })
        })?;

    let filler_file_path = filler_file_path.canonicalize()?;
    let meta = get_metadata(&filler_file_path)?;

    on_progress(ProgressUpdate::Status(FillStatus::SendingFileToDevice));
    send_file_to_device(
        &device,
        storage_id,
        &filler_file_path,
        meta,
        |sent, total| on_progress(ProgressUpdate::TransferProgress { sent, total }),
    )?;
    on_progress(ProgressUpdate::Status(FillStatus::FileWrittenToDevice));

    on_progress(ProgressUpdate::Status(FillStatus::FinalizingTransfer));
    maybe_delete_filler_file(&filler_file_path, request.delete_local_file)?;

    device.update_storage(StorageSort::NotSorted)?;
    let remaining_free_space = get_free_space(&device, storage_id)?;

    Ok(FillResult {
        remaining_free_space,
    })
}
