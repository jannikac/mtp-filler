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

use crate::shared::{create_filler_file, delete_fillter_file};

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
            format!(
                "ID {}: {} (capacity: {} free space: {})",
                v.id().to_string_lossy(),
                v.name().to_string_lossy(),
                get_capacity(device, v.id().into()).unwrap(),
                get_free_space(device, v.id().into()).unwrap()
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
    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;

    println!("\nSending file to device, this may take a while because MTP is slow,");
    println!("for example, 1GB may take up to 2 minutes");
    println!("There will be no progress indicator, please be patient...");
    storage.push_file(file_path.as_ref(), false)?;
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
