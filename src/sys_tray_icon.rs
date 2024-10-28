use tray_icon::{TrayIconBuilder, TrayIconEvent, menu::Menu, menu::MenuEvent, menu::MenuId, menu::MenuItem, Icon, TrayIcon};
use image::ImageReader;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
use dirs::home_dir;


pub fn create_tray_icon(main_thread: u32) -> Result<TrayIcon, tray_icon::Error> {
    let home_dir = home_dir().expect("can't find home path");
    let image_path = home_dir.join(".config").join("tacky-borders").join("icon.png");

    let image = match ImageReader::open(&image_path) {
        Ok(reader) => match reader.decode() {
            Ok(img) => img,
            Err(_) => panic!("can't decode image: {}", image_path.display()),
        },
        Err(_) => panic!("can't open image: {}", image_path.display()),
    };

    let image_bytes = image.into_bytes();
    let icon = Icon::from_rgba(image_bytes, 32, 32).expect(&format!("could not convert icon.png into Icon Icon: {}", image_path.display()));

    let tray_menu = Menu::new();
    tray_menu.append(&MenuItem::with_id("0", "Close", true, None));

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("tacky-borders")
        .with_icon(icon)
        .build();

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.0.as_str() == "0" {
            let result = unsafe { PostThreadMessageW(main_thread, WM_CLOSE, WPARAM(0), LPARAM(0)) };
            println!("Sending WM_CLOSE to main thread: {:?}", result);
        }
    }));

    return tray_icon;
}
