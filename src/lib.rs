use std::{collections::HashMap, fmt::Display, time::Duration};

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use libmtp_rs::{
    device::{
        MtpDevice, StorageSort,
        raw::{RawDevice, detect_raw_devices},
    },
    storage::Storage,
};
use slint::SharedString;

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
    pub fn write_mtp_file(
        &self,
        space_to_leave: ByteSize,
        selected_device: &SelectOption,
        keep_local: bool,
    ) -> Result<()> {
        // simulate write
        std::thread::sleep(Duration::from_secs(5));
        Ok(())
    }
}
