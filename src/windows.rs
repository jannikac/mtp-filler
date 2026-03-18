use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
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

use crate::backend::{DeviceInfo, FillRequest, FillResult, FillStatus, ProgressUpdate, StorageInfo};
use crate::shared::{create_filler_file, maybe_delete_filler_file, validate_desired_free_space};

fn get_bytes_from_property(
    device: &Device,
    storage_id: U16CString,
    property_key: PROPERTYKEY,
) -> Result<ByteSize> {
    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;
    let properties = storage.properties(&[property_key])?;
    // WPD returns values as strings for these properties.
    let bytes = properties.get_string(&property_key)?;
    let bytes_u64 = bytes.to_string()?.parse::<u64>()?;
    Ok(ByteSize::b(bytes_u64))
}

fn get_capacity(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(device, storage_id, WPD_STORAGE_CAPACITY)
}

fn get_free_space(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(device, storage_id, WPD_STORAGE_FREE_SPACE_IN_BYTES)
}

fn open_device_by_index(index: usize) -> Result<Device> {
    let app_ident = winmtp::make_current_app_identifiers!();
    let provider = Provider::new()?;
    let raw_devices = provider.enumerate_devices()?;
    if raw_devices.is_empty() {
        return Err(anyhow!("No attached MTP devices detected"));
    }

    raw_devices
        .get(index)
        .context("Failed to select device")?
        .open(&app_ident, true)
        .map_err(Into::into)
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let provider = Provider::new()?;
    let raw_devices = provider.enumerate_devices()?;

    Ok(raw_devices
        .iter()
        .enumerate()
        .map(|(i, dev)| DeviceInfo {
            label: format!("ID {}: {}", i, dev.friendly_name()),
        })
        .collect())
}

pub fn list_storages(device_index: usize) -> Result<Vec<StorageInfo>> {
    let device = open_device_by_index(device_index)?;
    let content = device.content()?;
    let root = content.root()?;
    let children = root.children()?.into_iter().collect::<Vec<_>>();

    Ok(children
        .iter()
        .map(|v| {
            let capacity = get_capacity(&device, v.id().into()).ok();
            let free_space = get_free_space(&device, v.id().into()).ok();

            StorageInfo {
                label: format!(
                    "ID {}: {} (capacity: {} free space: {})",
                    v.id().to_string_lossy(),
                    v.name().to_string_lossy(),
                    capacity
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    free_space
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                ),
                free_bytes: free_space.map(|x| x.as_u64()),
            }
        })
        .collect())
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

fn send_file_to_device(
    device: &Device,
    storage_id: U16CString,
    file_path: impl AsRef<Path>,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let file_path = file_path.as_ref();
    let file_name = file_path
        .file_name()
        .context("Could not determine source file name for transfer")?;
    let file_size = file_path.metadata()?.len();

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
        on_progress(bytes_sent.min(file_size), file_size);
    }

    dest_writer.commit()?;
    Ok(())
}

pub fn run_fill(
    request: FillRequest,
    mut on_progress: impl FnMut(ProgressUpdate),
) -> Result<FillResult> {
    let device = open_device_by_index(request.device_index)?;
    let content = device.content()?;
    let root = content.root()?;
    let storages = root.children()?.into_iter().collect::<Vec<_>>();
    let storage = storages
        .get(request.storage_index)
        .context("Failed to select storage")?;
    let storage_id: U16CString = storage.id().into();

    let free_space = get_free_space(&device, storage_id.clone())?;
    validate_desired_free_space(free_space, request.desired_free_bytes)?;

    on_progress(ProgressUpdate::Status(FillStatus::CreatingLocalFillerFile));
    let filler_file_path =
        create_filler_file(free_space, request.desired_free_bytes, |written, total| {
            on_progress(ProgressUpdate::LocalFileProgress { written, total })
        })?;

    let filler_file_path = canonicalize(filler_file_path)?;
    on_progress(ProgressUpdate::Status(FillStatus::SendingFileToDevice));
    send_file_to_device(
        &device,
        storage_id.clone(),
        &filler_file_path,
        |sent, total| on_progress(ProgressUpdate::TransferProgress { sent, total }),
    )?;
    on_progress(ProgressUpdate::Status(FillStatus::FileWrittenToDevice));

    on_progress(ProgressUpdate::Status(FillStatus::FinalizingTransfer));
    maybe_delete_filler_file(&filler_file_path, request.delete_local_file)?;

    let remaining_free_space = get_free_space(&device, storage_id)?;
    Ok(FillResult {
        remaining_free_space,
    })
}
