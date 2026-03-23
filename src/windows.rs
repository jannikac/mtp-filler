use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use dialoguer::Select;
use dunce::canonicalize;
use widestring::U16CString;
use winmtp::{
    PROPERTYKEY,
    PortableDevices::{WPD_STORAGE_CAPACITY, WPD_STORAGE_FREE_SPACE_IN_BYTES},
    Provider,
    device::Device,
};

use crate::shared::{create_filler_file, delete_fillter_file, make_progres_bar};

fn get_bytes_from_property(
    device: &Device,
    storage_id: U16CString,
    property_key: PROPERTYKEY,
) -> Result<ByteSize> {
    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;
    let properties = storage.properties(&[property_key])?;
    // honestly dont know why bytes is a float or string but not a u32 or similar..
    // we get the string since it doesnt have rounding errors and parse it to an u64
    let bytes = properties.get_string(&property_key)?;
    let bytes_u64 = bytes.to_string()?.parse::<u64>()?;
    let bytes = ByteSize::b(bytes_u64);
    Ok(bytes)
}

fn select_device() -> Result<Device> {
    let app_ident = winmtp::make_current_app_identifiers!();
    let provider = Provider::new()?;
    let raw_devices = provider.enumerate_devices()?;
    if raw_devices.len() < 1 {
        return Err(anyhow!("No attached MTP devices detected"));
    }
    let raw_devices_string = raw_devices
        .iter()
        .enumerate()
        .map(|(i, dev)| format!("ID {}: {}", i, dev.friendly_name()))
        .collect::<Vec<_>>();
    let input = Select::new()
        .with_prompt("Select the device to use")
        .default(0)
        .items(raw_devices_string)
        .interact()?;
    let selected_device = raw_devices
        .get(input)
        .context("Failed to select device")?
        .open(&app_ident, true)?;
    Ok(selected_device)
}

fn select_storage(device: &Device) -> Result<U16CString> {
    let content = device.content()?;
    let root = content.root()?;
    let children = root.children()?.into_iter().collect::<Vec<_>>();
    let children_string = children
        .iter()
        .map(|v| {
            let capacity = get_capacity(device, v.id().into()).ok();
            let free_space = get_free_space(device, v.id().into()).ok();

            format!(
                "ID {}: {} (capacity: {} free space: {})",
                v.id().to_string_lossy(),
                v.name().to_string_lossy(),
                capacity
                    .map(|v| v.to_string())
                    .unwrap_or("unknown".to_string()),
                free_space
                    .map(|v| v.to_string())
                    .unwrap_or("unknown".to_string())
            )
        })
        .collect::<Vec<_>>();
    let input = Select::new()
        .with_prompt("Select storage to use")
        .default(0)
        .items(children_string)
        .interact()?;
    let child = &children[input];
    Ok(child.id().into())
}

fn get_capacity(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(&device, storage_id, WPD_STORAGE_CAPACITY)
}

fn get_free_space(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(&device, storage_id, WPD_STORAGE_FREE_SPACE_IN_BYTES)
}

fn send_file_to_device(
    device: &Device,
    storage_id: U16CString,
    file_path: impl AsRef<Path>,
) -> Result<()> {
    let file_path = file_path.as_ref();
    let file_name = file_path
        .file_name()
        .context("Could not determine source file name for transfer")?;
    let file_size = file_path.metadata()?.len();
    let bar = make_progres_bar(file_size, "Sending file to device")?;

    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;
    let mut dest_writer = storage.create_write_stream(file_name, file_size)?;
    let mut source_reader = File::open(file_path)?;
    let mut bytes_sent = 0_u64;
    let buffer_size = dest_writer.capacity().max(64 * 1024);
    let mut buffer = vec![0_u8; buffer_size];

    loop {
        let read_bytes = source_reader.read(&mut buffer)?;
        if read_bytes == 0 {
            break;
        }

        dest_writer.write_all(&buffer[..read_bytes])?;
        bytes_sent += read_bytes as u64;
        bar.set_position(bytes_sent.min(file_size));
        std::io::stdout().lock().flush()?;
    }

    dest_writer.flush()?;
    bar.finish_and_clear();
    Ok(())
}

pub fn run() -> Result<()> {
    let device = select_device()?;
    let storage_id = select_storage(&device)?;
    let free_space = get_free_space(&device, storage_id.clone())?;
    let filler_file_path = create_filler_file(free_space)?;
    let filler_file_path = canonicalize(filler_file_path)?;
    send_file_to_device(&device, storage_id.clone(), &filler_file_path)?;
    delete_fillter_file(&filler_file_path)?;
    let remaining_free_space = get_free_space(&device, storage_id.clone())?;
    println!(
        "Successfully filled mtp storage, remaining free space is: {}",
        remaining_free_space.display()
    );
    Ok(())
}
