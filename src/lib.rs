use std::{
    collections::HashMap,
    fmt::Display,
    fs::{metadata, remove_file},
    path::Path,
    sync::mpsc::Sender,
};

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use libmtp_rs::{
    device::{
        MtpDevice, StorageSort,
        raw::{RawDevice, detect_raw_devices},
    },
    object::filetypes::Filetype,
    storage::{Parent, Storage, files::FileMetadata},
    util::CallbackReturn,
};
use slint::SharedString;

use crate::shared::{create_filler_file, create_filler_file2};

mod shared;

const MIN_DESIRED_FREE_SPACE_BYTES: u64 = 1024;

pub enum BackendCommand {
    Refresh,
    Write {
        space_to_leave: ByteSize,
        selected_index: usize,
        keep_local: bool,
    },
    Exit,
}

pub enum BackendWrite {
    InProgress(u64, u64, &'static str),
    Completed(Result<()>),
}

pub enum BackendEvent {
    RefreshFinished(anyhow::Result<Vec<slint::SharedString>>),
    Write(BackendWrite),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceInfo {
    bus_number: u32,
    dev_number: u8,
    vendor_id: u16,
    vendor: String,
    product_id: u16,
    product: String,
}

impl DeviceInfo {
    fn from_raw_device(raw: &RawDevice) -> Self {
        let entry = raw.device_entry();
        Self {
            bus_number: raw.bus_number(),
            dev_number: raw.dev_number(),
            vendor_id: entry.vendor_id,
            vendor: entry.vendor.to_string(),
            product_id: entry.product_id,
            product: entry.product.to_string(),
        }
    }
}

impl Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Bus {} Dev {}: {} {} (VID: {}, PID: {})",
            self.bus_number,
            self.dev_number,
            self.vendor,
            self.product,
            self.vendor_id,
            self.product_id
        )
    }
}

impl From<RawDevice> for DeviceInfo {
    fn from(value: RawDevice) -> Self {
        Self::from_raw_device(&value)
    }
}

impl From<&RawDevice> for DeviceInfo {
    fn from(value: &RawDevice) -> Self {
        Self::from_raw_device(value)
    }
}

impl PartialEq<RawDevice> for DeviceInfo {
    fn eq(&self, other: &RawDevice) -> bool {
        let entry = other.device_entry();
        self.bus_number == other.bus_number()
            && self.dev_number == other.dev_number()
            && self.vendor_id == entry.vendor_id
            && self.product_id == entry.product_id
    }
}

#[derive(Debug, Clone)]
pub struct StorageInfo {
    id: u32,
    description: Option<String>,
    free_space: ByteSize,
    capacity: ByteSize,
}

impl From<&Storage<'_>> for StorageInfo {
    fn from(storage: &Storage) -> Self {
        StorageInfo {
            id: storage.id(),
            description: storage.description().map(|v| v.to_string()),
            free_space: ByteSize::b(storage.free_space_in_bytes()),
            capacity: ByteSize::b(storage.maximum_capacity()),
        }
    }
}

impl Display for StorageInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ID {}: {} (capacity: {}, free space: {})",
            self.id,
            self.description.as_deref().unwrap_or("unknown"),
            self.capacity,
            self.free_space
        )
    }
}

#[derive(Debug)]
pub struct SelectOption {
    device: DeviceInfo,
    storage: StorageInfo,
}

impl SelectOption {
    pub fn to_shared_string(&self) -> SharedString {
        let s = format!("{}\n{}", self.device, self.storage);
        SharedString::from(s)
    }
}

pub struct DeviceState {
    info: DeviceInfo,
    handle: MtpDevice,
    storages: Vec<StorageInfo>,
}

impl DeviceState {
    fn open(raw: RawDevice) -> Result<(DeviceInfo, Self)> {
        let info = DeviceInfo::from(&raw);
        let mut handle = raw.open_uncached().context("Failed to open device")?;
        let storages = Self::load_storages(&mut handle)?;

        Ok((
            info.clone(),
            Self {
                info,
                handle,
                storages,
            },
        ))
    }

    fn refresh_storages(&mut self) -> Result<()> {
        self.storages = Self::load_storages(&mut self.handle)?;
        Ok(())
    }

    fn load_storages(handle: &mut MtpDevice) -> Result<Vec<StorageInfo>> {
        handle.update_storage(StorageSort::NotSorted)?;
        let storages = handle
            .storage_pool()
            .iter()
            .map(|(_, storage)| StorageInfo::from(storage))
            .collect::<Vec<_>>();

        if storages.is_empty() {
            return Err(anyhow!("No storage pools in device"));
        }

        Ok(storages)
    }
}

pub struct AppState {
    pub devices: HashMap<DeviceInfo, DeviceState>,
    pub select_options: Vec<SelectOption>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            select_options: vec![],
        }
    }

    fn reuse_or_open(
        &self,
        old_devices: &mut HashMap<DeviceInfo, DeviceState>,
        raw: RawDevice,
    ) -> Result<(DeviceInfo, DeviceState)> {
        let device_info = DeviceInfo::from(&raw);

        let device_tuple = match old_devices.remove(&device_info) {
            Some(mut existing) => {
                existing.refresh_storages()?;
                Ok((device_info, existing))
            }
            None => DeviceState::open(raw),
        }?;
        Ok(device_tuple)
    }

    fn refresh_devices(&mut self) -> Result<()> {
        let mut old_devices = std::mem::take(&mut self.devices);

        let devices = detect_raw_devices()?
            .into_iter()
            .map(|raw| self.reuse_or_open(&mut old_devices, raw))
            .collect::<Result<HashMap<_, _>>>()?;
        self.devices = devices;
        Ok(())
    }

    fn refresh_select_options(&mut self) {
        let select_options = self.get_select_options();
        self.select_options = select_options;
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.refresh_devices()?;
        self.refresh_select_options();
        Ok(())
    }

    fn get_select_options(&self) -> Vec<SelectOption> {
        self.devices
            .iter()
            .flat_map(|(device_info, device)| {
                device.storages.iter().map(|storage| SelectOption {
                    device: device_info.clone(),
                    storage: storage.clone(),
                })
            })
            .collect::<Vec<_>>()
    }

    fn write_to_storage(
        &self,
        storage_info: &SelectOption,
        filler_file_path: impl AsRef<Path>,
        metadata: FileMetadata,
        evt_tx: Sender<BackendEvent>,
    ) -> Result<()> {
        let device_state = self
            .devices
            .get(&storage_info.device)
            .context("No device found")?;
        let pool = device_state.handle.storage_pool();
        let storage = pool
            .by_id(storage_info.storage.id)
            .context("No storage found")?;

        storage.send_file_from_path_with_callback(
            &filler_file_path,
            Parent::Root,
            metadata,
            |sent, total| {
                let _ = evt_tx.send(BackendEvent::Write(BackendWrite::InProgress(
                    sent,
                    total,
                    "Sending to device (2/2)",
                )));
                CallbackReturn::Continue
            },
        )?;

        Ok(())
    }

    pub fn calculate_filler_size(
        &self,
        selected_option: &SelectOption,
        desired_free_space: ByteSize,
    ) -> ByteSize {
        let filler_file_size = selected_option.storage.free_space - desired_free_space;
        filler_file_size
    }

    pub fn validate_desired_free_space(
        &self,
        selected_option: &SelectOption,
        desired_free_space: ByteSize,
    ) -> Result<()> {
        let current_free_bytes = selected_option.storage.free_space;

        if desired_free_space >= current_free_bytes {
            Err(anyhow!(
                "Desired free bytes cannot be larger than or equal to the current free space on device"
            ))
        } else if desired_free_space < ByteSize::b(MIN_DESIRED_FREE_SPACE_BYTES) {
            Err(anyhow!(
                "Desired free bytes must be larger than 1024 bytes (1 KiB)"
            ))
        } else {
            Ok(())
        }
    }

    pub fn write_mtp_file(
        &self,
        space_to_leave: ByteSize,
        selected_device: &SelectOption,
        keep_local: bool,
        evt_tx: Sender<BackendEvent>,
    ) -> Result<()> {
        self.validate_desired_free_space(selected_device, space_to_leave)?;

        let filler_file_path = create_filler_file2(
            self.calculate_filler_size(selected_device, space_to_leave),
            evt_tx.clone(),
        )?;
        let filler_file_path = filler_file_path.canonicalize()?;
        let meta = get_metadata(&filler_file_path)?;

        self.write_to_storage(selected_device, &filler_file_path, meta, evt_tx)?;

        if !keep_local {
            remove_file(filler_file_path)?;
        }
        Ok(())
    }
}

fn get_metadata(path: &Path) -> Result<FileMetadata<'_>> {
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
