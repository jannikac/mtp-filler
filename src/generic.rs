use anyhow::{Context, Result};
use bytes::Bytes;
use bytesize::ByteSize;
use dialoguer::Select;
use mtp_rs::{NewObjectInfo, StorageId};
use smol::{fs, stream};
use std::fs::metadata;
use std::ops::ControlFlow;
use std::path::Path;

use crate::shared::{create_filler_file, delete_fillter_file, make_progres_bar};

async fn select_device() -> Result<mtp_rs::MtpDevice> {
    let raw_devices = mtp_rs::MtpDevice::list_devices()?;
    let raw_devices_string = raw_devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            format!(
                "ID {}: {} (VID: {}, PID: {})",
                i,
                dev.display(),
                dev.product_id,
                dev.vendor_id
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
        .serial_number
        .clone()
        .context("Device has no serial number")?;

    let aa = mtp_rs::MtpDevice::open_by_serial(&selected_device).await?;

    Ok(aa)
}

async fn select_storage(device: &mtp_rs::MtpDevice) -> Result<StorageId> {
    let storage_pools = device.storages().await?;
    let storage_pool_vec = storage_pools.iter().collect::<Vec<_>>();
    let storage_pool_strings = storage_pools
        .iter()
        .map(|storage| {
            let info = storage.info();
            format!(
                "ID {}: {} (capacity: {}, free space: {})",
                storage.id().0,
                info.description,
                ByteSize::b(info.max_capacity).display(),
                ByteSize::b(info.free_space_bytes).display()
            )
        })
        .collect::<Vec<_>>();
    let input = Select::new()
        .with_prompt("Select storage to use")
        .default(0)
        .items(storage_pool_strings)
        .interact()?;
    let storage = storage_pool_vec
        .get(input)
        .context("Failed to select storage")?;
    Ok(storage.id())
}

async fn get_free_space(device: &mtp_rs::MtpDevice, storage_id: StorageId) -> Result<ByteSize> {
    let mut storage = device.storage(storage_id).await?;
    // ensure fresh data
    storage.refresh().await?;
    let free_space = ByteSize::b(storage.info().free_space_bytes);
    Ok(free_space)
}

async fn send_file_to_device(
    device: &mtp_rs::MtpDevice,
    storage_id: StorageId,
    filler_file_path: impl AsRef<Path>,
    object_info: NewObjectInfo,
) -> Result<()> {
    let path = filler_file_path.as_ref();
    let bar = make_progres_bar(1, "Sending file to device")?;
    let storage = device.storage(storage_id).await?;

    let content = fs::read(path).await?;

    let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(content))]);

    storage
        .upload_with_progress(None, object_info, stream, |progress| {
            bar.set_length(progress.total_bytes.unwrap());
            bar.set_position(progress.bytes_transferred);
            ControlFlow::Continue(())
        })
        .await?;

    Ok(())
}

fn get_metadata(path: &Path) -> Result<NewObjectInfo> {
    let meta = metadata(path)?;
    // let modification_date: DateTime<Utc> = meta.modified()?.into();
    let file_name = path
        .file_name()
        .context("Path terminates in ..")?
        .to_str()
        .context("File name is not valid unicode")?;
    Ok(NewObjectInfo::file(file_name, meta.len()))
}

pub fn run() -> Result<()> {
    smol::block_on(async {
        let device = select_device().await?;
        let storage_id = select_storage(&device).await?;
        let free_space = get_free_space(&device, storage_id).await?;
        let filler_file_path = create_filler_file(free_space)?;
        let filler_file_path = filler_file_path.canonicalize()?;
        let meta = get_metadata(&filler_file_path)?;
        send_file_to_device(&device, storage_id, &filler_file_path, meta).await?;
        delete_fillter_file(&filler_file_path)?;

        let remaining_free_space = get_free_space(&device, storage_id).await?;
        println!(
            "Successfully filled MTP storage, remaining free space is: {}",
            remaining_free_space.display()
        );

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
