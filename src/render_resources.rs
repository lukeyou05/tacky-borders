use anyhow::Context;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{FALSE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext7, ID2D1HwndRenderTarget, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice3, IDCompositionDesktopDevice, IDCompositionDevice4,
    IDCompositionTarget, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIFactory7, IDXGISurface, IDXGISwapChain1, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::config::RendererType;
use crate::{utils::get_monitor_info, APP_STATE};

#[derive(Debug, Default)]
pub struct RenderResources {
    new_renderer: Option<NewRenderer>,
    legacy_renderer: Option<LegacyRenderer>,
    // The renderer_type is actually in the Config, but it's behind a RwLock, so we'll move it here
    // so we don't have to lock the Config as often.
    pub renderer_type: RendererType,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct NewRenderer {
    d2d_context: ID2D1DeviceContext7,
    swap_chain: IDXGISwapChain1,
    d_comp_device: IDCompositionDevice4,
    d_comp_target: IDCompositionTarget,
    d_comp_visual: IDCompositionVisual2,
    // target_bitmap will always be created, but the reason I keep it as an Option is because I
    // need to temporarily drop it in update(), and setting it to None is an easy way to do that
    target_bitmap: Option<ID2D1Bitmap1>,
    border_bitmap: Option<ID2D1Bitmap1>,
    mask_bitmap: Option<ID2D1Bitmap1>,
}

#[derive(Debug)]
pub struct LegacyRenderer {
    render_target: ID2D1HwndRenderTarget,
}

impl RenderResources {
    // TODO: maybe i should just have it return the whole struct instead because that seems like
    // it'd be easier to understand for new people reading the code
    pub fn d2d_context(&self) -> anyhow::Result<&ID2D1DeviceContext7> {
        Ok(&self
            .new_renderer
            .as_ref()
            .context("d2d_context: could not get new_renderer")?
            .d2d_context)
    }

    pub fn render_target(&self) -> anyhow::Result<&ID2D1HwndRenderTarget> {
        Ok(&self
            .legacy_renderer
            .as_ref()
            .context("render_target: could not get legacy_renderer")?
            .render_target)
    }

    pub fn swap_chain(&self) -> anyhow::Result<&IDXGISwapChain1> {
        Ok(&self
            .new_renderer
            .as_ref()
            .context("swap_chain: could not get new_renderer")?
            .swap_chain)
    }

    pub fn target_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.new_renderer
            .as_ref()
            .context("target_bitmap: could not get new_renderer")?
            .target_bitmap
            .as_ref()
            .context("could not get target_bitmap")
    }

    pub fn border_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.new_renderer
            .as_ref()
            .context("border_bitmap: could not get new_renderer")?
            .border_bitmap
            .as_ref()
            .context("could not get border_bitmap")
    }

    pub fn mask_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.new_renderer
            .as_ref()
            .context("mask_bitmap: could not get new_renderer")?
            .mask_bitmap
            .as_ref()
            .context("could not get mask_bitmap")
    }

    pub fn init(
        &mut self,
        current_monitor: HMONITOR,
        border_width: i32,
        window_padding: i32,
        border_window: HWND,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        match self.renderer_type {
            RendererType::New => {
                self.new_renderer = Some(NewRenderer::try_new(
                    current_monitor,
                    border_width,
                    window_padding,
                    border_window,
                    create_extra_bitmaps,
                )?)
            }
            RendererType::Legacy => {
                self.legacy_renderer = Some(LegacyRenderer::try_new(border_window)?);
            }
        }

        Ok(())
    }

    pub fn update(
        &mut self,
        current_monitor: HMONITOR,
        border_width: i32,
        window_padding: i32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        match self.renderer_type {
            RendererType::New => {
                let Some(ref mut new_renderer) = self.new_renderer else {
                    // Theoretically this branch shouldn't be reachable, but in case it does, let's
                    // just panic because there's not much else we can do
                    panic!();
                };

                new_renderer.update(
                    current_monitor,
                    border_width,
                    window_padding,
                    create_extra_bitmaps,
                )?;
            }
            // TODO: We already update/resize the buffers in the render() function within
            // WindowBorder, but I might want to move it here instead?
            RendererType::Legacy => return Ok(()),
        }

        Ok(())
    }
}

impl NewRenderer {
    pub fn try_new(
        current_monitor: HMONITOR,
        border_width: i32,
        window_padding: i32,
        border_window: HWND,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<Self> {
        let d2d_context = unsafe {
            APP_STATE
                .d2d_device
                .read()
                .unwrap()
                .as_ref()
                .context("could not get d2d_device")?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
        }
        .context("d2d_context")?;

        unsafe { d2d_context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };

        let m_info = get_monitor_info(current_monitor).context("mi")?;
        let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
        let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: screen_width + ((border_width + window_padding) * 2) as u32,
            Height: screen_height + ((border_width + window_padding) * 2) as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: FALSE,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };

        unsafe {
            let dxgi_adapter = APP_STATE
                .dxgi_device
                .read()
                .unwrap()
                .as_ref()
                .context("could not get dxgi_device")?
                .GetAdapter()
                .context("dxgi_adapter")?;
            let dxgi_factory: IDXGIFactory7 = dxgi_adapter.GetParent().context("dxgi_factory")?;

            let swap_chain = dxgi_factory
                .CreateSwapChainForComposition(
                    APP_STATE
                        .d3d11_device
                        .read()
                        .unwrap()
                        .as_ref()
                        .context("could not get d3d11_device")?,
                    &swap_chain_desc,
                    None,
                )
                .context("swap_chain")?;

            let d_comp_desktop_device: IDCompositionDesktopDevice = DCompositionCreateDevice3(
                APP_STATE
                    .dxgi_device
                    .read()
                    .unwrap()
                    .as_ref()
                    .context("could not get dxgi_device")?,
            )?;
            let d_comp_target = d_comp_desktop_device
                .CreateTargetForHwnd(border_window, true)
                .context("d_comp_target")?;

            // This is weird, but it's the only way that works afaik. IDCompositionDevice4 doesn't
            // implement CreateTargetForHwnd, so we have to first create an intermediate device
            // like IDCompositionDevice or IDCompositionDesktopDevice, create the target, then
            // .cast() it. Sidenote, you can't create an IDCompositionDevice3 using
            // DCompositionCreateDevice3, but you can create an IDCompositionDevice4... why?
            let d_comp_device: IDCompositionDevice4 = d_comp_desktop_device
                .cast()
                .context("d_comp_desktop_device.cast()")?;

            let d_comp_visual = d_comp_device.CreateVisual().context("visual")?;

            d_comp_visual
                .SetContent(&swap_chain)
                .context("d_comp_visual.SetContent()")?;
            d_comp_target
                .SetRoot(&d_comp_visual)
                .context("d_comp_target.SetRoot()")?;
            d_comp_device.Commit().context("d_comp_device.Commit()")?;

            let (target_bitmap_opt, border_bitmap_opt, mask_bitmap_opt) = Self::create_bitmaps(
                &d2d_context,
                &swap_chain,
                screen_width,
                screen_height,
                border_width,
                window_padding,
                create_extra_bitmaps,
            )?;

            Ok(Self {
                target_bitmap: target_bitmap_opt,
                border_bitmap: border_bitmap_opt,
                mask_bitmap: mask_bitmap_opt,
                d2d_context,
                swap_chain,
                d_comp_device,
                d_comp_target,
                d_comp_visual,
            })
        }
    }

    fn create_bitmaps(
        d2d_context: &ID2D1DeviceContext7,
        swap_chain: &IDXGISwapChain1,
        screen_width: u32,
        screen_height: u32,
        border_width: i32,
        window_padding: i32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<(
        Option<ID2D1Bitmap1>,
        Option<ID2D1Bitmap1>,
        Option<ID2D1Bitmap1>,
    )> {
        let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            colorContext: ManuallyDrop::new(None),
        };

        let dxgi_back_buffer: IDXGISurface =
            unsafe { swap_chain.GetBuffer(0) }.context("dxgi_back_buffer")?;

        let target_bitmap = unsafe {
            d2d_context.CreateBitmapFromDxgiSurface(&dxgi_back_buffer, Some(&bitmap_properties))
        }
        .context("d2d_target_bitmap")?;

        unsafe { d2d_context.SetTarget(&target_bitmap) };

        let mut border_bitmap_opt = None;
        let mut mask_bitmap_opt = None;

        // If border effects are enabled, we need to create two more bitmaps
        if create_extra_bitmaps {
            // We create two bitmaps because the first (target_bitmap) cannot be used for effects
            let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                colorContext: ManuallyDrop::new(None),
            };
            border_bitmap_opt = Some(
                unsafe {
                    d2d_context.CreateBitmap(
                        D2D_SIZE_U {
                            width: screen_width + ((border_width + window_padding) * 2) as u32,
                            height: screen_height + ((border_width + window_padding) * 2) as u32,
                        },
                        None,
                        0,
                        &bitmap_properties,
                    )
                }
                .context("border_bitmap")?,
            );

            // Aaaand yet another for the mask
            let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                colorContext: ManuallyDrop::new(None),
            };
            mask_bitmap_opt = Some(
                unsafe {
                    d2d_context.CreateBitmap(
                        D2D_SIZE_U {
                            width: screen_width + ((border_width + window_padding) * 2) as u32,
                            height: screen_height + ((border_width + window_padding) * 2) as u32,
                        },
                        None,
                        0,
                        &bitmap_properties,
                    )
                }
                .context("mask_bitmap")?,
            );
        }

        Ok((Some(target_bitmap), border_bitmap_opt, mask_bitmap_opt))
    }

    // After updating resources, we also need to update anything that relies on references to the
    // resources (e.g. border effects)
    pub fn update(
        &mut self,
        current_monitor: HMONITOR,
        border_width: i32,
        window_padding: i32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        // Release buffer references
        self.target_bitmap = None;
        self.border_bitmap = None;
        self.mask_bitmap = None;

        unsafe { self.d2d_context.SetTarget(None) };

        let m_info = get_monitor_info(current_monitor).context("mi")?;
        let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
        let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

        unsafe {
            self.swap_chain.ResizeBuffers(
                2,
                screen_width + ((border_width + window_padding) * 2) as u32,
                screen_height + ((border_width + window_padding) * 2) as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG::default(),
            )
        }
        .context("swap_chain.ResizeBuffers()")?;

        // Supposedly, cloning d2d_context or swap_chain just increases the underlying object's
        // reference count, so it's not actually cloning the object itself. Unfortunately, I need
        // to do it because Rust's borrow checker is a little stupid.
        (self.target_bitmap, self.border_bitmap, self.mask_bitmap) = Self::create_bitmaps(
            &self.d2d_context,
            &self.swap_chain,
            screen_width,
            screen_height,
            border_width,
            window_padding,
            create_extra_bitmaps,
        )?;

        Ok(())
    }
}

impl LegacyRenderer {
    fn try_new(border_window: HWND) -> anyhow::Result<Self> {
        let render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_UNKNOWN,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: border_window,
            pixelSize: Default::default(),
            presentOptions: D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS | D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };

        unsafe {
            let render_target = APP_STATE.render_factory.CreateHwndRenderTarget(
                &render_target_properties,
                &hwnd_render_target_properties,
            )?;

            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            Ok(Self { render_target })
        }
    }
}
