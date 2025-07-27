use anyhow::Context;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use crate::config::Config;
use crate::utils::LogIfErr;
use crate::{APP_STATE, reload_borders};

pub fn create_tray_icon(hwineventhook: HWINEVENTHOOK) -> anyhow::Result<TrayIcon> {
    let icon = match Icon::from_resource(1, Some((64, 64))) {
        Ok(icon) => icon,
        Err(err) => {
            error!("could not retrieve icon from tacky-borders.exe for tray menu: {err}");

            // If we could not retrieve an icon from the exe, then try to create an empty icon. If
            // even that fails, then we'll just return an Error.
            let rgba: Vec<u8> = vec![0, 0, 0, 0];
            Icon::from_rgba(rgba, 1, 1).context("could not create empty tray icon")?
        }
    };

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

    // Convert HWINEVENTHOOK to isize so we can move it into the event handler below
    let hwineventhook_isize = hwineventhook.0 as isize;

    // Handle tray icon events (i.e. clicking on the menu items)
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        // Show Config
        "0" => match Config::get_dir() {
            Ok(dir) => {
                open::that(dir).log_if_err();
            }
            Err(err) => error!("{err}"),
        },
        // Reload
        "1" => {
            Config::reload();
            reload_borders();
        }
        // Close
        "2" => {
            // Convert hwineventhook_isize back into HWINEVENTHOOK
            let hwineventhook = HWINEVENTHOOK(hwineventhook_isize as _);

            let event_unhook_res = unsafe { UnhookWinEvent(hwineventhook) }.ok();
            // NOTE: It's important to set these to None to ensure the Drop impl is called
            *APP_STATE.config_watcher.lock().unwrap() = None;
            *APP_STATE.komorebi_integration.lock().unwrap() = None;
            *APP_STATE.display_adapters_watcher.lock().unwrap() = None;

            if event_unhook_res.is_ok() {
                unsafe { PostQuitMessage(0) };
            } else {
                // TODO: Update this branch to reflect the fact that there is only one Result left
                // since RAII requires cleanup in the Drop impl (which cannot return a result).
                let results = [format!("attempt to unhook win event: {event_unhook_res:?}")];
                // TODO: Display an error box as well
                error!(
                    "one or more errors encountered when cleaning up resources upon application exit: \n{}",
                    results.join("\n")
                );
            }
        }
        _ => {}
    }));

    tray_icon.map_err(anyhow::Error::new)
}
