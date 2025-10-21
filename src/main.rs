use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use dialoguer::{Confirm, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};
use libmtp_rs::device::raw::detect_raw_devices;
use libmtp_rs::device::{MtpDevice, StorageSort};
use libmtp_rs::object::filetypes::Filetype;
use libmtp_rs::storage::Parent;
use libmtp_rs::storage::files::FileMetadata;
use libmtp_rs::util::CallbackReturn;

use std::borrow::Cow;
use std::cmp;
use std::fs::{File, metadata, remove_file};
use std::io::{BufWriter, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn make_progres_bar(size: u64, message: impl Into<Cow<'static, str>>) -> Result<ProgressBar> {
    let bar = ProgressBar::new(size).with_message(message).with_style(
        ProgressStyle::with_template(
            "{msg:60}  [{wide_bar}] {percent}% ({binary_bytes}/{binary_total_bytes})",
        )?
        .progress_chars("## "),
    );
    Ok(bar)
}

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

fn get_free_space(device: &MtpDevice, storage_id: u32) -> Result<u64> {
    let pool = device.storage_pool();
    let storage = pool.by_id(storage_id).context("Could not select storage")?;
    Ok(storage.free_space_in_bytes())
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

fn create_filler_file(device: &MtpDevice, storage_id: u32) -> Result<PathBuf> {
    const BUFFER_SIZE: usize = 1024;
    let free_bytes = get_free_space(device, storage_id)?;
    let input_size = Input::new()
        .with_prompt("How much space should be left on device?")
        .validate_with(|input: &String| -> Result<(), String> {
            let input_size = ByteSize::from_str(&input)?;
            if input_size >= ByteSize::b(free_bytes) {
                Err(
                    "Desired free bytes cannot be larger than current free space on device"
                        .to_string(),
                )
            } else if input_size < ByteSize::b(BUFFER_SIZE.try_into().unwrap()) {
                Err("Desired free bytes must be larger than 1024 bytes (1 KiB)".to_string())
            } else {
                Ok(())
            }
        })
        .default("10MiB".to_string())
        .interact_text()?;
    let input_bytes = ByteSize::from_str(&input_size).map_err(|e| anyhow!(e))?;

    let filler_file_size = free_bytes - input_bytes.as_u64();
    let filler_file_size: usize = filler_file_size.try_into()?;

    let filler_path = PathBuf::from("./zzz_filler.txt");

    let f = File::create(&filler_path)?;
    let mut writer = BufWriter::new(f);
    let bar = make_progres_bar((filler_file_size).try_into()?, "Creating filler file")?;

    let mut buffer = [0; BUFFER_SIZE];
    let mut remaining_size = filler_file_size;

    while remaining_size > 0 {
        let to_write = cmp::min(remaining_size, buffer.len());
        let buffer = &mut buffer[..to_write];
        fastrand::fill(buffer);
        writer.write(buffer).unwrap();

        remaining_size -= to_write;
        bar.inc(1024);
    }
    bar.finish_and_clear();
    Ok(filler_path)
}

fn delete_fillter_file(path: impl AsRef<Path>) -> Result<()> {
    let prompt = format!(
        "Delete the local filler file? ({})",
        path.as_ref().display()
    );
    let input = Confirm::new()
        .with_prompt(prompt)
        .default(true)
        .interact()?;
    if input {
        remove_file(path)?;
    }
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
        file_size: meta.size(),
        file_type: Filetype::Unknown,
        modification_date,
    })
}

fn main() -> Result<()> {
    let mut device = select_device()?;
    let storage_id = select_storage(&device)?;
    let filler_file_path = create_filler_file(&device, storage_id)?;
    let meta = get_metadata(&filler_file_path)?;
    send_file_to_device(&device, storage_id, &filler_file_path, meta)?;
    delete_fillter_file(&filler_file_path)?;
    device.update_storage(StorageSort::NotSorted)?;
    let free_space = get_free_space(&device, storage_id)?;
    println!(
        "Successfully filled mtp storage, remaining free space is: {}",
        ByteSize::b(free_space).display()
    );
    Ok(())
}
