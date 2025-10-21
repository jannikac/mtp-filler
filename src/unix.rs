use anyhow::{Context, Result};
use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use dialoguer::Select;
use libmtp_rs::device::raw::detect_raw_devices;
use libmtp_rs::device::{MtpDevice, StorageSort};
use libmtp_rs::object::filetypes::Filetype;
use libmtp_rs::storage::Parent;
use libmtp_rs::storage::files::FileMetadata;
use libmtp_rs::util::CallbackReturn;
use std::fs::metadata;
use std::io::Write;
use std::path::Path;

use crate::shared::{create_filler_file, delete_fillter_file, make_progres_bar};

fn select_device() -> Result<MtpDevice> {
    let raw_devices = detect_raw_devices()?;
    let raw_devices_string = raw_devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let entry = dev.device_entry();
            format!(
                "ID {}: {} {} (VID: {}, PID: {})",
                i, entry.vendor, entry.product, entry.product_id, entry.vendor_id
            )
        })
        .collect::<Vec<_>>();

    let input = Select::new()
        .with_prompt("Select the device to use")
        .default(0)
        .items(raw_devices_string)
        .interact()?;

    let selected_device = raw_devices
        .get(input)
        .context("Failed to select device")?
        .open_uncached()
        .context("Failed to open device")?;

    Ok(selected_device)
}

fn select_storage(device: &MtpDevice) -> Result<u32> {
    let storage_pools = device.storage_pool();
    let storage_pool_vec = storage_pools.iter().collect::<Vec<_>>();
    let storage_pool_strings = storage_pools
        .iter()
        .map(|(_, storage)| {
            format!(
                "ID {}: {} (capacity: {}, free space: {})",
                storage.id(),
                storage.description().unwrap_or("-"),
                ByteSize::b(storage.maximum_capacity()).display(),
                ByteSize::b(storage.free_space_in_bytes()).display()
            )
        })
        .collect::<Vec<_>>();
    let input = Select::new()
        .with_prompt("Select storage to use")
        .default(0)
        .items(storage_pool_strings)
        .interact()?;
    let (_, storage) = storage_pool_vec[input];
    Ok(storage.id())
}

fn get_free_space(device: &MtpDevice, storage_id: u32) -> Result<ByteSize> {
    let pool = device.storage_pool();
    let storage = pool.by_id(storage_id).context("Could not select storage")?;
    let free_space = ByteSize::b(storage.free_space_in_bytes());
    Ok(free_space)
}

fn send_file_to_device(
    device: &MtpDevice,
    storage_id: u32,
    filler_file_path: impl AsRef<Path>,
    metadata: FileMetadata,
) -> Result<()> {
    let bar = make_progres_bar(1, "Sending file to device")?;
    let pool = device.storage_pool();
    let storage = pool.by_id(storage_id).context("Could not select storage")?;

    storage.send_file_from_path_with_callback(
        filler_file_path,
        Parent::Root,
        metadata,
        |sent, total| {
            bar.set_length(total);
            bar.set_position(sent);
            std::io::stdout().lock().flush().expect("Failed to flush");
            CallbackReturn::Continue
        },
    )?;
    Ok(())
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

pub fn run() -> Result<()> {
    let mut device = select_device()?;
    let storage_id = select_storage(&device)?;
    let free_space = get_free_space(&device, storage_id)?;
    let filler_file_path = create_filler_file(free_space)?;
    let filler_file_path = filler_file_path.canonicalize()?;
    let meta = get_metadata(&filler_file_path)?;
    send_file_to_device(&device, storage_id, &filler_file_path, meta)?;
    delete_fillter_file(&filler_file_path)?;
    device.update_storage(StorageSort::NotSorted)?;
    let remaining_free_space = get_free_space(&device, storage_id)?;
    println!(
        "Successfully filled MTP storage, remaining free space is: {}",
        remaining_free_space.display()
    );
    Ok(())
}
