use anyhow::Result;
use bytesize::ByteSize;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct StorageInfo {
    pub label: String,
    pub free_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct FillRequest {
    pub device_index: usize,
    pub storage_index: usize,
    pub desired_free_bytes: ByteSize,
    pub delete_local_file: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillStatus {
    CreatingLocalFillerFile,
    SendingFileToDevice,
    FileWrittenToDevice,
    FinalizingTransfer,
}

impl FillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatingLocalFillerFile => "Creating local filler file",
            Self::SendingFileToDevice => "Sending file to device",
            Self::FileWrittenToDevice => "File successfully written to device",
            Self::FinalizingTransfer => "Finalizing transfer",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ProgressUpdate {
    Status(FillStatus),
    LocalFileProgress { written: u64, total: u64 },
    TransferProgress { sent: u64, total: u64 },
}

#[derive(Clone, Debug)]
pub struct FillResult {
    pub remaining_free_space: ByteSize,
}

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;

pub use platform::{list_devices, list_storages, run_fill};

#[cfg(windows)]
fn run_on_worker_thread<T: Send + 'static>(
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    std::thread::spawn(task)
        .join()
        .map_err(|_| anyhow::anyhow!("Background worker thread panicked"))?
}

pub fn list_devices_safe() -> Result<Vec<DeviceInfo>> {
    #[cfg(windows)]
    {
        return run_on_worker_thread(list_devices);
    }

    #[cfg(not(windows))]
    {
        list_devices()
    }
}

pub fn list_storages_safe(device_index: usize) -> Result<Vec<StorageInfo>> {
    #[cfg(windows)]
    {
        return run_on_worker_thread(move || list_storages(device_index));
    }

    #[cfg(not(windows))]
    {
        list_storages(device_index)
    }
}
