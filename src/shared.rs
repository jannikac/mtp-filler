use std::{
    cmp,
    fs::{File, remove_file},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use bytesize::ByteSize;
use uuid::Uuid;

pub const MIN_LEAVE_BYTES: u64 = 1024;

pub fn create_filler_file(
    current_free_bytes: ByteSize,
    desired_free_bytes: ByteSize,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    const BUFFER_SIZE: usize = 1024;

    validate_desired_free_space(current_free_bytes, desired_free_bytes)?;

    let filler_file_size = current_free_bytes - desired_free_bytes.as_u64();
    let filler_file_size: usize = filler_file_size.as_u64().try_into()?;

    // Put random uuid in file name to avoid overwriting an existing file with the same name.
    let uuid = Uuid::new_v4();
    let filler_path = PathBuf::from(format!("./{}_filler.txt", uuid));

    let f = File::create(&filler_path)?;
    let mut writer = BufWriter::new(f);

    let mut buffer = [0; BUFFER_SIZE];
    let mut remaining_size = filler_file_size;
    let total_size = filler_file_size as u64;
    let mut written = 0_u64;

    while remaining_size > 0 {
        let to_write = cmp::min(remaining_size, buffer.len());
        let buffer = &mut buffer[..to_write];
        fastrand::fill(buffer);
        writer.write_all(buffer)?;

        remaining_size -= to_write;
        written += to_write as u64;
        on_progress(written, total_size);
    }

    writer.flush()?;
    Ok(filler_path)
}

pub fn maybe_delete_filler_file(path: impl AsRef<Path>, should_delete: bool) -> Result<bool> {
    if should_delete {
        remove_file(path)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn validate_desired_free_space(
    current_free_bytes: ByteSize,
    desired_free_bytes: ByteSize,
) -> Result<()> {
    if desired_free_bytes >= current_free_bytes {
        Err(anyhow!(
            "Desired free bytes cannot be larger than current free space on device"
        ))
    } else if desired_free_bytes.as_u64() < MIN_LEAVE_BYTES {
        Err(anyhow!(
            "Desired free bytes must be larger than 1024 bytes (1 KiB)"
        ))
    } else {
        Ok(())
    }
}
