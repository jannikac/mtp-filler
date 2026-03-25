use std::{
    collections::HashMap,
    fmt::Display,
    fs::{File, remove_file},
    io::{Read, Write},
    path::Path,
    sync::mpsc::Sender,
};

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;

use slint::SharedString;
use widestring::U16CString;
use winmtp::{
    PROPERTYKEY,
    PortableDevices::{WPD_STORAGE_CAPACITY, WPD_STORAGE_FREE_SPACE_IN_BYTES},
    Provider,
    device::{BasicDevice, Device, device_values::AppIdentifiers},
    make_current_app_identifiers,
};

use crate::{
    BackendEvent, BackendWrite,
    shared::{create_filler_file, create_filler_file2},
};

const MIN_DESIRED_FREE_SPACE_BYTES: u64 = 1024;

#[derive(Debug, Clone)]
pub struct StorageInfo {
    id: U16CString,
    name: U16CString,
    free_space: ByteSize,
    capacity: ByteSize,
}

#[derive(Debug, Clone)]
pub struct SelectOption {
    device: DeviceKey,
    storage: StorageInfo,
    pub label: SharedString,
}

impl SelectOption {
    pub fn to_shared_string(&self) -> SharedString {
        let s = format!("{}\n{}", self.device, self.storage);
        SharedString::from(s)
    }
}

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

fn get_capacity(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(&device, storage_id, WPD_STORAGE_CAPACITY)
}

fn get_free_space(device: &Device, storage_id: U16CString) -> Result<ByteSize> {
    get_bytes_from_property(&device, storage_id, WPD_STORAGE_FREE_SPACE_IN_BYTES)
}

pub struct DeviceState {
    info: DeviceKey,
    handle: Device,
    storages: Vec<StorageInfo>,
}

impl DeviceState {
    fn open(raw: BasicDevice, app_ident: AppIdentifiers) -> Result<(DeviceKey, Self)> {
        let info = DeviceKey::from(&raw);
        let mut handle = raw
            .open(&app_ident, true)
            .context("Failed to open device")?;
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

    fn load_storages(handle: &mut Device) -> Result<Vec<StorageInfo>> {
        let root = handle.content()?.root()?;
        let children = root.children()?.into_iter().collect::<Vec<_>>();
        let storages = children
            .iter()
            .map(|storage| StorageInfo {
                id: storage.id().into(),
                capacity: get_capacity(handle, storage.id().into()).unwrap(),
                free_space: get_free_space(handle, storage.id().into()).unwrap(),
                name: storage.name().into(),
            })
            .collect::<Vec<_>>();

        if storages.is_empty() {
            return Err(anyhow!("No storage pools in device"));
        }

        Ok(storages)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey {
    id: String,
    friendly_name: String,
}

impl From<&BasicDevice> for DeviceKey {
    fn from(basic_device: &BasicDevice) -> Self {
        Self {
            id: basic_device.device_id(),
            friendly_name: basic_device.friendly_name().to_string(),
        }
    }
}

impl Display for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Device {}", self.id)
    }
}

impl Display for StorageInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (capacity: {}, free space: {})",
            self.name.to_string_lossy(),
            self.capacity,
            self.free_space
        )
    }
}

pub struct AppState {
    pub devices: HashMap<DeviceKey, DeviceState>,
    pub select_options: Vec<SelectOption>,
    app_ident: AppIdentifiers,
    provider: Provider,
}

impl AppState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            devices: HashMap::new(),
            select_options: vec![],
            app_ident: make_current_app_identifiers!(),
            provider: Provider::new()?,
        })
    }

    fn refresh_devices(&mut self) -> Result<()> {
        let mut old_devices = std::mem::take(&mut self.devices);
        let basic_devices = self.provider.enumerate_devices()?;

        let devices = basic_devices
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

    fn reuse_or_open(
        &self,
        old_devices: &mut HashMap<DeviceKey, DeviceState>,
        raw: BasicDevice,
    ) -> Result<(DeviceKey, DeviceState)> {
        let device_key = DeviceKey::from(&raw);

        let device_tuple = match old_devices.remove(&device_key) {
            Some(mut existing) => {
                existing.refresh_storages()?;
                Ok((device_key, existing))
            }
            None => DeviceState::open(raw, self.app_ident.clone()),
        }?;
        Ok(device_tuple)
    }

    pub fn get_select_options(&self) -> Vec<SelectOption> {
        self.devices
            .iter()
            .flat_map(|(_, device)| {
                let device_info = &device.info;
                device.storages.iter().map(move |storage| SelectOption {
                    device: device_info.clone(),
                    storage: storage.clone(),
                    label: SharedString::from(format!("{}\n{}", device_info, storage)),
                })
            })
            .collect::<Vec<_>>()
    }

    fn write_to_storage(
        &self,
        storage_info: &SelectOption,
        filler_file_path: impl AsRef<Path>,
        evt_tx: Sender<BackendEvent>,
    ) -> Result<()> {
        let device_state = self
            .devices
            .get(&storage_info.device)
            .context("No device found")?;
        send_file_to_device_with_callback(
            &device_state.handle,
            storage_info.storage.id.clone(),
            filler_file_path,
            |sent, total| {
                let _ = evt_tx.send(BackendEvent::Write(BackendWrite::InProgress(
                    sent,
                    total,
                    "Sending to device (2/2)",
                )));
                Ok(())
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
        selected_index: usize,
        keep_local: bool,
        evt_tx: Sender<BackendEvent>,
    ) -> Result<()> {
        let selected_device = self
            .select_options
            .get(selected_index)
            .context("failed to select device")?;
        self.validate_desired_free_space(selected_device, space_to_leave)?;

        let filler_file_path = create_filler_file2(
            self.calculate_filler_size(selected_device, space_to_leave),
            evt_tx.clone(),
        )?;
        let filler_file_path = filler_file_path.canonicalize()?;

        self.write_to_storage(selected_device, &filler_file_path, evt_tx)?;

        if !keep_local {
            remove_file(filler_file_path)?;
        }
        Ok(())
    }
}

fn send_file_to_device_with_callback<F: FnMut(u64, u64) -> Result<()>>(
    device: &Device,
    storage_id: U16CString,
    file_path: impl AsRef<Path>,
    mut on_progress: F,
) -> Result<()> {
    let file_path = file_path.as_ref();
    let file_name = file_path
        .file_name()
        .context("Could not determine source file name for transfer")?;
    let file_size = file_path.metadata()?.len();

    let content = device.content()?;
    let storage = content.object_by_id(storage_id)?;
    let mut dest_writer = storage.create_write_stream(file_name, file_size, false)?;
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
        on_progress(bytes_sent, file_size)?;
    }

    dest_writer.flush()?;
    Ok(())
}
