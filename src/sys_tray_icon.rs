use tray_icon::{TrayIconBuilder, TrayIconEvent, menu::Menu, menu::MenuEvent, menu::MenuId, menu::MenuItem, Icon, TrayIcon};
use windows::Win32::Foundation::WPARAM;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

pub fn create_tray_icon(main_thread: u32) -> Result<TrayIcon, tray_icon::Error> {
    let tray_menu = Menu::new();
    tray_menu.append(&MenuItem::with_id("0", "Close", true, None));

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("tacky-borders")
        .build();

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.0.as_str() == "0" {
            let result = unsafe { PostThreadMessageW(main_thread, WM_CLOSE, WPARAM(0), LPARAM(0)) };
            println!("Sending WM_CLOSE to main thread: {:?}", result);
        }
    }));

    return tray_icon;
}
