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
    Foundation::Numerics::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::*,
    Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
};
use std::cell::Cell;
use std::sync::LazyLock;

// Can I use mod drawer here somehow?
/*use crate::drawer::*;*/
use crate::event_hook;

thread_local! {
    pub static BORDER_POINTER: Cell<Option<*mut WindowBorder>> = Cell::new(None);
    pub static FACTORY_POINTER: Cell<Option<*const ID2D1Factory>> = Cell::new(None);
}

const SW_SHOWNA: i32 = 8;

pub static FACTORY: LazyLock<ID2D1Factory> = unsafe { LazyLock::new(|| D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None).expect("REASON")) };

/*#[derive(Debug, Default, Copy, Clone)]
pub struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}*/

#[derive(Debug, Default, Copy, Clone)]
pub struct WindowBorder {
    pub m_window: HWND,
    pub m_tracking_window: HWND,
    pub window_rect: RECT,
    pub border_size: i32,
    pub border_offset: i32,
    pub win_event_hook: HWINEVENTHOOK,
    pub dpi: f32,
    pub render_target_properties: D2D1_RENDER_TARGET_PROPERTIES,
    pub hwnd_render_target_properties: D2D1_HWND_RENDER_TARGET_PROPERTIES,
    pub m_border_brush: D2D1_BRUSH_PROPERTIES,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub color: D2D1_COLOR_F,
    //pub factory: &'static ID2D1Factory,
}

impl WindowBorder {
    pub fn create(window: HWND) -> WindowBorder {
        // let mut border: Box<WindowBorder> = Box::new(WindowBorder { m_window: HWND::default(), m_tracking_window: window } );
        //static DEBUG_LEVEL: D2D1_DEBUG_LEVEL = D2D1_DEBUG_LEVEL(0);
        //static FACTORY_OPTIONS: D2D1_FACTORY_OPTIONS = D2D1_FACTORY_OPTIONS { debugLevel: DEBUG_LEVEL };
        //static FACTORY_OPTIONS_POINTER: &'static D2D1_FACTORY_OPTIONS = &FACTORY_OPTIONS;
        //static factory_init: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(FACTORY_OPTIONS_POINTER)).expect("REASON") };
        //static FACTORY_POINTER: &ID2D1Factory = &*FACTORY;
        let mut border = WindowBorder { 
            m_window: HWND::default(), 
            m_tracking_window: window, 
            window_rect: RECT::default(), 
            border_size: 4, 
            border_offset: 1,
            //factory: &factory_init,
            //..unsafe{std::mem::zeroed()}
            ..Default::default()
        };
        BORDER_POINTER.replace(Some(std::ptr::addr_of_mut!(border)));
        //println!("border_pointer (creation): {:?}", std::ptr::addr_of_mut!(border));
        //println!("m_tracking_window (creation): {:?}", border.m_tracking_window);
        //TODO maybe check if dpi_aware is true or not
        let dpi_aware = unsafe { SetProcessDPIAware() };

        //println!("hinstance: {:?}", hinstance);
        //println!("border.m_window: {:?}", border.m_window);
        //println!("border.m_tracking_window: {:?}", border.m_tracking_window);

        // The lines below are currently useless because if a WindowBorder is successfully
        // initialized, it will be in a message loop and will never reach this part of the code.
        /*match WindowBorder::init(&mut border, hinstance) {
            Ok(val) => return border,
            Err(err) => println!("Error! {}", err),
        }*/

        return border;
    }

    pub fn init(&mut self, hinstance: HINSTANCE) -> Result<()> {
        /*let window_rect_opt: Option<RECT> = match self.m_tracking_window {
            Some(x) => get_frame_rect(x),
            None => return false,
        };*/

        if self.m_tracking_window.is_invalid() {
            /*return Err();*/
            println!("Error at m_tracking_window!");
        }

        self.get_frame_rect()?;

        /*let window_rect: RECT;
        match window_rect_opt {
            Some(val) => window_rect = val,
            /*None => return Err(),*/
            None => return Ok(()),
        };*/

        // println!("window_rect: {:?}", window_rect);

        unsafe {
            self.m_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                /*WS_EX_TOPMOST | WS_EX_TOOLWINDOW,*/
                w!("tacky-border"),
                w!("tacky-border"),
                WS_POPUP | WS_DISABLED,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                None,
                None,
                hinstance,
                Some(std::mem::transmute(&mut *self))
            )?;

            // println!("self: {:?}", self);

            // make window transparent
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            //println!("pos: {:?}", pos);
            let hrgn = CreateRectRgn(pos, 0, (pos + 1), 1);
            let mut bh: DWM_BLURBEHIND = Default::default();
            if !hrgn.is_invalid() {
                bh = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: TRUE,
                    hRgnBlur: hrgn,
                    fTransitionOnMaximized: FALSE
                };
            }

            DwmEnableBlurBehindWindow(self.m_window, &bh);

            if SetLayeredWindowAttributes(self.m_window, COLORREF(0x00000000), 0, LWA_COLORKEY).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.m_window, COLORREF(0x00000000), 255, LWA_ALPHA).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }

            // set position of the border-window behind the tracking window
            // helps to prevent border overlapping (happens after turning borders off and on)
            let set_pos = SetWindowPos(self.m_tracking_window,
                self.m_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                SWP_NOMOVE | SWP_NOSIZE);

            if set_pos.is_err() {
                println!("Error with SetWindowPos!");
            }
            
            let val: BOOL = TRUE;

            // I doubt the code below is functioning properly (the std::mem::transmute(&val))
            DwmSetWindowAttribute(self.m_window, DWMWA_EXCLUDED_FROM_PEEK, std::mem::transmute(&val), size_of::<BOOL>() as u32);
            //println!("pointer to BOOL: {:?} {:?}", &val, std::mem::transmute::<&BOOL, isize>(&val));

            ShowWindow(self.m_window, SHOW_WINDOW_CMD(SW_SHOWNA));

            UpdateWindow(self.m_window);

            /*self.win_event_hook = SetWinEventHook(
                EVENT_MIN,
                EVENT_MAX,
                None,
                Some(event_hook::handle_win_event),
                0,
                GetWindowThreadProcessId(self.m_tracking_window, None),
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );*/

            // println!("self.m_window (from init): {:?}", self.m_window);
            self.create_render_targets();
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)?;
            FACTORY_POINTER.replace(Some(std::ptr::addr_of!(factory)));
            self.render(&factory);
            /*loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                /*self.get_frame_rect();
                self.render(&factory);
                SetWindowPos(self.m_window,
                    self.m_tracking_window,
                    self.window_rect.left,
                    self.window_rect.top,
                    self.window_rect.right - self.window_rect.left,
                    self.window_rect.bottom - self.window_rect.top,
                SWP_NOREDRAW | SWP_NOACTIVATE
                );*/
                self.update();
            }*/

            //println!("border hwnd: {:?}", self.m_window);
            
            //TODO replace std::thread::sleep with a dedicated timer for the update function so
            //we don't miss any of the other messages.
            let mut message = MSG::default();

            //let mut before = std::time::Instant::now();
            //let idle = 15;
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                //let before = std::time::Instant::now();
                /*if message.message != idle {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                    //println!("Elapsed time (message loop): {:.2?}", before.elapsed());
                }*/
                if message.hwnd != self.m_window {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                //self.update(&factory);
                std::thread::sleep(std::time::Duration::from_millis(5));
                //println!("Elapsed time (message loop): {:.2?}", before.elapsed());
                //before = std::time::Instant::now();
            }
            println!("Potential error with message loop, exiting!");
        }

        return Ok(());
    }

    pub fn get_frame_rect(&mut self) -> Result<()> {
        //unsafe { println!("m_tracking_window: {:?}", self.m_tracking_window) };
        if unsafe { DwmGetWindowAttribute(self.m_tracking_window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut self.window_rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
            println!("Error getting frame rect!");
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }


    pub fn create_render_targets(&mut self) {
        self.dpi = 96.0;
        self.render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { 
                format: DXGI_FORMAT_UNKNOWN, 
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED 
            },
            dpiX: self.dpi,
            dpiY: self.dpi,
            ..Default::default() };

        self.hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { 
            hwnd: self.m_window, 
            pixelSize: Default::default(), 
            presentOptions: D2D1_PRESENT_OPTIONS_NONE 
        };

        self.m_border_brush = D2D1_BRUSH_PROPERTIES { 
            opacity: 1.0 as f32, 
            transform: Default::default() 
        };

        self.rounded_rect = D2D1_ROUNDED_RECT { 
            rect: Default::default(), 
            radiusX: 6.0 + ((self.border_size/2) as f32), 
            radiusY: 6.0 + ((self.border_size/2) as f32)
        };

        self.color = D2D1_COLOR_F { 
            r: 0.0, 
            g: 0.0, 
            b: 0.0, 
            a: 1.0 
        };
        if unsafe { GetForegroundWindow() } == self.m_tracking_window {
            self.set_color(true);
        } else {
            self.set_color(false);
        }
    }

    pub fn render(&mut self, factory: &ID2D1Factory) -> Result<()> {
        /*let render_target_size = D2D_SIZE_U { width: (client_rect.right - client_rect.left) as u32, height: (client_rect.bottom - client_rect.top) as u32 };*/
        self.hwnd_render_target_properties.pixelSize = D2D_SIZE_U { 
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32
        };

        //println!("hwnd_render_target_properties: {:?}", hwnd_render_target_properties);

        unsafe {
            let m_render_target = factory.CreateHwndRenderTarget(&self.render_target_properties, &self.hwnd_render_target_properties)?;
            // I'm not even sure what SetAntiAliasMode does because without the line, the corners are still anti-aliased.
            // Maybe there's an ever so slight bit more anti-aliasing with it but I could just be crazy. 
            //m_render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let m_brush = m_render_target.CreateSolidColorBrush(&self.color, Some(&self.m_border_brush))?;
            //println!("m_brush: {:?}", color);

            // Yes, the size calculations below are confusing, but they work, and that's all that
            // really matters.
            self.rounded_rect.rect = D2D_RECT_F { 
                left: (self.border_size/2 + self.border_offset) as f32, 
                top: (self.border_size/2 + self.border_offset) as f32, 
                right: (self.window_rect.right - self.window_rect.left - self.border_size/2 - self.border_offset) as f32, 
                bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 - self.border_offset) as f32
            };

            //println!("m_render_target: {:?}", m_render_target);

            m_render_target.BeginDraw();
            m_render_target.Clear(None);
            m_render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &m_brush,
                self.border_size as f32,
                None
            );
            m_render_target.EndDraw(None, None);
        }

        Ok(())
    }

    pub fn update(&mut self) {
        //let factory_pointer = FACTORY_POINTER.get().unwrap();
        //let before = std::time::Instant::now();
        let factory: &ID2D1Factory = &*FACTORY;
        let old_rect = self.window_rect.clone();
        self.get_frame_rect();
        unsafe {
            SetWindowPos(self.m_window,
                self.m_tracking_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                SWP_NOREDRAW | SWP_NOACTIVATE
            );

            self.render(factory);
        }
        //println!("Elapsed time (update): {:.2?}", before.elapsed());
        // Below is just proof of concept, should probably implement an equals function in the
        // future.
        /*if old_rect.top != self.window_rect.top ||
            old_rect.right != self.window_rect.right ||
            old_rect.left != self.window_rect.left ||
            old_rect.bottom != self.window_rect.bottom {
            unsafe {
                SetWindowPos(self.m_window,
                    self.m_tracking_window,
                    self.window_rect.left,
                    self.window_rect.top,
                    self.window_rect.right - self.window_rect.left,
                    self.window_rect.bottom - self.window_rect.top,
                    SWP_NOREDRAW | SWP_NOACTIVATE
                );

                self.render(factory);
            }
        }*/
        /*unsafe {
            let mut next_window = GetWindow(self.m_tracking_window, GW_HWNDNEXT).unwrap();
            while !IsWindowVisible(next_window).as_bool() {
                next_window = GetWindow(self.m_tracking_window, GW_HWNDLAST).unwrap();
            }
            println!("next_window: {:?}", next_window);
            println!("self.m_window: {:?}", self.m_window);
            if IsWindowVisible(next_window).as_bool() && self.m_window != next_window {
                println!("this thing!");
                SetWindowPos(self.m_window,
                    self.m_tracking_window,
                    self.window_rect.left,
                    self.window_rect.top,
                    self.window_rect.right - self.window_rect.left,
                    self.window_rect.bottom - self.window_rect.top,
                    SWP_SHOWWINDOW 
                );
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        }*/
    }

    pub fn set_pos(&mut self) {
        unsafe {
            SetWindowPos(self.m_window,
                self.m_tracking_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
            SWP_NOREDRAW | SWP_NOACTIVATE
            );
        }
    }

    pub fn set_color(&mut self, focus: bool) {
        //println!("Changing colors!");
        let mut pcr_colorization: u32 = 0;
        let mut pf_opaqueblend: BOOL = BOOL(0);
        //TODO should check whether DwmGetColorzationColor was successful or not. 
        unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) };

        let r = ((pcr_colorization & 0x00FF0000) >> 16) as f32;
        let g = ((pcr_colorization & 0x0000FF00) >> 8) as f32;
        let b = ((pcr_colorization & 0x000000FF) >> 0) as f32;

        if focus {
            self.color.r = r/255.0;
            self.color.g = g/255.0;
            self.color.b = b/255.0;
        } else {
            self.color.r = r/255.0/1.5;
            self.color.g = g/255.0/1.5;
            self.color.b = b/255.0/1.5;
        }
        self.update();
    }

    // When CreateWindowExW is called, we can optinally pass a value to its last field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the border 
    // structure, and here we are getting that pointer and assigning it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        //println!("Window Message: {:?}", message);
        if message == WM_DESTROY {
            println!("message == WM_DESTROY");
        }
        let mut this_ref: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;
        
        if this_ref == std::ptr::null_mut() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            this_ref = (*create_struct).lpCreateParams as *mut _;
            // println!("this_ref: {:?}", *this_ref);
            SetWindowLongPtrW(window, GWLP_USERDATA, this_ref as _);
        }
        match this_ref != std::ptr::null_mut() {
            true => return Self::wnd_proc(&mut *this_ref, window, message, wparam, lparam),
            false => return DefWindowProcW(window, message, wparam, lparam),
        }                                          
    }

    // TODO event_hook will send more messages than necessary if I do an action for long enough. I
    // should find a way to fix that.
    pub unsafe fn wnd_proc(&mut self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        //println!("message: {:?}", message);
        match message {
            WM_MOVE => {
                //let before = std::time::Instant::now();
                //println!("moving!");

                // Jump into another message loop with no sleep so as to maximize draw fps.
                /*let mut message = MSG::default();
                while GetMessageW(&mut message, HWND::default(), 0, 0).into() && message.message == WM_MOVE {
                    println!("Moving");
                    self.update();
                    //GetMessageW(&mut message, HWND::default(), 0, 0);
                }*/
                self.update();
                //println!("time elapsed: {:.2?}", before.elapsed());
            },
            //TODO maybe switch out WM_MOVE with WM_WINDOWPOSCHANGING because that seems like the
            //more correct way to do it. However, if I do it that way, I have to pass a WINDOWPOS
            //structure which I'm too lazy to deal with right now.
            //WM_WINDOWPOSCHANGING => { self.update() },
            //WM_WINDOWPOSCHANGED => { self.update() },
            WM_SETFOCUS => {
                //println!("Focus set: {:?}", self.m_tracking_window);
                self.set_pos();
                self.set_color(true); 
            },
            WM_KILLFOCUS => {
                //println!("Focus killed: {:?}", self.m_tracking_window);
                self.set_pos();
                self.set_color(false);
            },
            WM_DESTROY => {
                //Converting the pointer to a box seems to make the whole program exit so I think
                //it's better if I just simply set the windowlongptrw to 0 manually like Microsoft
                //does in PowerToys.
                /*let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut i32;
                // Converting to a box like below means it will automatically clean up when it goes
                // out of scope (I think). 
                println!("Converting to box.");
                let box_pointer = Box::from_raw(ptr);*/
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                //UnhookWinEvent(self.win_event_hook);
                PostQuitMessage(0);
            },
            _ => { /*std::thread::sleep(std::time::Duration::from_millis(10))*/ }
        }
        LRESULT(0)
    }
}


