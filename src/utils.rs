use windows::Win32::Foundation::{
    GetLastError, BOOL, FALSE, HINSTANCE, HWND, LPARAM, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
    DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
};
use windows::Win32::UI::Input::Ime::ImmDisableIME;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowPlacement, GetWindowTextW,
    IsWindowVisible, PostMessageW, SendNotifyMessageW, GWL_EXSTYLE, GWL_STYLE, WINDOWPLACEMENT,
    WM_APP, WM_NCDESTROY, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE,
    WS_MAXIMIZE,
};

use anyhow::Context;
use regex::Regex;
use std::ptr;
use std::thread;

use crate::border_config::{MatchKind, MatchStrategy, WindowRule, CONFIG};
use crate::window_border;
use crate::{SendHWND, __ImageBase, BORDERS, INITIAL_WINDOWS};

pub const WM_APP_LOCATIONCHANGE: u32 = WM_APP;
pub const WM_APP_REORDER: u32 = WM_APP + 1;
pub const WM_APP_FOCUS: u32 = WM_APP + 2;
pub const WM_APP_SHOWUNCLOAKED: u32 = WM_APP + 3;
pub const WM_APP_HIDECLOAKED: u32 = WM_APP + 4;
pub const WM_APP_MINIMIZESTART: u32 = WM_APP + 5;
pub const WM_APP_MINIMIZEEND: u32 = WM_APP + 6;
pub const WM_APP_ANIMATE: u32 = WM_APP + 7;

// Note: don't use this macro with fatal errors since there's no real logic to handle them
#[macro_export]
macro_rules! log_if_err {
    ($err:expr) => {
        if let Err(e) = $err {
            // TODO for some reason if I use {:#} or {:?}, some errors will repeatedly print (like
            // the one in main.rs for tray_icon_result). It could have something to do with how they
            // implement .source()
            error!("{:#}", e);
        }
    };
}

pub fn has_filtered_style(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

    style & WS_CHILD.0 != 0
        || ex_style & WS_EX_TOOLWINDOW.0 != 0
        || ex_style & WS_EX_NOACTIVATE.0 != 0
}

pub fn get_window_title(hwnd: HWND) -> String {
    let mut title_arr: [u16; 256] = [0; 256];

    if unsafe { GetWindowTextW(hwnd, &mut title_arr) } == 0 {
        let last_error = unsafe { GetLastError() };
        error!("Could not retrieve window title for {hwnd:?}: {last_error:?}");
    }

    let title_binding = String::from_utf16_lossy(&title_arr);
    title_binding.split_once("\0").unwrap().0.to_string()
}

pub fn get_window_class(hwnd: HWND) -> String {
    let mut class_arr: [u16; 256] = [0; 256];

    if unsafe { GetClassNameW(hwnd, &mut class_arr) } == 0 {
        let last_error = unsafe { GetLastError() };
        error!("Could not retrieve window class for {hwnd:?}: {last_error:?}");
    }

    let class_binding = String::from_utf16_lossy(&class_arr);
    class_binding.split_once("\0").unwrap().0.to_string()
}

// Get the window rule from 'window_rules' in the config
pub fn get_window_rule(hwnd: HWND) -> WindowRule {
    let title = get_window_title(hwnd);
    let class = get_window_class(hwnd);

    let config = CONFIG.lock().unwrap();

    for rule in config.window_rules.iter() {
        let window_name = match rule.kind {
            Some(MatchKind::Title) => &title,
            Some(MatchKind::Class) => &class,
            None => {
                error!("Expected 'match' for window rule but None found!");
                continue;
            }
        };

        let Some(match_name) = &rule.name else {
            error!("Expected `name` for window rule but None found!");
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

        if has_match {
            return rule.clone();
        }
    }

    drop(config);
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

pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut is_cloaked = FALSE;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            ptr::addr_of_mut!(is_cloaked) as _,
            size_of::<BOOL>() as u32,
        )
    };
    if result.is_err() {
        error!("Could not check if window is cloaked");
        return true;
    }
    is_cloaked.as_bool()
}

pub fn is_active_window(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

pub fn post_message_w(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::core::Result<()> {
    unsafe { PostMessageW(hwnd, msg, wparam, lparam) }
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
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        ex_style & WS_EX_WINDOWEDGE.0 != 0 && style & WS_MAXIMIZE.0 == 0
    }
}

pub fn get_show_cmd(hwnd: HWND) -> u32 {
    let mut wp: WINDOWPLACEMENT = WINDOWPLACEMENT::default();
    if let Err(e) = unsafe { GetWindowPlacement(hwnd, &mut wp) } {
        error!("Could not retrieve window placement; {e}");
        return 0;
    }
    wp.showCmd
}

pub fn create_border_for_window(tracking_window: HWND) {
    debug!("Creating border for: {:?}", tracking_window);
    let window = SendHWND(tracking_window);

    let _ = thread::spawn(move || {
        let window_sent = window;

        let window_rule = get_window_rule(window_sent.0);
        if window_rule.enabled == Some(false) {
            info!("border is disabled for {:?}!", window_sent.0);
            return;
        }

        let config = CONFIG.lock().unwrap();

        // TODO holy this is ugly
        let config_width = window_rule
            .border_width
            .unwrap_or(config.global.border_width);
        let config_offset = window_rule
            .border_offset
            .unwrap_or(config.global.border_offset);
        let config_radius = window_rule
            .border_radius
            .unwrap_or(config.global.border_radius);
        let config_active = window_rule
            .active_color
            .unwrap_or(config.global.active_color.clone());
        let config_inactive = window_rule
            .inactive_color
            .unwrap_or(config.global.inactive_color.clone());

        // Convert ColorConfig structs to Color
        let active_color = config_active.convert_to_color(true);
        let inactive_color = config_inactive.convert_to_color(false);

        // Adjust the border width and radius based on the monitor/window dpi
        let dpi = unsafe { GetDpiForWindow(window_sent.0) } as f32;
        let border_width = (config_width * dpi / 96.0) as i32;
        let border_radius = convert_config_radius(border_width, config_radius, window_sent.0, dpi);

        let animations = window_rule
            .animations
            .unwrap_or(config.global.animations.clone().unwrap_or_default());

        let window_isize = window_sent.0 .0 as isize;

        let initialize_delay = if INITIAL_WINDOWS.lock().unwrap().contains(&window_isize) {
            0
        } else {
            window_rule
                .initialize_delay
                .unwrap_or(config.global.initialize_delay.unwrap_or(250))
        };
        let unminimize_delay = window_rule
            .unminimize_delay
            .unwrap_or(config.global.unminimize_delay.unwrap_or(200));

        let mut border = window_border::WindowBorder {
            tracking_window: window_sent.0,
            border_width,
            border_offset: config_offset,
            border_radius,
            active_color,
            inactive_color,
            animations,
            unminimize_delay,
            ..Default::default()
        };

        drop(config);

        let mut borders_hashmap = BORDERS.lock().unwrap();

        // Check to see if the key already exists in the hashmap. I don't think this should ever
        // return true, but it's just in case.
        if borders_hashmap.contains_key(&window_isize) {
            drop(borders_hashmap);
            return;
        }

        let hinstance: HINSTANCE = unsafe { std::mem::transmute(&__ImageBase) };

        if let Err(e) = border.create_border_window(hinstance) {
            error!("Could not create border window! {:?}", e);
            return;
        };

        // Insert the border and its tracking window into the hashmap to keep track of them
        borders_hashmap.insert(window_isize, border.border_window.0 as isize);

        // Drop these values (to save some RAM?) before calling init and entering a message loop
        drop(borders_hashmap);
        let _ = window_sent;
        let _ = window_rule;
        let _ = config_width;
        let _ = config_offset;
        let _ = config_radius;
        let _ = config_active;
        let _ = config_inactive;
        let _ = active_color;
        let _ = inactive_color;
        let _ = dpi;
        let _ = border_width;
        let _ = border_radius;
        let _ = animations;
        let _ = window_isize;
        let _ = initialize_delay;
        let _ = unminimize_delay;
        let _ = hinstance;

        // Note: init() contains a loop, so this should never return unless it's an Error
        if let Err(e) = border.init(initialize_delay) {
            error!("{}", e);
        }
    });
}

fn convert_config_radius(
    border_width: i32,
    config_radius: f32,
    tracking_window: HWND,
    dpi: f32,
) -> f32 {
    // TODO use an enum for config_radius instead (-1.0 means we should automatically get radius,
    // so maybe use "Auto" for the enum)
    match config_radius {
        -1.0 => {
            let window_radius = get_window_radius(tracking_window, dpi);
            match window_radius {
                0.0 => 0.0,
                _ => window_radius + border_width as f32 / 2.0,
            }
        }
        _ => config_radius * dpi / 96.0,
    }
}

fn get_window_radius(tracking_window: HWND, dpi: f32) -> f32 {
    let mut corner_preference = DWM_WINDOW_CORNER_PREFERENCE::default();

    if let Err(e) = unsafe {
        DwmGetWindowAttribute(
            tracking_window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            ptr::addr_of_mut!(corner_preference) as _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    } {
        error!("Could not retrieve window corner preference; {e}");
    }

    match corner_preference {
        DWMWCP_DEFAULT => 8.0 * dpi / 96.0,
        DWMWCP_DONOTROUND => 0.0,
        DWMWCP_ROUND => 8.0 * dpi / 96.0,
        DWMWCP_ROUNDSMALL => 4.0 * dpi / 96.0,
        _ => 8.0 * dpi / 96.0,
    }
}

pub fn destroy_border_for_window(tracking_window: HWND) {
    let window_isize = tracking_window.0 as isize;
    let Some(&border_isize) = BORDERS.lock().unwrap().get(&window_isize) else {
        return;
    };

    let border_window: HWND = HWND(border_isize as _);
    log_if_err!(
        post_message_w(border_window, WM_NCDESTROY, WPARAM(0), LPARAM(0))
            .context("destroy_border_for_window")
    );
}

pub fn get_border_from_window(hwnd: HWND) -> Option<HWND> {
    let borders_hashmap = BORDERS.lock().unwrap();

    let hwnd_isize = hwnd.0 as isize;
    let Some(border_isize) = borders_hashmap.get(&hwnd_isize) else {
        drop(borders_hashmap);
        return None;
    };

    let border_window: HWND = HWND(*border_isize as _);
    drop(borders_hashmap);

    Some(border_window)
}

pub fn show_border_for_window(hwnd: HWND) {
    // If the border already exists, simply post a 'SHOW' message to its message queue. Otherwise,
    // create a new border.
    if let Some(border) = get_border_from_window(hwnd) {
        log_if_err!(
            post_message_w(border, WM_APP_SHOWUNCLOAKED, WPARAM(0), LPARAM(0))
                .context("show_border_for_window")
        );
    } else if is_window_visible(hwnd) && !is_cloaked(hwnd) && !has_filtered_style(hwnd) {
        create_border_for_window(hwnd);
    }
}

pub fn hide_border_for_window(hwnd: HWND) -> bool {
    let window = SendHWND(hwnd);

    // Spawn a new thread to guard against re-entrancy in the event hook, though it honestly isn't
    // that important for our purposes I think
    let _ = thread::spawn(move || {
        let window_sent = window;

        if let Some(border) = get_border_from_window(window_sent.0) {
            log_if_err!(
                post_message_w(border, WM_APP_HIDECLOAKED, WPARAM(0), LPARAM(0))
                    .context("hide_border_for_window")
            );
        }
    });
    true
}

// Bezier curve algorithm together with @0xJWLabs
const SUBDIVISION_PRECISION: f32 = 0.0001; // Precision for binary subdivision
const SUBDIVISION_MAX_ITERATIONS: u32 = 10; // Maximum number of iterations for binary subdivision

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
pub fn cubic_bezier(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) -> Result<impl Fn(f32) -> f32, BezierError> {
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
