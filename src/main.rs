slint::include_modules!();
use anyhow::{Context, Result};
use mtp_filler::AppState;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

mod shared;

fn main() -> Result<()> {
    let mut app_state = AppState::new();
    let main_window = MainWindow::new()?;
    let main_window_weak = main_window.as_weak();
    let my_vec = vec![
        slint::SharedString::from("one"),
        slint::SharedString::from("two"),
        slint::SharedString::from("three"),
    ];

    main_window.set_combo_options(slint::ModelRc::new(slint::VecModel::from(my_vec)));
    main_window.on_refresh_clicked(move || {
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
            Err(e) => handle2.set_select_device_text(slint::SharedString::from(e.to_string())),
        }
    });

    main_window.run().context("Failed to run gui")
}
