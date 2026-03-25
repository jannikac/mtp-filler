use std::{
    borrow::Cow,
    cmp,
    fs::{File, remove_file},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use bytesize::ByteSize;
use dialoguer::{Confirm, Input};
use indicatif::{ProgressBar, ProgressStyle};
use uuid::Uuid;

use crate::{BackendEvent, BackendWrite};

pub fn make_progres_bar(size: u64, message: impl Into<Cow<'static, str>>) -> Result<ProgressBar> {
    let bar = ProgressBar::new(size).with_message(message).with_style(
        ProgressStyle::with_template(
            "{msg:40}  [{wide_bar}] {percent}% ({binary_bytes}/{binary_total_bytes})",
        )?
        .progress_chars("## "),
    );
    Ok(bar)
}

pub fn create_filler_file2(filler_size: ByteSize, evt_tx: Sender<BackendEvent>) -> Result<PathBuf> {
    const BUFFER_SIZE: usize = 1024;
    let filler_file_size: usize = filler_size.as_u64().try_into()?;

    // put random uuid in file name to avoid overwriting an existing file with the same name
    let uuid = Uuid::new_v4();

    let filler_path = PathBuf::from(format!("./{}_filler.txt", uuid.to_string()));
    let f = File::create(&filler_path)?;

    let mut writer = BufWriter::new(f);

    let mut buffer = [0; BUFFER_SIZE];
    let mut remaining_size = filler_file_size;

    while remaining_size > 0 {
        let to_write = cmp::min(remaining_size, buffer.len());
        let buffer = &mut buffer[..to_write];
        fastrand::fill(buffer);
        writer.write_all(buffer)?;

        remaining_size -= to_write;
        let _ = evt_tx.send(BackendEvent::Write(crate::BackendWrite::InProgress(
            (filler_file_size - remaining_size).try_into().unwrap(),
            filler_file_size.try_into().unwrap(),
            "Creating filler file (1/2)",
        )));
    }
    Ok(filler_path)
}

pub fn create_filler_file(current_free_bytes: ByteSize) -> Result<PathBuf> {
    const BUFFER_SIZE: usize = 1024;
    let input_size = Input::new()
        .with_prompt("How much space should be left on device?")
        .validate_with(|input: &String| -> Result<(), String> {
            let input_size = ByteSize::from_str(&input)?;
            if input_size >= current_free_bytes {
                Err(
                    "Desired free bytes cannot be larger than current free space on device"
                        .to_string(),
                )
            } else if input_size < ByteSize::b(BUFFER_SIZE.try_into().unwrap()) {
                Err("Desired free bytes must be larger than 1024 bytes (1 KiB)".to_string())
            } else {
                Ok(())
            }
        })
        .default("10MiB".to_string())
        .interact_text()?;
    let input_bytes = ByteSize::from_str(&input_size).map_err(|e| anyhow!(e))?;

    let filler_file_size = current_free_bytes - input_bytes.as_u64();
    let filler_file_size: usize = filler_file_size.as_u64().try_into()?;

    // put random uuid in file name to avoid overwriting an existing file with the same name
    let uuid = Uuid::new_v4();

    let filler_path = PathBuf::from(format!("./{}_filler.txt", uuid.to_string()));

    let f = File::create(&filler_path)?;
    let mut writer = BufWriter::new(f);
    let bar = make_progres_bar((filler_file_size).try_into()?, "Creating filler file")?;

    let mut buffer = [0; BUFFER_SIZE];
    let mut remaining_size = filler_file_size;

    while remaining_size > 0 {
        let to_write = cmp::min(remaining_size, buffer.len());
        let buffer = &mut buffer[..to_write];
        fastrand::fill(buffer);
        writer.write_all(buffer)?;

        remaining_size -= to_write;
        bar.inc(to_write.try_into()?);
    }
    bar.finish_and_clear();
    Ok(filler_path)
}

pub fn delete_fillter_file(path: impl AsRef<Path>) -> Result<()> {
    let prompt = format!(
        "Delete the local filler file? ({})",
        path.as_ref().display()
    );
    let input = Confirm::new()
        .with_prompt(prompt)
        .default(true)
        .interact()?;
    if input {
        remove_file(path)?;
    }
    Ok(())
}

const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(50);

pub struct ThrottledProgressReporter {
    evt_tx: Sender<BackendEvent>,
    message: &'static str,
    last_reported_at: Option<Instant>,
}

impl ThrottledProgressReporter {
    pub fn new(evt_tx: Sender<BackendEvent>, message: &'static str) -> Self {
        Self {
            evt_tx,
            message,
            last_reported_at: None,
        }
    }

    pub fn emit(&mut self, sent: u64, total: u64) {
        let should_emit = sent == 0
            || sent >= total
            || self.last_reported_at.is_none_or(|last_reported_at| {
                last_reported_at.elapsed() >= PROGRESS_UPDATE_INTERVAL
            });

        if should_emit {
            self.last_reported_at = Some(Instant::now());
            let _ = self
                .evt_tx
                .send(BackendEvent::Write(BackendWrite::InProgress(
                    sent,
                    total,
                    self.message,
                )));
        }
    }
}

pub fn create_filler_file_with_progress(
    filler_size: ByteSize,
    evt_tx: &Sender<BackendEvent>,
) -> Result<PathBuf> {
    const BUFFER_SIZE: usize = 1024;
    let filler_file_size: usize = filler_size.as_u64().try_into()?;

    let uuid = Uuid::new_v4();
    let filler_path = PathBuf::from(format!("./{}_filler.txt", uuid));
    let file = File::create(&filler_path)?;
    let mut writer = BufWriter::new(file);
    let mut buffer = [0; BUFFER_SIZE];
    let mut remaining_size = filler_file_size;
    let total_bytes = filler_file_size as u64;
    let mut progress = ThrottledProgressReporter::new(evt_tx.clone(), "Creating filler file (1/2)");

    while remaining_size > 0 {
        let to_write = cmp::min(remaining_size, buffer.len());
        let buffer = &mut buffer[..to_write];
        fastrand::fill(buffer);
        writer.write_all(buffer)?;

        remaining_size -= to_write;
        progress.emit((filler_file_size - remaining_size) as u64, total_bytes);
    }

    Ok(filler_path)
}
