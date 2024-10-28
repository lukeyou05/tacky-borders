// TODO remove allow unused and fix all the warnings generated
#![allow(unused)]
// This hides the console when running the app. Comment it out to debug.
//#![windows_subsystem = "windows"]

use std::sync::{Arc, Mutex, LazyLock};
use std::collections::HashMap;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::System::Threading::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::Common::*,
};

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

mod window_border;
mod event_hook;
mod sys_tray_icon;
mod border_config;

pub static mut BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
pub static CONFIG: LazyLock<border_config::Config> = LazyLock::new(|| border_config::create_config());

// This shit supposedly unsafe af but it works so idgaf. 
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    register_window_class();
    println!("window class is registered!");
    enum_windows();

    let main_thread = unsafe { GetCurrentThreadId() };
    let tray_icon_option = sys_tray_icon::create_tray_icon(main_thread);
    if tray_icon_option.is_err() {
        println!("Error creating tray icon!");
    }

    let win_event_hook = set_event_hook();
    unsafe {
        println!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            if message.message == WM_CLOSE {
                let result = UnhookWinEvent(win_event_hook);
                if result.as_bool() {
                    ExitProcess(0);
                } else {
                    println!("Error. Could not unhook win event hook");
                }
            }

            TranslateMessage(&message);
            DispatchMessageW(&message);
            std::thread::sleep(std::time::Duration::from_millis(16))
        }
        println!("MESSSAGE LOOP IN MAIN.RS EXITED. THIS SHOULD NOT HAPPEN");
    }
}

pub fn register_window_class() -> Result<()> {
    unsafe {
        let window_class = w!("tacky-border");
        let hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);

        let mut wcex = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_border::WindowBorder::s_wnd_proc),
            hInstance: hinstance,
            lpszClassName: window_class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        let result = RegisterClassExW(&wcex);
            
        if result == 0 {
            let last_error = GetLastError();
            println!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
        }
    }

    return Ok(());
}

pub fn set_event_hook() -> HWINEVENTHOOK {
    unsafe {
        return SetWinEventHook(
            EVENT_MIN,
            EVENT_MAX,
            None,
            Some(event_hook::handle_win_event_main),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
    }
}

pub fn enum_windows() {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut windows as *mut _ as isize),
        );
    }
    println!("Windows have been enumerated!");
    println!("Windows: {:?}", windows);

    for hwnd in windows {
        spawn_border_thread(hwnd, 0);
    }
}

pub fn spawn_border_thread(tracking_window: HWND, delay: u64) -> Result<()> {
    let borders_mutex = unsafe { &*BORDERS };
    let config = unsafe { &*CONFIG };
    let window = SendHWND(tracking_window);

    let thread = std::thread::spawn(move || {
        let window_sent = window;

        std::thread::sleep(std::time::Duration::from_millis(delay));
        if unsafe { !IsWindowVisible(window_sent.0).as_bool() } {
            return;
        }

        let red = ((config.active_color & 0x00FF0000) >> 16) as f32/255.0;
        let green = ((config.active_color & 0x0000FF00) >> 8) as f32/255.0;
        let blue = ((config.active_color & 0x000000FF) >> 0) as f32/255.0;
        let avg = (red + green + blue)/3.0;

        let active_color = D2D1_COLOR_F {
            r: red,
            g: green,
            b: blue,
            a: 1.0
        };

        let red = ((config.inactive_color & 0x00FF0000) >> 16) as f32/255.0;
        let green = ((config.inactive_color & 0x0000FF00) >> 8) as f32/255.0;
        let blue = ((config.inactive_color & 0x000000FF) >> 0) as f32/255.0;
        let avg = (red + green + blue)/3.0;

        let inactive_color = D2D1_COLOR_F {
            r: red,
            g: green,
            b: blue,
            a: 1.0
        };

        let mut border = window_border::WindowBorder { 
            tracking_window: window_sent.0, 
            border_size: config.border_size, 
            border_offset: config.border_offset,
            force_border_radius: config.border_radius,
            active_color: active_color,
            inactive_color: inactive_color,
            ..Default::default()
        };

        let mut borders_hashmap = borders_mutex.lock().unwrap();
        let window_isize = window_sent.0.0 as isize; 

        // Check to see if the key already exists in the hashmap. If not, then continue
        // adding the key and initializing the border. This is important because sometimes, the
        // event_hook function will call spawn_border_thread multiple times for the same window. 
        if borders_hashmap.contains_key(&window_isize) {
            //println!("Duplicate window: {:?}", borders_hashmap);
            drop(borders_hashmap);
            return;
        }

        let hinstance: HINSTANCE = unsafe { std::mem::transmute(&__ImageBase) };
        border.create_border_window(hinstance);
        borders_hashmap.insert(window_isize, border.border_window.0 as isize);
        drop(borders_hashmap);
        
        border.init(hinstance);
    });

    return Ok(());
}

pub fn destroy_border_thread(tracking_window: HWND) -> Result<()> {
    let mutex = unsafe { &*BORDERS };
    let window = SendHWND(tracking_window);

    let thread = std::thread::spawn(move || {
        let window_sent = window;
        let mut borders_hashmap = mutex.lock().unwrap();
        let window_isize = window_sent.0.0 as isize;
        let border_option = borders_hashmap.get(&window_isize);
        
        if border_option.is_some() {
            let border_window: HWND = HWND((*border_option.unwrap()) as *mut _);
            unsafe { SendMessageW(border_window, WM_DESTROY, WPARAM(0), LPARAM(0)) };
            borders_hashmap.remove(&window_isize);
        }

        drop(borders_hashmap);
    });

    return Ok(());
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd).as_bool() {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let mut is_cloaked = FALSE;
        let result = unsafe { DwmGetWindowAttribute(
            hwnd, 
            DWMWA_CLOAKED,
            std::ptr::addr_of_mut!(is_cloaked) as *mut _,
            size_of::<BOOL>() as u32
        ) };
        if result.is_err() {
            return FALSE;
        }

        // Exclude certain window styles
        if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
            let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam.0);
            println!("visible_windows: {:?}", visible_windows);
            visible_windows.push(hwnd);
        }
    }
    TRUE 
}
