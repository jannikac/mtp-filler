use std::{
    collections::HashMap,
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
    messages::SelectOption,
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
    handle: Device,
    storages: Vec<StorageInfo>,
}

impl DeviceState {
    fn open(raw: BasicDevice, app_ident: AppIdentifiers) -> Result<(DeviceKey, Self)> {
        let key = DeviceKey {
            id: raw.device_id(),
        };
        let mut handle = raw
            .open(&app_ident, true)
            .context("Failed to open device")?;
        let storages = Self::load_storages(&mut handle)?;

        Ok((key.clone(), Self { handle, storages }))
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
struct DeviceKey {
    id: String,
}

#[derive(Debug, Clone)]
struct Selection {
    device: DeviceKey,
    storage_id: U16CString,
}

pub struct AppState {
    pub devices: HashMap<DeviceKey, DeviceState>,
    pub entries: Vec<Entry>,
    app_ident: AppIdentifiers,
    provider: Provider,
}

pub struct Entry {
    pub option: SelectOption,
    pub selection: Selection,
}

impl AppState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            devices: HashMap::new(),
            entries: vec![],
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

    fn rebuild_entries(&mut self) {
        self.entries = self
            .devices
            .iter()
            .flat_map(|(device_key, device)| {
                device.storages.iter().map(|storage| Entry {
                    option: SelectOption {
                        label: SharedString::from(format!(
                            "{} {}",
                            device_key.id,
                            storage.name.to_string_lossy()
                        )),
                    },
                    selection: Selection {
                        device: device_key.clone(),
                        storage_id: storage.id.clone(),
                    },
                })
            })
            .collect();
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.refresh_devices()?;
        self.rebuild_entries();
        Ok(())
    }

    fn reuse_or_open(
        &self,
        old_devices: &mut HashMap<DeviceKey, DeviceState>,
        raw: BasicDevice,
    ) -> Result<(DeviceKey, DeviceState)> {
        let device_key = DeviceKey {
            id: raw.device_id(),
        };

        let device_tuple = match old_devices.remove(&device_key) {
            Some(mut existing) => {
                existing.refresh_storages()?;
                Ok((device_key, existing))
            }
            None => DeviceState::open(raw, self.app_ident.clone()),
        }?;
        Ok(device_tuple)
    }

    fn get_selections(&self) -> Vec<Selection> {
        self.devices
            .iter()
            .flat_map(|(device_key, device)| {
                device.storages.iter().map(|storage| Selection {
                    device: device_key.clone(),
                    storage_id: storage.id.clone(),
                })
            })
            .collect::<Vec<_>>()
    }

    fn selection(&self, selected_index: usize) -> Result<&Selection> {
        self.entries
            .get(selected_index)
            .map(|entry| &entry.selection)
            .context("failed to select device")
    }

    pub fn get_select_options(&self) -> Vec<SelectOption> {
        self.entries
            .iter()
            .map(|entry| entry.option.clone())
            .collect()
    }

    fn write_to_storage(
        &self,
        selection: &Selection,
        filler_file_path: impl AsRef<Path>,
        evt_tx: Sender<BackendEvent>,
    ) -> Result<()> {
        let device_state = self.devices.get(&selection.device).context("asd")?;
        send_file_to_device_with_callback(
            &device_state.handle,
            selection.storage_id.clone(),
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

    fn calculate_filler_size(
        &self,
        selection: &Selection,
        desired_free_space: ByteSize,
    ) -> Result<ByteSize> {
        let dev = self
            .devices
            .get(&selection.device)
            .context("failed to get device")?;
        let storage = dev
            .storages
            .iter()
            .find(|v| v.id == selection.storage_id)
            .context("asd")?;
        let filler_file_size = storage.free_space - desired_free_space;
        Ok(filler_file_size)
    }

    fn validate_desired_free_space(
        &self,
        selection: &Selection,
        desired_free_space: ByteSize,
    ) -> Result<()> {
        let dev = self
            .devices
            .get(&selection.device)
            .context("failed to get device")?;
        let storage = dev
            .storages
            .iter()
            .find(|v| v.id == selection.storage_id)
            .context("asd")?;
        let current_free_bytes = storage.free_space;

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
        let selection = self.selection(selected_index)?;
        self.validate_desired_free_space(selection, space_to_leave)?;

        let filler_file_path = create_filler_file2(
            self.calculate_filler_size(selection, space_to_leave)?,
            evt_tx.clone(),
        )?;
        let filler_file_path = filler_file_path.canonicalize()?;

        self.write_to_storage(selection, &filler_file_path, evt_tx)?;

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
