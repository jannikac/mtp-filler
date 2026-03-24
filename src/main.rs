slint::include_modules!();
use std::str::FromStr;

use anyhow::{Context, Result};
use bytesize::ByteSize;
use mtp_filler::AppState;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

mod shared;

fn main() -> Result<()> {
    let app_state = Rc::new(RefCell::new(AppState::new()));
    let main_window = MainWindow::new()?;
    let main_window_weak = main_window.as_weak();
    let my_vec = vec![
        slint::SharedString::from("one"),
        slint::SharedString::from("two"),
        slint::SharedString::from("three"),
    ];

    let handle2 = main_window_weak.clone();
    let app_state_for_write = app_state.clone();
    main_window.on_write_clicked(move |space_to_leave, selected_index, keep_local| {
        let handle2 = handle2.upgrade().unwrap();
        let app_state = app_state_for_write.borrow();

        handle2.set_space_to_leave_error("".into());

        let space_to_leave = match ByteSize::from_str(&space_to_leave) {
            Ok(v) => v,
            Err(e) => {
                handle2.set_space_to_leave_error(slint::SharedString::from(e));
                return ();
            }
        };

        let device_index: usize = match selected_index.try_into() {
            Ok(v) => v,
            Err(_) => {
                handle2
                    .set_select_device_error(slint::SharedString::from("Please select a device"));
                return ();
            }
        };

        let selected_option = match &app_state.select_options.get(device_index) {
            Some(v) => *v,
            None => {
                handle2
                    .set_select_device_error(slint::SharedString::from("Invalid device selection"));
                return ();
            }
        };
        dbg!(space_to_leave, selected_index, keep_local);
    });

    main_window.set_combo_options(slint::ModelRc::new(slint::VecModel::from(my_vec)));
    let app_state_for_refresh = app_state.clone();
    main_window.on_refresh_clicked(move || {
        let mut app_state = app_state_for_refresh.borrow_mut();
        let handle2 = main_window_weak.upgrade().unwrap();
        let refresh_result = app_state.refresh();

        match refresh_result {
            Ok(_) => {
                let options = app_state
                    .select_options
                    .iter()
                    .map(|v| v.to_shared_string())
                    .collect::<Vec<_>>();
                handle2.set_combo_options(slint::ModelRc::new(slint::VecModel::from(options)));
            }
            Err(e) => handle2.set_select_device_error(slint::SharedString::from(e.to_string())),
        }
    });

    main_window.run().context("Failed to run gui")
}
