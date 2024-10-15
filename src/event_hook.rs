use std::ffi::c_ulong;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::prelude::OsStringExt;
use core::ffi::c_void;
use core::ffi::c_int;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::*,
    Win32::Graphics::Direct2D::Common::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
};

pub struct DrawableRect {
    rect: Option<D2D_RECT_F>,
    rounded_rect: Option<D2D1_ROUNDED_RECT>,
    border_color: D2D1_COLOR_F,
    thickness: i32,
}

pub struct FrameDrawer {
   m_window: HWND,
   m_render_target_size_hash: i64,
   m_render_target: *mut ID2D1HwndRenderTarget,
   m_border_brush: *mut ID2D1SolidColorBrush,
   m_scene_rect: DrawableRect,
}

impl FrameDrawer {
    /*pub fn create(window: HWND) -> Box<WindowDrawer> {
        let mut drawer: Box<FrameDrawer> = Box::new(FrameDrawer {m_window: window,
                                                                 m_render_target_size_hash: 0,
                                                                 m_render_target: 0,
                                                                 m_border_brush: 0
                                                                 m_scene_rect: 0});
    }*/
}
