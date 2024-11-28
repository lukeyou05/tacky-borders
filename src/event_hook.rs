use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, PostMessageW, SendNotifyMessageW, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER,
    EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, OBJID_CLIENT, OBJID_CURSOR, OBJID_WINDOW,
};

use crate::utils::*;
use crate::BORDERS;

pub extern "system" fn handle_win_event(
    _h_win_event_hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _dw_event_thread: u32,
    _dwms_event_time: u32,
) {
    if _id_object == OBJID_CURSOR.0 {
        return;
    }

    match _event {
        EVENT_OBJECT_LOCATIONCHANGE => {
            if has_filtered_style(_hwnd) {
                return;
            }

            let border_window = get_border_from_window(_hwnd);
            if let Some(hwnd) = border_window {
                unsafe {
                    let _ = SendNotifyMessageW(hwnd, WM_APP_LOCATIONCHANGE, WPARAM(0), LPARAM(0));
                }
            }
        }
        EVENT_OBJECT_REORDER => {
            if has_filtered_style(_hwnd) {
                return;
            }

            let borders = BORDERS.lock().unwrap();

            for value in borders.values() {
                let border_window: HWND = HWND(*value as _);
                if is_window_visible(border_window) {
                    unsafe {
                        let _ = PostMessageW(border_window, WM_APP_REORDER, WPARAM(0), LPARAM(0));
                    }
                }
            }
            drop(borders);
        }
        EVENT_OBJECT_FOCUS => {
            // TODO not sure if I should use GA_ROOT or GA_ROOTOWNER
            let parent = unsafe { GetAncestor(_hwnd, GA_ROOT) };

            if has_filtered_style(parent) {
                return;
            }

            for val in BORDERS.lock().unwrap().values() {
                let border_window: HWND = HWND(*val as _);
                if is_window_visible(border_window) {
                    unsafe {
                        let _ = PostMessageW(border_window, WM_APP_FOCUS, WPARAM(0), LPARAM(0));
                    }
                }
            }
        }
        EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => {
            if _id_object == OBJID_WINDOW.0 {
                show_border_for_window(_hwnd);
            }
        }
        EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED => {
            if _id_object == OBJID_WINDOW.0 {
                hide_border_for_window(_hwnd);
            }
        }
        EVENT_SYSTEM_MINIMIZESTART => {
            let border_option = get_border_from_window(_hwnd);
            if let Some(border_window) = border_option {
                unsafe {
                    let _ = PostMessageW(border_window, WM_APP_MINIMIZESTART, WPARAM(0), LPARAM(0));
                }
            }
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            let border_option = get_border_from_window(_hwnd);
            if let Some(border_window) = border_option {
                unsafe {
                    let _ = PostMessageW(border_window, WM_APP_MINIMIZEEND, WPARAM(0), LPARAM(0));
                }
            }
        }
        EVENT_OBJECT_DESTROY => {
            if (_id_object == OBJID_WINDOW.0 || _id_object == OBJID_CLIENT.0)
                && !has_filtered_style(_hwnd)
            {
                let _ = destroy_border_for_window(_hwnd);
            }
        }
        _ => {}
    }
}
