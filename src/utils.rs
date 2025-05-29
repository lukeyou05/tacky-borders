use anyhow::{Context, anyhow};
use regex::Regex;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::{ptr, thread};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ENVVAR_NOT_FOUND, ERROR_INVALID_WINDOW_HANDLE, ERROR_SUCCESS, FALSE,
    GetLastError, HWND, LPARAM, LRESULT, RECT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE,
    DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Diagnostics::Debug::FACILITY_ITF;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, GetDpiForMonitor, MONITOR_DPI_TYPE, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::Ime::ImmDisableIME;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
    RealGetWindowClassW, SendMessageW, SendNotifyMessageW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_NCDESTROY, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_MAXIMIZE,
};
use windows::core::{BOOL, HRESULT, PWSTR};

use crate::APP_STATE;
use crate::config::{EnableMode, MatchKind, MatchStrategy, WindowRule};
use crate::window_border::WindowBorder;

pub const WM_APP_LOCATIONCHANGE: u32 = WM_APP;
pub const WM_APP_REORDER: u32 = WM_APP + 1;
pub const WM_APP_FOREGROUND: u32 = WM_APP + 2;
pub const WM_APP_SHOWUNCLOAKED: u32 = WM_APP + 3;
pub const WM_APP_HIDECLOAKED: u32 = WM_APP + 4;
pub const WM_APP_MINIMIZESTART: u32 = WM_APP + 5;
pub const WM_APP_MINIMIZEEND: u32 = WM_APP + 6;
pub const WM_APP_ANIMATE: u32 = WM_APP + 7;
pub const WM_APP_KOMOREBI: u32 = WM_APP + 8;

// Custom HRESULT error code indicating an uninitialized COM object within this application.
// T_E_UNINIT typically represents an Option::None where an Option::Some(_) was expected. This is
// used instead of something like E_POINTER to prevent overlap with Windows COM interface errors.
// The code 0x2222 is completely arbitrary, but is within Microsoft's recommended range for custom
// FACILITY_ITF HRESULTs (0x0200 to 0xFFFF).
pub const T_E_UNINIT: HRESULT = HRESULT((1 << 31) | ((FACILITY_ITF.0 as i32) << 16) | (0x2222));

pub trait LogIfErr {
    fn log_if_err(&self);
}

impl<T> LogIfErr for Result<(), T>
where
    T: std::fmt::Display,
{
    fn log_if_err(&self) {
        if let Err(err) = self {
            error!("{err:#}");
        }
    }
}

pub trait ToWindowsResult<T> {
    fn to_windows_result(self, hresult: HRESULT) -> windows::core::Result<T>;
}

impl<T> ToWindowsResult<T> for anyhow::Result<T> {
    fn to_windows_result(self, hresult: HRESULT) -> windows::core::Result<T> {
        self.map_err(|err| windows::core::Error::new(hresult, err.to_string()))
    }
}

pub fn get_window_style(hwnd: HWND) -> WINDOW_STYLE {
    unsafe { WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32) }
}

pub fn get_window_ex_style(hwnd: HWND) -> WINDOW_EX_STYLE {
    unsafe { WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) }
}

pub fn is_window_top_level(hwnd: HWND) -> bool {
    let style = get_window_style(hwnd);

    !style.contains(WS_CHILD)
}

pub fn has_filtered_style(hwnd: HWND) -> bool {
    let ex_style = get_window_ex_style(hwnd);

    ex_style.contains(WS_EX_TOOLWINDOW) || ex_style.contains(WS_EX_NOACTIVATE)
}

pub fn get_window_title(hwnd: HWND) -> anyhow::Result<String> {
    let mut title_buf: [u16; 256] = [0; 256];

    if unsafe { GetWindowTextW(hwnd, &mut title_buf) } == 0 {
        let last_error = get_last_error();

        // ERROR_ENVVAR_NOT_FOUND just means the title is empty which isn't necessarily an issue
        // TODO: figure out whats with the invalid window handles
        if !matches!(
            last_error,
            ERROR_ENVVAR_NOT_FOUND | ERROR_SUCCESS | ERROR_INVALID_WINDOW_HANDLE
        ) {
            // We manually reset LastError here because it doesn't seem to reset by itself
            unsafe { SetLastError(ERROR_SUCCESS) };
            return Err(anyhow!("{last_error:?}"));
        }
    }

    let title_binding = String::from_utf16_lossy(&title_buf);
    Ok(title_binding.split_once("\0").unwrap().0.to_string())
}

pub fn get_window_class(hwnd: HWND) -> anyhow::Result<String> {
    let mut class_buf: [u16; 256] = [0; 256];

    if unsafe { RealGetWindowClassW(hwnd, &mut class_buf) } == 0 {
        let last_error = get_last_error();

        // ERROR_ENVVAR_NOT_FOUND just means the title is empty which isn't necessarily an issue
        // TODO: figure out whats with the invalid window handles
        if !matches!(
            last_error,
            ERROR_ENVVAR_NOT_FOUND | ERROR_SUCCESS | ERROR_INVALID_WINDOW_HANDLE
        ) {
            // We manually reset LastError here because it doesn't seem to reset by itself
            unsafe { SetLastError(ERROR_SUCCESS) };
            return Err(anyhow!("{last_error:?}"));
        }
    }

    let class_binding = String::from_utf16_lossy(&class_buf);
    Ok(class_binding.split_once("\0").unwrap().0.to_string())
}

pub fn get_window_process_name(hwnd: HWND) -> anyhow::Result<String> {
    let mut process_id = 0;
    // This function returns the thread id. If it's 0, that means we likely passed an invalid hwnd.
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) } == 0 {
        return Err(anyhow!(
            "could not get thread and process id for {hwnd:?}: {:?}",
            get_last_error()
        ));
    }

    let hprocess = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
        .context(format!("could not open process of {hwnd:?}"))?;

    let mut process_buf = [0u16; 256];
    let mut lpdwsize = process_buf.len() as u32;

    let result = unsafe {
        QueryFullProcessImageNameW(
            hprocess,
            PROCESS_NAME_WIN32,
            PWSTR(process_buf.as_mut_ptr()),
            &mut lpdwsize,
        )
    }
    .context(format!("could not query process image name for {hwnd:?}"));

    unsafe { CloseHandle(hprocess) }.context("could not close {hprocess:?}")?;

    result?;

    // QueryFullProcessImageNameW will update lpdwsize with the number of characters written
    // (excluding the null terminating char), so if it's about the same as the size of our buffer,
    // it means we may not have gotten all of it
    if lpdwsize >= process_buf.len() as u32 - 1 {
        warn!("process buffer size too small; truncation may occur");
    }

    let exe_path = PathBuf::from(OsString::from_wide(&process_buf[..lpdwsize as usize]));

    Ok(exe_path
        .file_stem()
        .context("could not get exe file stem from process path")?
        .to_string_lossy()
        .into_owned())
}

// Get the window rule from 'window_rules' in the config
pub fn get_window_rule(hwnd: HWND) -> WindowRule {
    let mut title_opt: Option<String> = None;
    let mut class_opt: Option<String> = None;
    let mut process_opt: Option<String> = None;

    let title = || -> String {
        get_window_title(hwnd).unwrap_or_else(|err| {
            error!("could not retrieve window title for {hwnd:?}: {err}");
            "".to_string()
        })
    };
    let class = || -> String {
        get_window_class(hwnd).unwrap_or_else(|err| {
            error!("could not retrieve window class for {hwnd:?}: {err}");
            "".to_string()
        })
    };
    let process = || -> String {
        get_window_process_name(hwnd).unwrap_or_else(|err| {
            error!("could not retrieve window process name for {hwnd:?}: {err}");
            "".to_string()
        })
    };

    let config = APP_STATE.config.read().unwrap();

    for rule in config.window_rules.iter() {
        let window_name: &String = match rule.kind {
            Some(MatchKind::Title) => title_opt.get_or_insert_with(title),
            Some(MatchKind::Class) => class_opt.get_or_insert_with(class),
            Some(MatchKind::Process) => process_opt.get_or_insert_with(process),
            None => {
                error!("expected 'match' for window rule but None found!");
                continue;
            }
        };

        let Some(match_name) = &rule.name else {
            error!("expected `name` for window rule but None found!");
            continue;
        };

        // Check if the window rule matches the window
        let has_match = match rule.strategy {
            Some(MatchStrategy::Equals) | None => {
                window_name.to_lowercase().eq(&match_name.to_lowercase())
            }
            Some(MatchStrategy::Contains) => window_name
                .to_lowercase()
                .contains(&match_name.to_lowercase()),
            Some(MatchStrategy::Regex) => Regex::new(match_name)
                .unwrap()
                .captures(window_name)
                .is_some(),
        };

        // Return the first match
        if has_match {
            return rule.clone();
        }
    }

    WindowRule::default()
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn is_rect_visible(rect: &RECT) -> bool {
    rect.top >= 0 || rect.left >= 0 || rect.bottom >= 0 || rect.right >= 0
}

pub fn are_rects_same_size(rect1: &RECT, rect2: &RECT) -> bool {
    rect1.right - rect1.left == rect2.right - rect2.left
        && rect1.bottom - rect1.top == rect2.bottom - rect2.top
}

pub fn is_window_cloaked(hwnd: HWND) -> bool {
    let mut is_cloaked = FALSE;
    if let Err(err) = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            ptr::addr_of_mut!(is_cloaked) as _,
            size_of::<BOOL>() as u32,
        )
    } {
        error!("could not check if window is cloaked: {err}");
        return true;
    }
    is_cloaked.as_bool()
}

pub fn get_foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn is_window_minimized(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd).as_bool() }
}

pub fn is_window(hwnd: Option<HWND>) -> bool {
    unsafe { IsWindow(hwnd).as_bool() }
}

pub fn post_message_w(
    hwnd: Option<HWND>,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::core::Result<()> {
    unsafe { PostMessageW(hwnd, msg, wparam, lparam) }
}

pub fn send_message_w(
    hwnd: HWND,
    msg: u32,
    wparam: Option<WPARAM>,
    lparam: Option<LPARAM>,
) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, wparam, lparam) }
}

pub fn send_notify_message_w(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::core::Result<()> {
    unsafe { SendNotifyMessageW(hwnd, msg, wparam, lparam) }
}

pub fn imm_disable_ime(param0: u32) -> BOOL {
    unsafe { ImmDisableIME(param0) }
}

pub fn set_process_dpi_awareness_context(
    value: DPI_AWARENESS_CONTEXT,
) -> windows::core::Result<()> {
    unsafe { SetProcessDpiAwarenessContext(value) }
}

pub fn has_native_border(hwnd: HWND) -> bool {
    let style = get_window_style(hwnd);
    let ex_style = get_window_ex_style(hwnd);

    !style.contains(WS_MAXIMIZE) && ex_style.contains(WS_EX_WINDOWEDGE)
}

pub fn create_border_for_window(tracking_window: HWND, window_rule: WindowRule) {
    debug!("creating border for: {:?}", tracking_window);
    let tracking_window_isize = tracking_window.0 as isize;

    let _ = thread::spawn(move || {
        let tracking_window = HWND(tracking_window_isize as _);

        // Note: 'key' for the hashmap is the tracking window, 'value' is the border window
        let mut borders_hashmap = APP_STATE.borders.lock().unwrap();

        // Check to see if there is already a border for the given tracking window
        if borders_hashmap.contains_key(&tracking_window_isize) {
            return;
        }

        // Otherwise, continue creating the border window
        let mut border = WindowBorder::new(tracking_window);
        let border_window = match border.create_window() {
            Ok(hwnd) => hwnd,
            Err(err) => {
                error!("could not create border window: {err}");
                return;
            }
        };

        borders_hashmap.insert(tracking_window_isize, border_window.0 as isize);
        drop(borders_hashmap);

        // Drop these values (to save some RAM?) before calling init and entering a message loop
        let _ = tracking_window;
        let _ = tracking_window_isize;

        // NOTE: init() contains a message loop
        border.init(window_rule).log_if_err();

        // If init() exits, that means the border has been destroyed, so we should remove it from
        // the hashmap
        APP_STATE
            .borders
            .lock()
            .unwrap()
            .remove(&tracking_window_isize);
    });
}

pub fn get_adjusted_radius(radius: f32, dpi: u32, border_width: i32) -> f32 {
    radius * dpi as f32 / 96.0 + (border_width as f32 / 2.0)
}

pub fn get_window_corner_preference(
    tracking_window: HWND,
) -> anyhow::Result<DWM_WINDOW_CORNER_PREFERENCE> {
    let mut corner_preference = DWM_WINDOW_CORNER_PREFERENCE::default();

    unsafe {
        DwmGetWindowAttribute(
            tracking_window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            ptr::addr_of_mut!(corner_preference) as _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    }
    .context("could not retrieve window corner preference")?;

    Ok(corner_preference)
}

pub fn get_dpi_for_monitor(hmonitor: HMONITOR, dpitype: MONITOR_DPI_TYPE) -> anyhow::Result<u32> {
    let (mut dpi_x, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(hmonitor, dpitype, &mut dpi_x, &mut dpi_y) }?;

    // According to the docs, dpi_x and dpi_y will always be identical, so we only need one
    Ok(dpi_x)
}

pub fn monitor_from_window(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

pub fn hiword(val: usize) -> u16 {
    ((val >> 16) & 0xFFFF) as u16
}

pub fn loword(val: usize) -> u16 {
    (val & 0xFFFF) as u16
}

pub fn get_monitor_info(hmonitor: HMONITOR) -> anyhow::Result<MONITORINFO> {
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    if !unsafe { GetMonitorInfoW(hmonitor, &mut mi) }.as_bool() {
        return Err(anyhow!(
            "could not get monitor info for {:?}: {:?}",
            hmonitor,
            get_last_error()
        ));
    };

    Ok(mi)
}

pub fn get_monitor_resolution(hmonitor: HMONITOR) -> anyhow::Result<(u32, u32)> {
    let m_info = get_monitor_info(hmonitor).context("could not get m_info")?;
    let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
    let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

    Ok((screen_width, screen_height))
}

pub fn destroy_border_for_window(tracking_window: HWND) {
    if let Some(&border_isize) = APP_STATE
        .borders
        .lock()
        .unwrap()
        .get(&(tracking_window.0 as isize))
    {
        let border_window = HWND(border_isize as _);

        post_message_w(Some(border_window), WM_NCDESTROY, WPARAM(0), LPARAM(0))
            .context("destroy_border_for_window")
            .log_if_err();
    }
}

pub fn get_border_for_window(hwnd: HWND) -> Option<HWND> {
    let borders_hashmap = APP_STATE.borders.lock().unwrap();

    let hwnd_isize = hwnd.0 as isize;
    let Some(border_isize) = borders_hashmap.get(&hwnd_isize) else {
        drop(borders_hashmap);
        return None;
    };

    let border_window: HWND = HWND(*border_isize as _);

    Some(border_window)
}

pub fn show_border_for_window(hwnd: HWND) {
    // If the border already exists, simply post a 'SHOW' message to its message queue. Otherwise,
    // create a new border.
    if let Some(border) = get_border_for_window(hwnd) {
        post_message_w(Some(border), WM_APP_SHOWUNCLOAKED, WPARAM(0), LPARAM(0))
            .context("show_border_for_window")
            .log_if_err();
    } else if is_window_top_level(hwnd) && is_window_visible(hwnd) && !is_window_cloaked(hwnd) {
        let window_rule = get_window_rule(hwnd);

        if window_rule.enabled == Some(EnableMode::Bool(false)) {
            info!("border is disabled for {hwnd:?}");
        } else if window_rule.enabled == Some(EnableMode::Bool(true)) || !has_filtered_style(hwnd) {
            create_border_for_window(hwnd, window_rule);
        }
    }
}

pub fn hide_border_for_window(hwnd: HWND) {
    let hwnd_isize = hwnd.0 as isize;

    // Spawn a new thread to guard against re-entrancy in the event hook, though it honestly isn't
    // that important for our purposes I think
    let _ = thread::spawn(move || {
        let hwnd = HWND(hwnd_isize as _);

        if let Some(border) = get_border_for_window(hwnd) {
            post_message_w(Some(border), WM_APP_HIDECLOAKED, WPARAM(0), LPARAM(0))
                .context("hide_border_for_window")
                .log_if_err();
        }
    });
}

pub fn get_last_error() -> WIN32_ERROR {
    unsafe { GetLastError() }
}

// Bezier curve algorithm together with @0xJWLabs
const SUBDIVISION_PRECISION: f32 = 0.0001; // Precision for binary subdivision
const SUBDIVISION_MAX_ITERATIONS: u32 = 10; // Maximum number of iterations for binary subdivision

#[derive(Debug)]
pub enum BezierError {
    InvalidControlPoint,
}

impl std::fmt::Display for BezierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BezierError::InvalidControlPoint => {
                write!(f, "cubic-bezier control points must be in the range [0, 1]")
            }
        }
    }
}
impl std::error::Error for BezierError {}

struct Point {
    x: f32,
    y: f32,
}

fn lerp(t: f32, p1: f32, p2: f32) -> f32 {
    p1 + (p2 - p1) * t
}

// Compute the cubic Bézier curve using De Casteljau's algorithm.
fn de_casteljau(t: f32, p_i: f32, p1: f32, p2: f32, p_f: f32) -> f32 {
    // First level
    let q1 = lerp(t, p_i, p1);
    let q2 = lerp(t, p1, p2);
    let q3 = lerp(t, p2, p_f);

    // Second level
    let r1 = lerp(t, q1, q2);
    let r2 = lerp(t, q2, q3);

    // Final level
    lerp(t, r1, r2)
}

// Generates a cubic Bézier curve function from control points.
pub fn cubic_bezier(control_points: &[f32; 4]) -> Result<impl Fn(f32) -> f32 + use<>, BezierError> {
    let [x1, y1, x2, y2] = *control_points;

    // Ensure control points are within bounds.
    //
    // I think any y-value for the control points should be fine. But, we can't have negative
    // x-values for the control points, otherwise the cubic bezier function could have multiple
    // outputs for any given input 'x' (different from the control points' x-values), meaning we
    // would have a mathematical non-function.
    if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
        return Err(BezierError::InvalidControlPoint);
    }

    Ok(move |x: f32| {
        // If the curve is linear, shortcut.
        if x1 == y1 && x2 == y2 {
            return x;
        }

        // Boundary cases
        if x == 0.0 || x == 1.0 {
            return x;
        }

        let mut t0 = 0.0;
        let mut t1 = 1.0;
        let mut t = x;

        let p_i = Point { x: 0.0, y: 0.0 }; // Start point
        let p1 = Point { x: x1, y: y1 }; // First control point
        let p2 = Point { x: x2, y: y2 }; // Second control point
        let p_f = Point { x: 1.0, y: 1.0 }; // End point

        // Search for `t` from the 'x' given as an argument to this function.
        //
        // Note: 'x' and 't' are not the same. 'x' refers to the position along the x-axis, whereas
        // 't' refers to the position along the control point lines, hence why we need to search.
        for _ in 0..SUBDIVISION_MAX_ITERATIONS {
            // Evaluate the x-component of the Bézier curve at `t`
            let x_val = de_casteljau(t, p_i.x, p1.x, p2.x, p_f.x);
            let error = x - x_val;

            // Adjust the range based on the error.
            if error.abs() < SUBDIVISION_PRECISION {
                break;
            }
            if error > 0.0 {
                t0 = t;
            } else {
                t1 = t;
            }
            t = (t0 + t1) / 2.0;
        }

        // After finding 't', evalaute the y-component of the Bezier curve
        de_casteljau(t, p_i.y, p1.y, p2.y, p_f.y)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cubic_bezier() -> anyhow::Result<()> {
        let easing_fn = cubic_bezier(&[0.45, 0.0, 0.55, 1.0])?;

        let y_coord_0_2 = easing_fn(0.2);
        let y_coord_0_5 = easing_fn(0.5);

        assert!((0.07..=0.08).contains(&y_coord_0_2));
        assert!((0.499..=0.501).contains(&y_coord_0_5));

        Ok(())
    }
}
