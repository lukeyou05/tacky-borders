/*#![windows_subsystem = "windows"]*/
#![allow(unused)]

use std::ffi::c_ulong;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::prelude::OsStringExt;
/*use winapi::ctypes::c_int;*/
/*use winapi::ctypes::c_void;*/
use core::ffi::c_void;
use core::ffi::c_int;
/*use winapi::shared::minwindef::{BOOL, LPARAM};
use winapi::shared::windef::HWND;
use winapi::um::dwmapi::DwmSetWindowAttribute;
use winapi::um::shellapi::ShellExecuteExW;
use winapi::um::shellapi::SEE_MASK_NOASYNC;
use winapi::um::shellapi::SEE_MASK_NOCLOSEPROCESS;
use winapi::um::shellapi::SHELLEXECUTEINFOW;
use winapi::um::winuser::EnumWindows;
use winapi::um::winuser::GetClassNameW;
use winapi::um::winuser::GetWindowTextLengthW;
use winapi::um::winuser::GetWindowTextW;
use winapi::um::winuser::WS_EX_TOOLWINDOW;
use winapi::um::winuser::{
  DispatchMessageW, GetForegroundWindow, GetMessageW, IsWindowVisible, TranslateMessage,
  GWL_EXSTYLE,
};

use winapi::shared::winerror::SUCCEEDED;
use winapi::um::dwmapi::DwmGetColorizationColor;
use winapi::um::winnt::{KEY_READ, KEY_WRITE};
use winapi::um::winuser::MessageBoxA;
use winapi::um::winuser::MB_ICONERROR;
use winapi::um::winuser::MB_OK;
use winapi::um::winuser::GetWindowLongW;*/

mod border;
mod drawer;

const DWMWA_COLOR_DEFAULT: u32 = 0xFFFFFFFF;
const DWMWA_COLOR_NONE: u32 = 0xFFFFFFFE;
const COLOR_INVALID: u32 = 0x000000FF;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
};

extern "C" {
    static __ImageBase: IMAGE_DOS_HEADER;
}

fn main() {
    print!("applying colors\n");
    let m_tracking_window: Option<HWND> = None; 
    apply_colors(true);
    print!("finished applying\n");
}

fn apply_colors(reset: bool) {
    let mut visible_windows: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut visible_windows as *mut _ as isize),
        );
    }

    for hwnd in visible_windows {
        unsafe {
            let active = GetForegroundWindow();
            let string = "#FF0000";
            let rgb_red = hex_to_colorref(&string);
            let rgb_green = 65280 as u32;

            if active == hwnd {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &rgb_red as *const _ as *const c_void, 
                    std::mem::size_of::<c_ulong>() as u32,
                );

                print!("{:X}\n", rgb_red);
                assign_border(hwnd);
            }
        }
    }
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
  if IsWindowVisible(hwnd).into() {
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

    // Exclude certain window styles like WS_EX_TOOLWINDOW
    if ex_style & (WS_EX_TOOLWINDOW.0) == 0 {
      let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam);
      visible_windows.push(hwnd);
    }
  }

  BOOL(1)
}

pub fn hex_to_colorref(hex: &str) -> u32 {
  let r = u8::from_str_radix(&hex[1..3], 16);
  let g = u8::from_str_radix(&hex[3..5], 16);
  let b = u8::from_str_radix(&hex[5..7], 16);

  match (r, g, b) {
    (Ok(r), Ok(g), Ok(b)) => (b as u32) << 16 | (g as u32) << 8 | r as u32,
    _ => {
      COLOR_INVALID
    }
  }
}

pub fn assign_border(window: HWND) -> bool {
    unsafe {
        if window == GetForegroundWindow() {
            let m_hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);
            let border = border::WindowBorder::create(window, m_hinstance);
        }
    }
    return true;
}
