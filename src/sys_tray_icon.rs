use anyhow::Context;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::Accessibility::{UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use crate::config::Config;
use crate::{reload_borders, APP_STATE};

pub fn create_tray_icon(hwineventhook: HWINEVENTHOOK) -> anyhow::Result<TrayIcon> {
    let icon = match Icon::from_resource(1, Some((64, 64))) {
        Ok(icon) => icon,
        Err(e) => {
            error!("could not retrieve icon from tacky-borders.exe for tray menu: {e}");

            // If we could not retrieve an icon from the exe, then try to create an empty icon. If
            // even that fails, just return an Error using '?'.
            let rgba: Vec<u8> = vec![0, 0, 0, 0];
            Icon::from_rgba(rgba, 1, 1).context("could not create empty tray icon")?
        }
    };

    // Include the application name and version number in the tray icon tooltip
    let tooltip = format!("{}{}", "tacky-borders v", env!("CARGO_PKG_VERSION"));

    let tray_menu = Menu::new();
    tray_menu.append_items(&[
        &MenuItem::with_id("0", "Show Config", true, None),
        &MenuItem::with_id("1", "Reload", true, None),
        &MenuItem::with_id("2", "Close", true, None),
    ])?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip(tooltip)
        .with_icon(icon)
        .build();

    // Convert HWINEVENTHOOK to isize so we can move it into the thread below
    let hwineventhook_isize = hwineventhook.0 as isize;

    // Handle tray icon events (i.e. clicking on the menu items)
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        // Show Config
        "0" => {
            match Config::get_dir() {
                Ok(dir) => {
                    // I don't really think I need to check this Result from open::that() because
                    // it's pretty obvious to the user if they can't open the directory
                    let _ = open::that(dir);
                }
                Err(e) => error!("{e}"),
            }
        }
        // Reload
        "1" => {
            Config::reload();
            reload_borders();
        }
        // Close
        "2" => unsafe {
            // Convert hwineventhook_isize back into HWINEVENTHOOK
            let hwineventhook = HWINEVENTHOOK(hwineventhook_isize as _);

            let unhook_bool = UnhookWinEvent(hwineventhook).as_bool();
            let stop_res = APP_STATE.config_watcher.lock().unwrap().stop();
            let close_res = APP_STATE.komorebi_integration.lock().unwrap().stop();

            if unhook_bool && stop_res.is_ok() && close_res.is_ok() {
                PostQuitMessage(0);
            } else {
                error!("attempt to unhook win event: {unhook_bool:?}; attempt to stop config watcher: {stop_res:?}; attempt to close socket: {close_res:?}");
            }
        },
        _ => {}
    }));

    tray_icon.map_err(anyhow::Error::new)
}
