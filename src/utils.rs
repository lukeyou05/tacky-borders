use windows::{
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::Common::*,
};

use crate::*;
use crate::border_config::CONFIG;

// TODO THE CODE IS STILL A MESS

pub fn has_filtered_style(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

    if style & WS_CHILD.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0  {
        return true;
    }
    return false;
}

pub fn create_border_for_window(tracking_window: HWND, delay: u64) -> Result<()> {
    let borders_mutex = unsafe { &*BORDERS };
    let config_mutex = unsafe { &*CONFIG };
    let window = SendHWND(tracking_window);

    let thread = std::thread::spawn(move || {
        let window_sent = window;

        // This delay can be used to wait for a window to finish its opening animation or for it to
        // become visible if it is not so at first
        std::thread::sleep(std::time::Duration::from_millis(delay));
        if unsafe { !IsWindowVisible(window_sent.0).as_bool() } {
            return;
        }

        //let before = std::time::Instant::now();
        let active_color: D2D1_COLOR_F;
        let inactive_color: D2D1_COLOR_F;
        let config = config_mutex.lock().unwrap();

        if config.active_color == "accent" || config.inactive_color == "accent" {
            // Get the Windows accent color
            let mut pcr_colorization: u32 = 0;
            let mut pf_opaqueblend: BOOL = FALSE;
            let result = unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) };
            if result.is_err() {
                println!("Error getting Windows accent color!");
            }
            let red = ((pcr_colorization & 0x00FF0000) >> 16) as f32/255.0;
            let green = ((pcr_colorization & 0x0000FF00) >> 8) as f32/255.0;
            let blue = ((pcr_colorization & 0x000000FF) >> 0) as f32/255.0;
            let avg = (red + green + blue)/3.0;

            if config.active_color == "accent" {
                active_color = D2D1_COLOR_F {
                    r: red,
                    g: green,
                    b: blue,
                    a: 1.0
                };
            } else {
                active_color = get_color_from_hex(config.active_color.as_str());
            }

            if config.inactive_color == "accent" {
                inactive_color = D2D1_COLOR_F {
                    r: avg/1.5 + red/10.0,
                    g: avg/1.5 + green/10.0,
                    b: avg/1.5 + blue/10.0,
                    a: 1.0
                };
            } else {
                inactive_color = get_color_from_hex(config.inactive_color.as_str());
            }
        } else {
            active_color = get_color_from_hex(config.active_color.as_str());
            inactive_color = get_color_from_hex(config.inactive_color.as_str());
        }
        //println!("time it takes to get colors: {:?}", before.elapsed());

        let mut border = window_border::WindowBorder { 
            tracking_window: window_sent.0, 
            border_size: config.border_size, 
            border_offset: config.border_offset,
            force_border_radius: config.border_radius,
            active_color: active_color,
            inactive_color: inactive_color,
            ..Default::default()
        };
        drop(config);

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

pub fn destroy_border_of_window(tracking_window: HWND) -> Result<()> {
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
            // TODO figure out why DestroyWindow doesn't work
            //unsafe { DestroyWindow(border_window) };
            borders_hashmap.remove(&window_isize);
        }

        drop(borders_hashmap);
    });

    return Ok(());
}

pub fn get_border_from_window(hwnd: HWND) -> Option<HWND> {
    let mutex = unsafe { &*BORDERS };
    let borders = mutex.lock().unwrap();
    let hwnd_isize = hwnd.0 as isize;
    let border_option = borders.get(&hwnd_isize);

    if border_option.is_some() {
        let border_window: HWND = HWND(*border_option.unwrap() as _);
        drop(borders);
        return Some(border_window);
    } else {
        drop(borders);
        return None;
    }
}

// Return true if the border exists in the border hashmap. Otherwise, return false.
// Specify a delay to prevent the border from appearing while a window is in its opening animation.
pub fn show_border_for_window(hwnd: HWND, delay: u64) -> bool {
    let mutex = unsafe { &*BORDERS };
    let borders = mutex.lock().unwrap();
    let hwnd_isize = hwnd.0 as isize;
    let border_option = borders.get(&hwnd_isize);

    if border_option.is_some() {
        let border_window: HWND = HWND(*border_option.unwrap() as _);
        drop(borders);
        unsafe { ShowWindow(border_window, SW_SHOWNA) };
        return true;
    } else {
        drop(borders);
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
            return false;
        }

        // When a popup window is created, it can mess up the z-order of the border so we
        // reset it here. I also wait a milisecond for the popup window to set its
        // position. THIS IS SO TACKY LMAO. TODO
        if style & WS_POPUP.0 != 0 {
            //println!("popup window created!");
            std::thread::sleep(std::time::Duration::from_millis(1));
            let borders = mutex.lock().unwrap();

            // Get the parent window of the popup so we can find the border window and reset its
            // position
            let parent = unsafe { GetParent(hwnd) };
            if parent.is_ok() {
                let parent_isize = parent.unwrap().0 as isize;
                let border_option = borders.get(&parent_isize);
                if border_option.is_some() {
                    let border_window = HWND(*border_option.unwrap() as *mut _);
                    unsafe { PostMessageW(border_window, WM_SETFOCUS, WPARAM(0), LPARAM(0)) };
                }
            }
            drop(borders);
        }

        if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_CHILD.0 == 0 && !is_cloaked.as_bool() {
            //println!("creating window border for: {:?}", hwnd);
            create_border_for_window(hwnd, delay);
        }
        return false;
    }
}

pub fn hide_border_of_window(hwnd: HWND) -> bool {
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
            unsafe { ShowWindow(border_window, SW_HIDE) };
        } else {
            drop(borders);
        }
    });
    return true;
}

pub fn get_color_from_hex(hex: &str) -> D2D1_COLOR_F {
    // Assuming hex is a string in the format "#FFFFFF" or "FFFFFF"
    let hex = hex.trim_start_matches('#'); // Remove leading '#'
    // Convert hex string to u32
    let value = u32::from_str_radix(hex, 16).expect("Invalid hex color");
    // Extract RGB components as f32 values
    let red = ((value & 0x00FF0000) >> 16) as f32 / 255.0; // Red component
    let green = ((value & 0x0000FF00) >> 8) as f32 / 255.0; // Green component
    let blue = (value & 0x000000FF) as f32 / 255.0; // Blue component
    // Return the D2D1_COLOR_F struct
    D2D1_COLOR_F {
        r: red,
        g: green,
        b: blue,
        a: 1.0,
    }
}
pub fn get_color_from_rgba(rgba: &str) -> D2D1_COLOR_F {
    let rgba = rgba.trim_start_matches("rgb(").trim_start_matches("rgba(").trim_end_matches(')');
    let components: Vec<&str> = rgba.split(',').map(|s| s.trim()).collect();
    // Check for correct number of components
    if components.len() == 3 || components.len() == 4 {
        // Parse red, green, and blue values
        let red: f32 = components[0].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let green: f32 = components[1].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let blue: f32 = components[2].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let alpha: f32 = if components.len() == 4 {
            components[3].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        return D2D1_COLOR_F {
            r: red,
            g: green,
            b: blue,
            a: alpha, // Default alpha value for rgb()
        };
    }
    // Return a default color if parsing fails
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
pub fn get_color_from_oklch(oklch: &str) -> D2D1_COLOR_F {
    let oklch = oklch.trim_start_matches("oklch(").trim_end_matches(')');
    let components: Vec<&str> = oklch.split(',').map(|s| s.trim()).collect(); // Split by commas
    // Check for the correct number of components (3)
    if components.len() == 3 {
        // Parse lightness, chroma, and hue values
        let lightness_str = components[0];
        let lightness: f64 = if lightness_str.ends_with('%') {
            lightness_str.trim_end_matches('%').parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0) / 100.0 // Convert percentage to a 0.0 - 1.0 range
        } else {
            lightness_str.parse::<f64>().unwrap_or(0.0).clamp(0.0, 1.0) // Handle non-percentage case
        };
        let chroma: f64 = components[1].parse::<f64>().unwrap_or(0.0).clamp(0.0, f64::MAX);
        let hue: f64 = components[2].parse::<f64>().unwrap_or(0.0).clamp(0.0, 360.0);
        // Convert OKLCH to RGB
        let (r, g, b) = oklch_to_rgb(lightness, chroma, hue);
        return D2D1_COLOR_F {
            r: r as f32, // Convert back to f32 for D2D1_COLOR_F
            g: g as f32,
            b: b as f32,
            a: 1.0, // Default alpha value
        };
    }
    // Return a default color if parsing fails
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
// Placeholder for the actual OKLCH to RGB conversion function
fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> (f64, f64, f64) {
    // Implement the conversion from OKLCH to RGB here
    // For now, returning a placeholder RGB value
    (lightness, chroma, hue) // This is just a placeholder; replace with actual conversion logic
}
pub fn get_color_from_hsl(hsl: &str) -> D2D1_COLOR_F {
    let hsl = hsl.trim_start_matches("hsl(").trim_end_matches(')');
    let components: Vec<&str> = hsl.split(',').map(|s| s.trim()).collect(); // Split by commas
    // Check for the correct number of components (3)
    if components.len() == 3 {
        // Parse hue, saturation, and lightness values
        let hue: f64 = components[0].parse::<f64>().unwrap_or(0.0).clamp(0.0, 360.0);
        
        let saturation_str = components[1];
        let saturation: f64 = if saturation_str.ends_with('%') {
            saturation_str.trim_end_matches('%').parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0) / 100.0 // Convert percentage to a 0.0 - 1.0 range
        } else {
            saturation_str.parse::<f64>().unwrap_or(0.0).clamp(0.0, 1.0) // Handle non-percentage case
        };
        let lightness_str = components[2];
        let lightness: f64 = if lightness_str.ends_with('%') {
            lightness_str.trim_end_matches('%').parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0) / 100.0 // Convert percentage to a 0.0 - 1.0 range
        } else {
            lightness_str.parse::<f64>().unwrap_or(0.0).clamp(0.0, 1.0) // Handle non-percentage case
        };
        // Convert HSL to RGB
        let (r, g, b) = hsl_to_rgb(hue, saturation, lightness);
        return D2D1_COLOR_F {
            r: r as f32, // Convert back to f32 for D2D1_COLOR_F
            g: g as f32,
            b: b as f32,
            a: 1.0, // Default alpha value
        };
    }
    // Return a default color if parsing fails
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
// Placeholder for the actual HSL to RGB conversion function
fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (f64, f64, f64) {
    // Implement the conversion from HSL to RGB here
    // For now, returning a placeholder RGB value
    // This is just a placeholder; replace with actual conversion logic
    // HSL to RGB conversion logic
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation; // Chroma
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs()); // Second largest component
    let m = lightness - c / 2.0; // Match lightness
    
    let (r_prime, g_prime, b_prime) = match hue {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    // Convert to RGB and apply match lightness
    let r = (r_prime + m).clamp(0.0, 1.0);
    let g = (g_prime + m).clamp(0.0, 1.0);
    let b = (b_prime + m).clamp(0.0, 1.0);
    (r, g, b)
}
