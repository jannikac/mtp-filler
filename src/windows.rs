use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use dialoguer::Select;
use dunce::canonicalize;
use widestring::U16CString;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{GUID, PCWSTR, PWSTR};
use winmtp::{
    PROPERTYKEY,
    PortableDevices::{IPortableDeviceContent, IPortableDeviceValues, PortableDeviceValues},
    PortableDevices::{
        WPD_OBJECT_ORIGINAL_FILE_NAME, WPD_OBJECT_PARENT_ID, WPD_OBJECT_SIZE, WPD_STORAGE_CAPACITY,
        WPD_STORAGE_FREE_SPACE_IN_BYTES,
    },
    Provider,
    device::Device,
    io::WriteStream,
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
    let file_path = file_path.as_ref();
    let file_name = file_path
        .file_name()
        .context("Could not determine source file name for transfer")?;
    let file_size = file_path.metadata()?.len();
    let bar = make_progres_bar(file_size, "Sending file to device")?;

    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;
    let file_properties = make_values_for_create_file(storage.id().into(), file_name, file_size)?;
    let mut dest_writer = make_dest_writer(content.com_object(), &file_properties)?;
    let mut source_reader = File::open(file_path)?;
    let mut bytes_sent = 0_u64;
    let buffer_size = dest_writer.optimal_transfer_size().max(64 * 1024);
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

    dest_writer.commit()?;
    bar.finish_and_clear();
    Ok(())
}

fn make_values_for_create_file(
    parent_id: U16CString,
    file_name: &OsStr,
    file_size: u64,
) -> Result<IPortableDeviceValues> {
    let device_values: IPortableDeviceValues =
        unsafe { CoCreateInstance(&PortableDeviceValues as *const GUID, None, CLSCTX_ALL) }?;

    let file_name_wide = U16CString::from_os_str_truncate(file_name);
    let file_name_wide_ptr = PCWSTR::from_raw(file_name_wide.as_ptr());
    unsafe {
        device_values.SetStringValue(
            &WPD_OBJECT_PARENT_ID as *const _,
            PCWSTR::from_raw(parent_id.as_ptr()),
        )
    }?;
    unsafe { device_values.SetUnsignedLargeIntegerValue(&WPD_OBJECT_SIZE as *const _, file_size) }?;
    unsafe {
        device_values.SetStringValue(
            &WPD_OBJECT_ORIGINAL_FILE_NAME as *const _,
            file_name_wide_ptr,
        )
    }?;

    Ok(device_values)
}

fn make_dest_writer(
    com_object: &IPortableDeviceContent,
    file_properties: &IPortableDeviceValues,
) -> Result<WriteStream> {
    let mut write_stream = None;
    let mut optimal_write_buffer_size = 0;
    let mut cookie = PWSTR::null();
    unsafe {
        com_object.CreateObjectWithPropertiesAndData(
            file_properties,
            &mut write_stream as *mut _,
            &mut optimal_write_buffer_size,
            &mut cookie as *mut PWSTR,
        )
    }?;

    let write_stream = write_stream.context("Unable to create destination write stream")?;
    Ok(WriteStream::new(
        write_stream,
        optimal_write_buffer_size as usize,
    ))
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
