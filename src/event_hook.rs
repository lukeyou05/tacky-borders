use windows::{
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::Graphics::Dwm::*,
};

use crate::window_border::WindowBorder;
use crate::BORDERS;
use crate::set_event_hook;
use crate::spawn_border_thread;
use crate::destroy_border_thread;
use crate::SendHWND;

pub extern "system" fn handle_win_event_main(
    h_win_event_hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    dw_event_thread: u32,
    dwms_event_time: u32,
) {
    if id_object == OBJID_CURSOR.0 {
        return;
    }

    match event {
        EVENT_OBJECT_LOCATIONCHANGE => {
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
            let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

            if style & WS_CHILD.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0  {
                return;
            }

            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                unsafe { SendMessageW(border_window, WM_MOVE, WPARAM(0), LPARAM(0)); }
            }
            drop(borders);
        },
        EVENT_OBJECT_FOCUS => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
           
            // I have to loop through because for whatever reason, EVENT_OBJECT_REORDER only gets
            // sent with some random memory address that might be important but idk.
            for value in borders.values() {
                let border_window: HWND = HWND(*value as _);
                if unsafe { IsWindowVisible(border_window).as_bool() } {
                    unsafe { PostMessageW(border_window, WM_SETFOCUS, WPARAM(0), LPARAM(0)) };
                }
            }
            drop(borders);
        },
        EVENT_OBJECT_SHOW => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                drop(borders);
                unsafe { PostMessageW(border_window, WM_SHOWWINDOW, WPARAM(0), LPARAM(0)); }
            } else {
                // Check if the window is a tool window or popup
                let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
                let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
                
                // When a popup window is created, it can mess up the z-order of the border so we
                // reset it here. I also wait a milisecond for the popup window to set its
                // position. THIS IS SO TACKY LMAO. TODO
                if style & WS_POPUP.0 != 0 {
                    //println!("popup window created!");

                    drop(borders);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    let borders = mutex.lock().unwrap();

                    for value in borders.values() {
                        let border_window: HWND = HWND(*value as _);
                        if unsafe { IsWindowVisible(border_window).as_bool() } {
                            unsafe { PostMessageW(border_window, WM_SETFOCUS, WPARAM(0), LPARAM(0)) };
                        }
                    }
                    drop(borders);
                }

                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    hwnd, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() {
                    return;
                }

                if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
                    //println!("creating window border for: {:?}", hwnd);
                    spawn_border_thread(hwnd, 350);
                }
            }
        },
        EVENT_OBJECT_HIDE => {
            let mutex = unsafe { &*BORDERS };
            let window = SendHWND(hwnd);

            let thread = std::thread::spawn(move || {
                let window_sent = window;
                let borders = mutex.lock().unwrap();
                let window_isize = window_sent.0.0 as isize;
                let border_option = borders.get(&window_isize);

                // I have to check IsWindowVisible because for whatever reason, EVENT_OBJECT_HIDE
                // can be sent even if the window is still visible (it does this for Vesktop)
                if border_option.is_some() && unsafe { !IsWindowVisible(window_sent.0).as_bool() } {
                    let border_window: HWND = HWND(*border_option.unwrap() as _);
                    drop(borders);
                    unsafe { SendMessageW(border_window, WM_CLOSE, WPARAM(0), LPARAM(0)); }
                } else {
                    drop(borders);
                }
            });
        },
        EVENT_OBJECT_UNCLOAKED => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                drop(borders);
                unsafe { PostMessageW(border_window, WM_SHOWWINDOW, WPARAM(0), LPARAM(0)); }
            } else {
                // Check if the window is a tool window or popup
                let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
                let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    hwnd, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() {
                    drop(borders);
                    return;
                }

                if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
                    //println!("creating window border for: {:?}", hwnd);
                    spawn_border_thread(hwnd, 0);
                }
                drop(borders);
            }
        },
        EVENT_OBJECT_CLOAKED => {
            let mutex = unsafe { &*BORDERS };
            let window = SendHWND(hwnd);

            let thread = std::thread::spawn(move || {
                let window_sent = window;
                let borders = mutex.lock().unwrap();
                let window_isize = window_sent.0.0 as isize;
                let border_option = borders.get(&window_isize);

                if border_option.is_some() {
                    let border_window: HWND = HWND(*border_option.unwrap() as _);
                    drop(borders);
                    unsafe { SendMessageW(border_window, WM_CLOSE, WPARAM(0), LPARAM(0)); }
                } else {
                    drop(borders);
                }
            });
        },
        EVENT_SYSTEM_MINIMIZEEND => {
            let mutex = unsafe { &*BORDERS };
            let borders = mutex.lock().unwrap();
            let hwnd_isize = hwnd.0 as isize;
            let border_option = borders.get(&hwnd_isize);

            if border_option.is_some() {
                let border_window: HWND = HWND(*border_option.unwrap() as _);
                drop(borders);
                unsafe { SendMessageW(border_window, WM_QUERYOPEN, WPARAM(0), LPARAM(0)); }
            } else {
                drop(borders);
            }
        },
        EVENT_OBJECT_DESTROY => {
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
            let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

            if style & WS_CHILD.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0  {
                return;
            }

            //println!("destroying");
            if unsafe { !IsWindowVisible(hwnd).as_bool() } {
                destroy_border_thread(hwnd);
                //println!("destroyed a border");

                // Use below to debug whether window borders are properly destroyed
                /*let mutex = unsafe { &*BORDERS };
                let borders = mutex.lock().unwrap();
                println!("borders after destroying window: {:?}", borders);
                drop(borders);*/
            }
        },
        _ => {}
    }
}
