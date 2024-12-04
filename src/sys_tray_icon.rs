use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::System::Threading::ExitProcess;
use windows::Win32::UI::Accessibility::UnhookWinEvent;

use crate::border_config::Config;
use crate::{reload_borders, EVENT_HOOK};

pub fn create_tray_icon() -> Result<TrayIcon, tray_icon::Error> {
    let icon = match Icon::from_resource(1, Some((64, 64))) {
        Ok(icon) => icon,
        Err(_) => {
            error!("Could not retrieve the icon from tacky-borders.exe for the tray menu!");

            // If we could not retrieve an icon from the exe, then create an empty icon
            let rgba: Vec<u8> = vec![0, 0, 0, 0];
            let Ok(empty_icon) = Icon::from_rgba(rgba, 1, 1) else {
                // If we could not even create an empty icon, just return an Error
                return Err(tray_icon::Error::OsError(std::io::Error::other(
                    "Could not create tray icon for tray menu",
                )));
            };

            empty_icon
        }
    };

    // Include the application name and version number in the tray icon tooltip
    let tooltip = format!("{}{}", "tacky-borders v", env!("CARGO_PKG_VERSION"));

    let tray_menu = Menu::new();
    if tray_menu
        .append_items(&[
            &MenuItem::with_id("0", "Show Config", true, None),
            &MenuItem::with_id("1", "Reload", true, None),
            &MenuItem::with_id("2", "Close", true, None),
        ])
        .is_err()
    {
        // TODO add better error handling here
        error!("Could not create tray menu items!");
    };

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip(tooltip)
        .with_icon(icon)
        .build();

    // Handle tray icon events (i.e. clicking on the menu items)
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        // Show Config
        "0" => {
            match Config::get_config_dir() {
                Ok(dir) => {
                    // I don't really think I need to check this Result from open::that() because
                    // it's pretty obvious to the user if they can't open the directory
                    let _ = open::that(dir);
                }
                Err(err) => error!("Error: {}", err),
            }
        }
        // Reload
        "1" => {
            Config::reload_config();
            reload_borders();
        }
        // Close
        "2" => unsafe {
            if UnhookWinEvent(EVENT_HOOK.get()).as_bool() {
                debug!("Exiting tacky-borders!");
                ExitProcess(0);
            } else {
                error!("Could not unhook win event hook");
            }
        },
        _ => {}
    }));

    tray_icon
}
