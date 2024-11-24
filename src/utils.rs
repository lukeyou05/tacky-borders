use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*, Win32::Graphics::Dwm::*,
    Win32::UI::HiDpi::*, Win32::UI::WindowsAndMessaging::*,
};

use regex::Regex;
use std::ptr;
use std::thread;

use crate::border_config::MatchKind;
use crate::border_config::MatchStrategy;
use crate::border_config::WindowRule;
use crate::border_config::CONFIG;
use crate::colors::GradientCoordinates;
use crate::window_border;
use crate::SendHWND;
use crate::__ImageBase;
use crate::BORDERS;
use crate::INITIAL_WINDOWS;

// I need these because Rust doesn't allow expressions for a match pattern
pub const WM_APP_LOCATIONCHANGE: u32 = WM_APP;
pub const WM_APP_REORDER: u32 = WM_APP + 1;
pub const WM_APP_SHOWUNCLOAKED: u32 = WM_APP + 2;
pub const WM_APP_HIDECLOAKED: u32 = WM_APP + 3;
pub const WM_APP_MINIMIZESTART: u32 = WM_APP + 4;
pub const WM_APP_MINIMIZEEND: u32 = WM_APP + 5;
pub const WM_APP_ANIMATE: u32 = WM_APP + 6;
pub const WM_APP_FOCUS: u32 = WM_APP + 7;

// TODO THE CODE IS STILL A MESS

pub fn has_filtered_style(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

    style & WS_CHILD.0 != 0
        || ex_style & WS_EX_TOOLWINDOW.0 != 0
        || ex_style & WS_EX_NOACTIVATE.0 != 0
}

// Getting the window title sometimes takes unexpectedly long (over 1ms), but it should be fine.
pub fn get_window_title(hwnd: HWND) -> String {
    let mut title_arr: [u16; 256] = [0; 256];

    if unsafe { GetWindowTextW(hwnd, &mut title_arr) } == 0 {
        error!("Could not retrieve window title!");
    }

    let title_binding = String::from_utf16_lossy(&title_arr);
    return title_binding.split_once("\0").unwrap().0.to_string();
}

pub fn get_window_class(hwnd: HWND) -> String {
    let mut class_arr: [u16; 256] = [0; 256];

    if unsafe { GetClassNameW(hwnd, &mut class_arr) } == 0 {
        error!("Could not retrieve window class!");
    }

    let class_binding = String::from_utf16_lossy(&class_arr);
    return class_binding.split_once("\0").unwrap().0.to_string();
}

pub fn get_window_rule(hwnd: HWND) -> WindowRule {
    let title = get_window_title(hwnd);
    let class = get_window_class(hwnd);

    let config = CONFIG.lock().unwrap();

    for rule in config.window_rules.iter() {
        let name = match rule.kind {
            Some(MatchKind::Title) => &title,
            Some(MatchKind::Class) => &class,
            None => {
                error!("Expected 'match' for window rule but None found!");
                continue;
            }
        };

        let Some(pattern) = &rule.pattern else {
            error!("Expected `pattern` for window rule but None found!");
            continue;
        };

        if match rule.strategy {
            Some(MatchStrategy::Equals) | None => name.to_lowercase().eq(&pattern.to_lowercase()),
            Some(MatchStrategy::Contains) => name.to_lowercase().contains(&pattern.to_lowercase()),
            Some(MatchStrategy::Regex) => Regex::new(pattern).unwrap().captures(name).is_some(),
        } {
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

// If the tracking window does not have a window edge or is maximized, then there should be no
// border.
pub fn has_native_border(hwnd: HWND) -> bool {
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        ex_style & WS_EX_WINDOWEDGE.0 != 0 && style & WS_MAXIMIZE.0 == 0
    }
}

pub fn get_show_cmd(hwnd: HWND) -> u32 {
    let mut wp: WINDOWPLACEMENT = WINDOWPLACEMENT::default();
    let result = unsafe { GetWindowPlacement(hwnd, &mut wp) };
    if result.is_err() {
        error!("Could not retrieve window placement!");
        return 0;
    }
    wp.showCmd
}

pub fn create_border_for_window(tracking_window: HWND) -> Result<()> {
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

        let active_color = config_active.convert_to_color(true);
        let inactive_color = config_inactive.convert_to_color(false);

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
            active_animations: animations.active,
            inactive_animations: animations.inactive,
            animation_fps: animations.fps,
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
        let _ = border.create_border_window(hinstance);
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
        let _ = animations;
        let _ = window_isize;
        let _ = hinstance;

        let _ = border.init(initialize_delay);

        drop(border);
    });

    Ok(())
}

pub fn convert_config_radius(
    config_width: i32,
    config_radius: f32,
    tracking_window: HWND,
    dpi: f32,
) -> f32 {
    let mut corner_preference = DWM_WINDOW_CORNER_PREFERENCE::default();

    // -1.0 means to use default Windows corner preference. I might want to use an enum to allow
    // for something like border_radius == "system" instead TODO
    if config_radius == -1.0 {
        let result = unsafe {
            DwmGetWindowAttribute(
                tracking_window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                ptr::addr_of_mut!(corner_preference) as _,
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            )
        };
        if result.is_err() {
            error!("Could not retrieve window corner preference!");
        }
        match corner_preference {
            DWMWCP_DEFAULT => {
                return 8.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            DWMWCP_DONOTROUND => {
                return 0.0;
            }
            DWMWCP_ROUND => {
                return 8.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            DWMWCP_ROUNDSMALL => {
                return 4.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            _ => {}
        }
    }

    config_radius * dpi / 96.0
}

pub fn destroy_border_for_window(tracking_window: HWND) -> Result<()> {
    let window = SendHWND(tracking_window);

    let _ = thread::spawn(move || {
        let window_sent = window;
        let mut borders_hashmap = BORDERS.lock().unwrap();
        let window_isize = window_sent.0 .0 as isize;
        let Some(border_isize) = borders_hashmap.get(&window_isize) else {
            drop(borders_hashmap);
            return;
        };

        let border_window: HWND = HWND(*border_isize as _);
        unsafe {
            let _ = PostMessageW(border_window, WM_NCDESTROY, WPARAM(0), LPARAM(0));
        }
        borders_hashmap.remove(&window_isize);

        drop(borders_hashmap);
    });

    Ok(())
}

pub fn get_border_from_window(hwnd: HWND) -> Option<HWND> {
    let borders = BORDERS.lock().unwrap();
    let hwnd_isize = hwnd.0 as isize;
    let Some(border_isize) = borders.get(&hwnd_isize) else {
        drop(borders);
        return None;
    };

    let border_window: HWND = HWND(*border_isize as _);
    drop(borders);
    Some(border_window)
}

// Return true if the border exists in the border hashmap. Otherwise, create a new border and
// return false.
pub fn show_border_for_window(hwnd: HWND) -> bool {
    let border_window = get_border_from_window(hwnd);
    if let Some(hwnd) = border_window {
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_SHOWUNCLOAKED, WPARAM(0), LPARAM(0));
        }
        true
    } else {
        if is_window_visible(hwnd) && !is_cloaked(hwnd) && !has_filtered_style(hwnd) {
            let _ = create_border_for_window(hwnd);
        }
        false
    }
}

pub fn hide_border_for_window(hwnd: HWND) -> bool {
    let window = SendHWND(hwnd);

    let _ = thread::spawn(move || {
        let window_sent = window;
        let border_option = get_border_from_window(window_sent.0);
        if let Some(border_window) = border_option {
            unsafe {
                let _ = PostMessageW(border_window, WM_APP_HIDECLOAKED, WPARAM(0), LPARAM(0));
            }
        }
    });
    true
}

pub fn interpolate_d2d1_colors(
    current_color: &D2D1_COLOR_F,
    start_color: &D2D1_COLOR_F,
    end_color: &D2D1_COLOR_F,
    anim_elapsed: f32,
    anim_speed: f32,
    finished: &mut bool,
) -> D2D1_COLOR_F {
    // D2D1_COLOR_F has the copy trait so we can just do this to create an implicit copy
    let mut interpolated = *current_color;

    let anim_step = anim_elapsed * anim_speed;

    let diff_r = end_color.r - start_color.r;
    let diff_g = end_color.g - start_color.g;
    let diff_b = end_color.b - start_color.b;
    let diff_a = end_color.a - start_color.a;

    interpolated.r += diff_r * anim_step;
    interpolated.g += diff_g * anim_step;
    interpolated.b += diff_b * anim_step;
    interpolated.a += diff_a * anim_step;

    // Check if we have overshot the active_color
    // TODO if I also check the alpha here, then things start to break when opening windows, not
    // sure why. Might be some sort of conflict with interpoalte_d2d1_to_visible().
    if (interpolated.r - end_color.r) * diff_r.signum() >= 0.0
        && (interpolated.g - end_color.g) * diff_g.signum() >= 0.0
        && (interpolated.b - end_color.b) * diff_b.signum() >= 0.0
    {
        *finished = true;
        return *end_color;
    } else {
        *finished = false;
    }

    interpolated
}

pub fn interpolate_d2d1_to_visible(
    current_color: &D2D1_COLOR_F,
    end_color: &D2D1_COLOR_F,
    anim_elapsed: f32,
    anim_speed: f32,
    finished: &mut bool,
) -> D2D1_COLOR_F {
    let mut interpolated = *current_color;

    let anim_step = anim_elapsed * anim_speed;

    // Figure out which direction we should be interpolating
    let diff = end_color.a - interpolated.a;
    match diff.is_sign_positive() {
        true => interpolated.a += anim_step,
        false => interpolated.a -= anim_step,
    }

    if (interpolated.a - end_color.a) * diff.signum() >= 0.0 {
        *finished = true;
        return *end_color;
    } else {
        *finished = false;
    }

    interpolated
}

pub fn interpolate_direction(
    current_direction: &GradientCoordinates,
    start_direction: &GradientCoordinates,
    end_direction: &GradientCoordinates,
    anim_elapsed: f32,
    anim_speed: f32,
) -> GradientCoordinates {
    let mut interpolated = (*current_direction).clone();

    let x_start_step = end_direction.start[0] - start_direction.start[0];
    let y_start_step = end_direction.start[1] - start_direction.start[1];
    let x_end_step = end_direction.end[0] - start_direction.end[0];
    let y_end_step = end_direction.end[1] - start_direction.end[1];

    // Not gonna bother checking if we overshot the direction tbh
    let anim_step = anim_elapsed * anim_speed;
    interpolated.start[0] += x_start_step * anim_step;
    interpolated.start[1] += y_start_step * anim_step;
    interpolated.end[0] += x_end_step * anim_step;
    interpolated.end[1] += y_end_step * anim_step;

    interpolated
}
