use anyhow::Context;
use auto_launch::AutoLaunchBuilder;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use crate::config::Config;
use crate::utils::LogIfErr;
use crate::{APP_STATE, destroy_borders, reload_borders};

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

    let exe_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|str| str.to_owned()))
        .context("failed to get tackey-borders.exe path")?;
    let auto = AutoLaunchBuilder::new()
        .set_app_name("tacky-borders")
        .set_app_path(&exe_path)
        .build()?;
    let auto_enabled = auto.is_enabled().is_ok_and(|e| e);

    let tray_menu = Menu::new();
    tray_menu.append_items(&[
        &MenuItem::with_id("0", "Show Config", true, None),
        &CheckMenuItem::with_id("1", "Auto Start", true, auto_enabled, None),
        &MenuItem::with_id("2", "Reload", true, None),
        &MenuItem::with_id("3", "Close", true, None),
    ])?;

    let tooltip = format!("{}{}", "tacky-borders v", env!("CARGO_PKG_VERSION"));

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
        // Auto Start
        "1" => match auto.is_enabled() {
            Ok(is_enabled) => {
                let toggle_auto_start = || {
                    if is_enabled {
                        auto.disable()
                    } else {
                        auto.enable()
                    }
                };

                if let Err(err) = toggle_auto_start() {
                    error!("{err}")
                }
            }
            Err(err) => error!("{err}"),
        },
        // Reload
        "2" => {
            Config::reload();
            reload_borders();
        }
        // Close
        "3" => {
            destroy_borders();

            // Convert hwineventhook_isize back into HWINEVENTHOOK, and unhook it
            let hwineventhook = HWINEVENTHOOK(hwineventhook_isize as _);
            unsafe { UnhookWinEvent(hwineventhook) }
                .ok()
                .context("could not unhook win event")
                .log_if_err();

            // Set to None to call their Drop impls
            *APP_STATE.config_watcher.lock().unwrap() = None;
            *APP_STATE.komorebi_integration.lock().unwrap() = None;
            *APP_STATE.display_adapters_watcher.lock().unwrap() = None;

            unsafe { PostQuitMessage(0) };
        }
        _ => {}
    }));

    tray_icon.map_err(anyhow::Error::new)
}
