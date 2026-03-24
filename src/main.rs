slint::include_modules!();
use std::str::FromStr;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, anyhow};
use bytesize::ByteSize;
use mtp_filler::{AppState, BackendCommand, BackendEvent, BackendWrite};
use slint::SharedString;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

mod shared;

// handlers for commands
fn handle_refresh(app_state: &mut AppState) -> Result<Vec<SharedString>> {
    app_state.refresh()?;
    let result = app_state
        .select_options
        .iter()
        .map(|v| v.to_shared_string())
        .collect::<Vec<_>>();
    Ok(result)
}

fn handle_write(
    app_state: &mut AppState,
    space_to_leave: ByteSize,
    selected_index: usize,
    keep_local: bool,
    evt_tx: Sender<BackendEvent>,
) -> Result<()> {
    let selected_option = app_state
        .select_options
        .get(selected_index)
        .ok_or_else(|| anyhow!("Invalid device selection"))?;
    app_state.write_mtp_file(space_to_leave, selected_option, keep_local, evt_tx)?;
    Ok(())
}

fn main() -> Result<()> {
    let main_window = MainWindow::new()?;
    let main_window_weak_sync_thread = main_window.as_weak();
    let main_window_weak = main_window.as_weak();

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<BackendCommand>();
    let (evt_tx, evt_rx) = std::sync::mpsc::channel::<BackendEvent>();

    // worker-thread
    // owns the device handles and handles long-running tasks
    // handles are not send which is why they will remain in worker thread
    std::thread::spawn(move || {
        let mut app_state = AppState::new();

        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                BackendCommand::Refresh => {
                    let res = handle_refresh(&mut app_state);
                    evt_tx.send(BackendEvent::RefreshFinished(res));
                }
                BackendCommand::Write {
                    space_to_leave,
                    selected_index,
                    keep_local,
                } => {
                    let res = handle_write(
                        &mut app_state,
                        space_to_leave,
                        selected_index,
                        keep_local,
                        evt_tx.clone(),
                    );
                    evt_tx.send(BackendEvent::Write(BackendWrite::Completed(res)));
                }
            }
        }
    });

    // sync-thread
    // thread that updates slint ui based on events from channel
    std::thread::spawn(move || {
        while let Ok(evt) = evt_rx.recv() {
            let weak = main_window_weak_sync_thread.clone();

            slint::invoke_from_event_loop(move || {
                let window = weak.upgrade().unwrap();
                match evt {
                    BackendEvent::RefreshFinished(shared_strings) => match shared_strings {
                        Ok(options) => {
                            window.set_select_device_error(slint::SharedString::from(""));
                            window.set_combo_options(slint::ModelRc::new(slint::VecModel::from(
                                options,
                            )));
                        }
                        Err(e) => {
                            window
                                .set_select_device_error(slint::SharedString::from(e.to_string()));
                        }
                    },
                    BackendEvent::Write(event) => match event {
                        BackendWrite::InProgress(sent, total) => {
                            window.set_sent_bytes(sent.try_into().unwrap());
                            window.set_total_bytes(total.try_into().unwrap());
                        }
                        BackendWrite::Completed(_) => {
                            window.set_select_device_error(slint::SharedString::from("done"))
                        }
                    },
                }
            })
            .expect("Failed to add event to slint loop");
        }
    });

    {
        let cmd_tx = cmd_tx.clone();
        let weak = main_window_weak.clone();

        main_window.on_write_clicked(move |space_to_leave, selected_index, keep_local| {
            let handle2 = weak.upgrade().unwrap();

            handle2.set_space_to_leave_error("".into());

            let space_to_leave = match ByteSize::from_str(&space_to_leave) {
                Ok(v) => v,
                Err(e) => {
                    handle2.set_space_to_leave_error(slint::SharedString::from(e));
                    return ();
                }
            };

            let selected_index: usize = match selected_index.try_into() {
                Ok(v) => v,
                Err(_) => {
                    handle2.set_select_device_error(slint::SharedString::from(
                        "Please select a device",
                    ));
                    return ();
                }
            };

            let _ = cmd_tx.send(BackendCommand::Write {
                space_to_leave,
                selected_index,
                keep_local,
            });
        });
    }

    {
        let cmd_tx = cmd_tx.clone();

        main_window.on_refresh_clicked(move || {
            let _ = cmd_tx.send(BackendCommand::Refresh);
        });
    }

    main_window.run().context("Failed to run gui")
}
