use dirs::home_dir;
use tray_icon::{menu::Menu, menu::MenuEvent, menu::MenuItem, Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::System::Threading::ExitProcess;
use windows::Win32::UI::Accessibility::UnhookWinEvent;

use crate::border_config::Config;
use crate::reload_borders;
use crate::EVENT_HOOK;

pub fn create_tray_icon() -> Result<TrayIcon, tray_icon::Error> {
    let icon = match Icon::from_resource(1, Some((64, 64))) {
        Ok(icon) => icon,
        Err(_) => {
            error!("Could not retrieve tray icon!");
            std::process::exit(1)
        }
    };

    let tray_menu = Menu::new();
    let _ = tray_menu.append(&MenuItem::with_id("0", "Show Config", true, None));
    let _ = tray_menu.append(&MenuItem::with_id("1", "Reload", true, None));
    let _ = tray_menu.append(&MenuItem::with_id("2", "Close", true, None));

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("tacky-borders")
        .with_icon(icon)
        .build();

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        "0" => {
            let home_dir = home_dir().expect("can't find home path");
            let config_dir = home_dir.join(".config").join("tacky-borders");
            let _ = open::that(config_dir);
        }
        "1" => {
            Config::reload_config();
            reload_borders();
        }
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
