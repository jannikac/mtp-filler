use std::{str::FromStr, sync::mpsc};

use anyhow::{Result, anyhow};
use bytesize::ByteSize;
use dialoguer::{Confirm, Input, Select};
use mtp_filler::{AppState, BackendEvent};

use crate::{BackendWrite, shared::make_progres_bar};

fn prompt_device(app_state: &AppState) -> Result<usize> {
    if app_state.select_options.is_empty() {
        return Err(anyhow!("No attached MTP devices with storage detected"));
    }

    let items = app_state
        .select_options
        .iter()
        .map(|option| option.to_shared_string().to_string())
        .collect::<Vec<_>>();

    let selected_index = Select::new()
        .with_prompt("Select the device and storage to use")
        .default(0)
        .items(&items)
        .interact()?;

    Ok(selected_index)
}

fn prompt_desired_free_space(app_state: &AppState, selected_index: usize) -> Result<ByteSize> {
    let selected_option = app_state
        .select_options
        .get(selected_index)
        .ok_or_else(|| anyhow!("Invalid device selection"))?;

    let input = Input::<String>::new()
        .with_prompt("How much space should be left on device?")
        .default("10MiB".to_string())
        .validate_with(|input: &String| -> Result<(), String> {
            let desired_free_space = ByteSize::from_str(input).map_err(|e| e.to_string())?;
            app_state
                .validate_desired_free_space(selected_option, desired_free_space)
                .map_err(|e| e.to_string())
        })
        .interact_text()?;

    ByteSize::from_str(&input).map_err(|e| anyhow!(e))
}

fn prompt_keep_local() -> Result<bool> {
    Confirm::new()
        .with_prompt("Keep the local filler file after the transfer?")
        .default(false)
        .interact()
        .map_err(Into::into)
}

pub fn run_cli() -> Result<()> {
    let mut app_state = AppState::new()?;
    app_state.refresh()?;

    let selected_index = prompt_device(&app_state)?;
    let selected_option = app_state
        .select_options
        .get(selected_index)
        .ok_or_else(|| anyhow!("Invalid device selection"))?;
    let desired_free_space = prompt_desired_free_space(&app_state, selected_index)?;
    let keep_local = prompt_keep_local()?;

    let (evt_tx, evt_rx) = mpsc::channel::<BackendEvent>();
    let progress_thread = std::thread::spawn(move || -> Result<()> {
        let bar = make_progres_bar(0, "Preparing transfer")?;

        while let Ok(event) = evt_rx.recv() {
            if let BackendEvent::Write(BackendWrite::InProgress(sent, total, message)) = event {
                bar.set_message(message);
                bar.set_length(total);
                bar.set_position(sent);
            }
        }

        bar.finish_and_clear();
        Ok(())
    });

    app_state.write_mtp_file(desired_free_space, selected_index, keep_local, evt_tx)?;

    progress_thread
        .join()
        .map_err(|_| anyhow!("Progress thread panicked"))??;

    println!("Write finished successfully");
    Ok(())
}
