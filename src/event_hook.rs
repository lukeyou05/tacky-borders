use anyhow::Context;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_FOCUS, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GA_ROOT, OBJID_CLIENT, OBJID_CURSOR,
    OBJID_WINDOW,
};

use crate::BORDERS;
use crate::{log_if_err, utils::*};

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

            if let Some(border) = get_border_from_window(_hwnd) {
                log_if_err!(send_notify_message_w(
                    border,
                    WM_APP_LOCATIONCHANGE,
                    WPARAM(0),
                    LPARAM(0)
                )
                .context("EVENT_OBJECT_LOCATIONCHANGE"));
            }
        }
        EVENT_OBJECT_REORDER => {
            if has_filtered_style(_hwnd) {
                return;
            }

            let borders = BORDERS.lock().unwrap();

            // Send reoder messages to all the border windows
            for value in borders.values() {
                let border_window: HWND = HWND(*value as _);
                if is_window_visible(border_window) {
                    log_if_err!(post_message_w(
                        border_window,
                        WM_APP_REORDER,
                        WPARAM(0),
                        LPARAM(0)
                    )
                    .context("EVENT_OBJECT_REORDER"));
                }
            }

            drop(borders);
        }
        EVENT_OBJECT_FOCUS => {
            // This event can send a child window for its hwnd, so we have to find its parent
            let parent = unsafe { GetAncestor(_hwnd, GA_ROOT) };

            if has_filtered_style(parent) {
                return;
            }

            // Send focus messages to all the border windows
            for (key, val) in BORDERS.lock().unwrap().iter() {
                let border_window: HWND = HWND(*val as _);
                // Some apps like Flow Launcher can become focused even if they aren't visible yet,
                // so I also need to check if 'key' is equal to 'parent' (the focused window)
                if is_window_visible(border_window) || key == &(parent.0 as isize) {
                    log_if_err!(
                        post_message_w(border_window, WM_APP_FOCUS, WPARAM(0), LPARAM(0))
                            .context("EVENT_OBJECT_FOCUS")
                    );
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
            if let Some(border) = get_border_from_window(_hwnd) {
                log_if_err!(
                    post_message_w(border, WM_APP_MINIMIZESTART, WPARAM(0), LPARAM(0))
                        .context("EVENT_SYSTEM_MINIMIZESTART")
                );
            }
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            if let Some(border) = get_border_from_window(_hwnd) {
                log_if_err!(
                    post_message_w(border, WM_APP_MINIMIZEEND, WPARAM(0), LPARAM(0))
                        .context("EVENT_SYSTEM_MINIMIZEEND")
                );
            }
        }
        EVENT_OBJECT_DESTROY => {
            if (_id_object == OBJID_WINDOW.0 || _id_object == OBJID_CLIENT.0)
                && !has_filtered_style(_hwnd)
            {
                destroy_border_for_window(_hwnd);
            }
        }
        _ => {}
    }
}
