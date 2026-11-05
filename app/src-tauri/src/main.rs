//! the Footprints menu bar app: today's recorded changes and an undo button.
//!
//! it reads the same ledger as the CLI (`FOOTPRINTS_HOME` or the platform default) and undoes
//! through the same library call, so the conflict check is exactly the one `footprints undo`
//! runs. There is no force option here: a conflict is shown and the file is left alone.

// a release build on Windows would otherwise open a console window beside the app
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rows;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

#[tauri::command]
fn today() -> Result<rows::Today, String> {
    rows::today().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn undo(id: i64) -> Result<String, String> {
    rows::undo(id).map_err(|e| format!("{e:#}"))
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        // the page reloads its list whenever it becomes visible
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![today, undo])
        .setup(|app| {
            // a menu bar app has no Dock icon
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let open = MenuItem::with_id(app, "open", "Show today's changes", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Footprints", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            // the 32 point @2x template: macOS scales the menu bar image down to fit, so the
            // larger source stays sharp on a Retina display
            let icon = tauri::image::Image::from_bytes(include_bytes!(
                "../icons/tray/FootprintsTemplate-32@2x.png"
            ))?;
            TrayIconBuilder::with_id("footprints")
                .icon(icon)
                // a template image follows the menu bar's light or dark appearance
                .icon_as_template(true)
                .tooltip("Footprints")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the window only hides it; the app lives in the menu bar
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("the Footprints app failed to start");
}
