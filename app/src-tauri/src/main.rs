//! the Footprints menu bar app: today's recorded changes and an undo button. It lives in the
//! macOS menu bar, the Windows notification area and a Linux panel through AppIndicator.
//!
//! it reads the same ledger as the CLI (`FOOTPRINTS_HOME` or the platform default) and undoes
//! through the same library call, so the conflict check is exactly the one `footprints undo`
//! runs. There is no force option here: a conflict is shown and the file is left alone.

// a release build on Windows would otherwise open a console window beside the app
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rows;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

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
        // a window minimised on Windows or Linux stays minimised after show() alone
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        // WebView2 does not always fire visibilitychange for a hidden window, so the page is
        // told to reload its list directly as well
        let _ = w.emit("shown", ());
    }
}

/// the tray image for this platform. macOS gets a template image that the system recolours to
/// match the menu bar. The Windows notification area and Linux panels draw an image as it is,
/// on a light or a dark bar depending on the theme, so they get a colour mark with a light halo
/// that reads on both.
fn tray_icon() -> tauri::Result<tauri::image::Image<'static>> {
    // the 32 point @2x template: macOS scales the menu bar image down to fit, so the larger
    // source stays sharp on a Retina display
    #[cfg(target_os = "macos")]
    let bytes: &[u8] = include_bytes!("../icons/tray/FootprintsTemplate-32@2x.png");
    // Windows asks for a 16 to 32 pixel icon, so a 32 pixel source only ever scales down
    #[cfg(windows)]
    let bytes: &[u8] = include_bytes!("../icons/tray/Footprints-colour-32.png");
    // AppIndicator panels run from 22 pixels to twice that on a high density screen
    #[cfg(not(any(target_os = "macos", windows)))]
    let bytes: &[u8] = include_bytes!("../icons/tray/Footprints-colour-64.png");
    tauri::image::Image::from_bytes(bytes)
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
            TrayIconBuilder::with_id("footprints")
                .icon(tray_icon()?)
                // a template image follows the menu bar's light or dark appearance. Only macOS
                // reads this flag
                .icon_as_template(cfg!(target_os = "macos"))
                // Narrator reads the tooltip as the icon's name on Windows. AppIndicator has no
                // tooltip, so on Linux the menu items are what a screen reader announces
                .tooltip("Footprints")
                .menu(&menu)
                // a left click opens the window and a right click the menu. AppIndicator sends
                // no click events, so on Linux every click opens the menu, whose first item
                // shows the window
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
            // closing the window only hides it; the app lives in the menu bar or the tray
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("the Footprints app failed to start");
}
