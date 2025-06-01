use anyhow::{Context, anyhow};
use std::time;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BRUSH_PROPERTIES, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_ROUNDED_RECT, ID2D1Brush,
    ID2D1Multithread, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::core::Interface;
use windows_numerics::Matrix3x2;

use crate::APP_STATE;
use crate::animations::{AnimType, Animations};
use crate::colors::ColorBrush;
use crate::effects::Effects;
use crate::render_backend::{RenderBackend, RenderBackendConfig, TARGET_BITMAP_PROPS};
use crate::utils::{PrependErr, T_E_UNINIT, ToWindowsResult};
use crate::window_border::WindowState;

#[derive(Debug, Default, Clone)]
pub struct BorderDrawer {
    pub border_width: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    // TODO: maybe get rid of render_rect; it would make sense to have the WindowBorder struct
    // calculate the coordinates for the border, and then delegate the rendering here
    pub render_rect: D2D1_ROUNDED_RECT,
    pub render_backend: RenderBackend,
    pub active_color: ColorBrush,
    pub inactive_color: ColorBrush,
    pub animations: Animations,
    pub effects: Effects,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
}

impl BorderDrawer {
    #[allow(clippy::too_many_arguments)]
    pub fn configure_appearance(
        &mut self,
        border_width: i32,
        border_offset: i32,
        border_radius: f32,
        active_color: ColorBrush,
        inactive_color: ColorBrush,
        animations: Animations,
        effects: Effects,
    ) {
        self.border_width = border_width;
        self.border_offset = border_offset;
        self.border_radius = border_radius;
        self.active_color = active_color;
        self.inactive_color = inactive_color;
        self.animations = animations;
        self.effects = effects;
    }

    pub fn init(
        &mut self,
        width: u32,
        height: u32,
        border_window: HWND,
        window_rect: &RECT,
        render_backend_config: RenderBackendConfig,
    ) -> anyhow::Result<()> {
        self.render_backend = render_backend_config
            .to_render_backend(width, height, border_window, self.effects.is_enabled())
            .context("could not initialize render backend in init()")?;

        let renderer: &ID2D1RenderTarget = match self.render_backend {
            RenderBackend::V2(ref backend) => &backend.d2d_context,
            RenderBackend::Legacy(ref backend) => &backend.render_target,
            RenderBackend::None => return Err(anyhow!("render backend is None")),
        };

        // We will adjust opacity later. For now, we set it to 0.
        let brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 0.0,
            transform: Matrix3x2::identity(),
        };
        self.active_color
            .init_brush(renderer, window_rect, &brush_properties)?;
        self.inactive_color
            .init_brush(renderer, window_rect, &brush_properties)?;

        if self.render_backend.supports_effects() {
            self.effects
                .init_command_lists_if_enabled(&self.render_backend)
                .context("could not initialize command list")?;
        }

        self.render_rect = D2D1_ROUNDED_RECT {
            rect: Default::default(),
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        Ok(())
    }

    pub fn uninit(&mut self) {
        self.render_backend = RenderBackend::None;
        let _ = self.active_color.take_brush();
        let _ = self.inactive_color.take_brush();
        let _ = self.effects.take_active_command_list();
        let _ = self.effects.take_inactive_command_list();
    }

    pub fn resize_renderer(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        self.render_backend
            .resize(width, height, self.effects.is_enabled())
            .prepend_err("could not update render resources")?;

        if self.render_backend.supports_effects() {
            self.effects
                .init_command_lists_if_enabled(&self.render_backend)
                .context("could not initialize command lists")
                .to_windows_result(T_E_UNINIT)?;
        }

        Ok(())
    }

    pub fn render(
        &mut self,
        window_rect: &RECT,
        window_padding: i32,
        window_state: WindowState,
    ) -> windows::core::Result<()> {
        self.last_render_time = Some(time::Instant::now());

        let border_width = self.border_width as f32;
        let border_offset = self.border_offset as f32;
        let window_padding = window_padding as f32;

        self.render_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: border_width / 2.0 + window_padding - border_offset,
                top: border_width / 2.0 + window_padding - border_offset,
                right: (window_rect.right - window_rect.left) as f32
                    - border_width / 2.0
                    - window_padding
                    + border_offset,
                bottom: (window_rect.bottom - window_rect.top) as f32
                    - border_width / 2.0
                    - window_padding
                    + border_offset,
            },
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        // Note that Rust's borrow checker prevents passing the render backend from the match arm,
        // so I'll need to grab it from within the respective functions instead
        match self.render_backend {
            RenderBackend::V2(_) if self.effects.should_apply(window_state) => {
                self.render_v2_with_effects(window_rect, window_state)?
            }
            RenderBackend::V2(_) => self.render_v2(window_rect, window_state)?,
            RenderBackend::Legacy(_) => self.render_legacy(window_rect, window_state)?,
            RenderBackend::None => {
                return Err(windows::core::Error::new(
                    T_E_UNINIT,
                    "render_backend is None",
                ));
            }
        }

        Ok(())
    }

    fn render_legacy(
        &mut self,
        window_rect: &RECT,
        window_state: WindowState,
    ) -> windows::core::Result<()> {
        let RenderBackend::Legacy(ref backend) = self.render_backend else {
            return Err(windows::core::Error::new(
                T_E_UNINIT,
                "could not get render_backend within render()",
            ));
        };
        let render_target = &backend.render_target;

        let pixel_size = D2D_SIZE_U {
            width: (window_rect.right - window_rect.left) as u32,
            height: (window_rect.bottom - window_rect.top) as u32,
        };

        unsafe {
            render_target.Resize(&pixel_size)?;

            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            render_target.BeginDraw();
            render_target.Clear(None);

            if bottom_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = bottom_color {
                    gradient.update_start_end_points(window_rect);
                }

                match bottom_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(render_target, id2d1_brush),
                    None => debug!("ID2D1Brush for bottom_color has not been created yet"),
                }
            }
            if top_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = top_color {
                    gradient.update_start_end_points(window_rect);
                }

                match top_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(render_target, id2d1_brush),
                    None => debug!("ID2D1Brush for top_color has not been created yet"),
                }
            }

            render_target.EndDraw(None, None)?;
        }

        Ok(())
    }

    fn render_v2(
        &mut self,
        window_rect: &RECT,
        window_state: WindowState,
    ) -> windows::core::Result<()> {
        let RenderBackend::V2(ref backend) = self.render_backend else {
            return Err(windows::core::Error::new(
                T_E_UNINIT,
                "could not get render_backend within render()",
            ));
        };
        let d2d_context = &backend.d2d_context;

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            // We're about to use DirectComposition which means we will be using the underlying
            // Direct3D objects without Direct2D's knowledge. To avoid resource access conflict, we
            // must explicitly acquire a lock. Read the following article for more info:
            // https://learn.microsoft.com/en-us/windows/win32/direct2d/multi-threaded-direct2d-apps
            let d2d_multithread: ID2D1Multithread = APP_STATE
                .render_factory
                .cast()
                .prepend_err("d2d_multithread")?;
            d2d_multithread.Enter();

            // Set d2d_context's target back to the target_bitmap so we can draw to the display
            let mut point = POINT::default();
            let dxgi_surface: IDXGISurface = backend
                .d_comp_surface
                .BeginDraw(None, &mut point)
                .prepend_err("dxgi_surface")?;
            let target_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&TARGET_BITMAP_PROPS))
                .prepend_err("target_bitmap")?;
            d2d_context.SetTarget(&target_bitmap);

            // Draw to the target_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            if bottom_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = bottom_color {
                    gradient.update_start_end_points(window_rect);
                }

                match bottom_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(d2d_context, id2d1_brush),
                    None => debug!("ID2D1Brush for bottom_color has not been created yet"),
                }
            }
            if top_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = top_color {
                    gradient.update_start_end_points(window_rect);
                }

                match top_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(d2d_context, id2d1_brush),
                    None => debug!("ID2D1Brush for top_color has not been created yet"),
                }
            }

            d2d_context.EndDraw(None, None)?;

            d2d_context.SetTarget(None);
            backend
                .d_comp_surface
                .EndDraw()
                .prepend_err("d_comp_surface.EndDraw()")?;
            backend
                .d_comp_device
                .Commit()
                .prepend_err("d_comp_device.Commit()")?;

            d2d_multithread.Leave();
        }

        Ok(())
    }

    fn render_v2_with_effects(
        &mut self,
        window_rect: &RECT,
        window_state: WindowState,
    ) -> windows::core::Result<()> {
        let RenderBackend::V2(ref backend) = self.render_backend else {
            return Err(windows::core::Error::new(
                T_E_UNINIT,
                "could not get render_backend within render()",
            ));
        };
        let d2d_context = &backend.d2d_context;

        let border_width = self.border_width as f32;

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match window_state {
                WindowState::Active => (&self.inactive_color, &self.active_color),
                WindowState::Inactive => (&self.active_color, &self.inactive_color),
            };

            // Create a rect that covers up to the outer edge of the border
            let render_rect_adjusted = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: self.render_rect.rect.left - (border_width / 2.0),
                    top: self.render_rect.rect.top - (border_width / 2.0),
                    right: self.render_rect.rect.right + (border_width / 2.0),
                    bottom: self.render_rect.rect.bottom + (border_width / 2.0),
                },
                radiusX: self.border_radius + (border_width / 2.0),
                radiusY: self.border_radius + (border_width / 2.0),
            };

            // Set the d2d_context target to the border_bitmap
            let border_bitmap = backend
                .border_bitmap
                .as_ref()
                .context("could not get border_bitmap")
                .to_windows_result(T_E_UNINIT)?;
            d2d_context.SetTarget(border_bitmap);

            // Draw to the border_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            // We use filled rectangles here because it helps make the effects more visible.
            // Additionally, if someone sets the border width to 0, the effects will still be
            // visible (whereas they wouldn't be if we used a hollow rectangle).
            if bottom_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = bottom_color {
                    gradient.update_start_end_points(window_rect);
                }

                match bottom_color.get_brush() {
                    Some(id2d1_brush) => {
                        self.fill_rectangle(&render_rect_adjusted, d2d_context, id2d1_brush)
                    }
                    None => debug!("ID2D1Brush for bottom_color has not been created yet"),
                }
            }
            if top_color.get_opacity().to_windows_result(T_E_UNINIT)? > 0.0 {
                if let ColorBrush::Gradient(gradient) = top_color {
                    gradient.update_start_end_points(window_rect);
                }

                match top_color.get_brush() {
                    Some(id2d1_brush) => {
                        self.fill_rectangle(&render_rect_adjusted, d2d_context, id2d1_brush)
                    }
                    None => debug!("ID2D1Brush for top_color has not been created yet"),
                }
            }

            d2d_context.EndDraw(None, None)?;
        }

        unsafe {
            // Set the d2d_context target to the mask_bitmap to create an alpha mask
            let mask_bitmap = backend
                .mask_bitmap
                .as_ref()
                .context("could not get mask_bitmap")
                .to_windows_result(T_E_UNINIT)?;
            d2d_context.SetTarget(mask_bitmap);

            // Create a rect that covers up to the inner edge of the border
            // This rect is used to mask out the inner portion of the border
            // Note this is different from the earlier render_rect_adjusted
            let render_rect_adjusted = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: self.render_rect.rect.left + (border_width / 2.0),
                    top: self.render_rect.rect.top + (border_width / 2.0),
                    right: self.render_rect.rect.right - (border_width / 2.0),
                    bottom: self.render_rect.rect.bottom - (border_width / 2.0),
                },
                radiusX: self.border_radius - (border_width / 2.0),
                radiusY: self.border_radius - (border_width / 2.0),
            };

            // Create a 100% opaque brush because our active/inactive colors' brushes might not be
            let opaque_brush = d2d_context.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?;

            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            self.fill_rectangle(&render_rect_adjusted, d2d_context, &opaque_brush);

            d2d_context.EndDraw(None, None)?;
        }

        unsafe {
            // We're about to use DirectComposition which means we will be using the underlying
            // Direct3D objects without Direct2D's knowledge. To avoid resource access conflict, we
            // must explicitly acquire a lock. Read the following article for more info:
            // https://learn.microsoft.com/en-us/windows/win32/direct2d/multi-threaded-direct2d-apps
            let d2d_multithread: ID2D1Multithread = APP_STATE
                .render_factory
                .cast()
                .prepend_err("d2d_multithread")?;
            d2d_multithread.Enter();

            // Set d2d_context's target back to the target_bitmap so we can draw to the display
            let mut point = POINT::default();
            let dxgi_surface: IDXGISurface = backend
                .d_comp_surface
                .BeginDraw(None, &mut point)
                .prepend_err("dxgi_surface")?;
            let target_bitmap = d2d_context
                .CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&TARGET_BITMAP_PROPS))
                .prepend_err("target_bitmap")?;
            d2d_context.SetTarget(&target_bitmap);

            // Retrieve our command list (includes border_bitmap, mask_bitmap, and effects)
            let command_list = self
                .effects
                .get_current_command_list(window_state)
                .to_windows_result(T_E_UNINIT)?;

            // Draw to the target_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            d2d_context.DrawImage(
                command_list,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );

            d2d_context.EndDraw(None, None)?;

            d2d_context.SetTarget(None);
            backend
                .d_comp_surface
                .EndDraw()
                .prepend_err("d_comp_surface.EndDraw()")?;
            backend
                .d_comp_device
                .Commit()
                .prepend_err("d_comp_device.Commit()")?;

            d2d_multithread.Leave();
        }

        Ok(())
    }

    // NOTE: ID2D1DeviceContext implements From<&ID2D1DeviceContext> for &ID2D1RenderTarget
    fn draw_rectangle(&self, renderer: &ID2D1RenderTarget, brush: &ID2D1Brush) {
        unsafe {
            match self.border_radius {
                0.0 => renderer.DrawRectangle(
                    &self.render_rect.rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
                _ => renderer.DrawRoundedRectangle(
                    &self.render_rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
            }
        }
    }

    // NOTE: ID2D1DeviceContext implements From<&ID2D1DeviceContext> for &ID2D1RenderTarget
    fn fill_rectangle(
        &self,
        render_rect: &D2D1_ROUNDED_RECT,
        renderer: &ID2D1RenderTarget,
        brush: &ID2D1Brush,
    ) {
        unsafe {
            match self.border_radius {
                0.0 => renderer.FillRectangle(&render_rect.rect, brush),
                _ => renderer.FillRoundedRectangle(render_rect, brush),
            }
        }
    }

    pub fn set_anims_timer_if_enabled(&mut self, border_window: HWND) {
        self.animations
            .set_timer_if_enabled(border_window, &mut self.last_anim_time);
    }

    pub fn destroy_anims_timer(&mut self) {
        self.animations.destroy_timer();
    }

    pub fn animate(
        &mut self,
        window_rect: &RECT,
        window_padding: i32,
        window_state: WindowState,
    ) -> anyhow::Result<()> {
        let anim_elapsed = self
            .last_anim_time
            .get_or_insert_with(time::Instant::now)
            .elapsed();
        let render_elapsed = self
            .last_render_time
            .get_or_insert_with(time::Instant::now)
            .elapsed();

        let mut update = false;

        for anim_params in self.animations.get_current(window_state).clone().iter() {
            match anim_params.anim_type {
                AnimType::Spiral | AnimType::ReverseSpiral => {
                    self.animations.animate_spiral(
                        window_rect,
                        &self.active_color,
                        &self.inactive_color,
                        &anim_elapsed,
                        anim_params,
                    );
                    update = true;
                }
                AnimType::Fade => {
                    let correct_active_opacity = if window_state == WindowState::Active {
                        1.0
                    } else {
                        0.0
                    };

                    if self.active_color.get_opacity()? != correct_active_opacity
                        || self.inactive_color.get_opacity()? != 1.0 - correct_active_opacity
                    {
                        self.animations.animate_fade(
                            window_state,
                            &self.active_color,
                            &self.inactive_color,
                            &anim_elapsed,
                            anim_params,
                        )?;
                        update = true;
                    }
                }
            }
        }

        self.last_anim_time = Some(time::Instant::now());

        let render_interval = 1.0 / self.animations.fps as f32;
        let time_diff = render_elapsed.as_secs_f32() - render_interval;
        if update && (time_diff.abs() <= 0.001 || time_diff >= 0.0) {
            self.render(window_rect, window_padding, window_state)?;
        }

        Ok(())
    }
}
