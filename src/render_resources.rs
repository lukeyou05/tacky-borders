use anyhow::Context;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{FALSE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext7, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice3, IDCompositionDesktopDevice, IDCompositionDevice4,
    IDCompositionTarget, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIFactory7, IDXGISurface, IDXGISwapChain1, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::{utils::get_monitor_info, APP_STATE};

#[derive(Debug, Default)]
pub struct RenderResources {
    d2d_context: Option<ID2D1DeviceContext7>,
    swap_chain: Option<IDXGISwapChain1>,
    d_comp_device: Option<IDCompositionDevice4>,
    d_comp_target: Option<IDCompositionTarget>,
    d_comp_visual: Option<IDCompositionVisual2>,
    bitmaps: Bitmaps,
}

#[derive(Debug, Default)]
pub struct Bitmaps {
    target_bitmap: Option<ID2D1Bitmap1>,
    border_bitmap: Option<ID2D1Bitmap1>,
    mask_bitmap: Option<ID2D1Bitmap1>,
}

impl RenderResources {
    pub fn d2d_context(&self) -> anyhow::Result<&ID2D1DeviceContext7> {
        self.d2d_context
            .as_ref()
            .context("could not get d2d_context")
    }

    pub fn swap_chain(&self) -> anyhow::Result<&IDXGISwapChain1> {
        self.swap_chain.as_ref().context("could not get swap_chain")
    }

    pub fn target_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.bitmaps
            .target_bitmap
            .as_ref()
            .context("could not get target_bitmap")
    }

    pub fn border_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.bitmaps
            .border_bitmap
            .as_ref()
            .context("could not get border_bitmap")
    }

    pub fn mask_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.bitmaps
            .mask_bitmap
            .as_ref()
            .context("could not get mask_bitmap")
    }

    pub fn create(
        &mut self,
        current_monitor: HMONITOR,
        border_width: i32,
        window_padding: i32,
        border_window: HWND,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        let d2d_context = unsafe {
            APP_STATE
                .d2d_device
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
            let dxgi_adapter = APP_STATE.dxgi_device.GetAdapter().context("dxgi_adapter")?;
            let dxgi_factory: IDXGIFactory7 = dxgi_adapter.GetParent().context("dxgi_factory")?;

            let swap_chain = dxgi_factory
                .CreateSwapChainForComposition(&APP_STATE.device, &swap_chain_desc, None)
                .context("swap_chain")?;

            let d_comp_desktop_device: IDCompositionDesktopDevice =
                DCompositionCreateDevice3(&APP_STATE.dxgi_device)?;
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

            self.bitmaps
                .create(
                    &d2d_context,
                    &swap_chain,
                    screen_width,
                    screen_height,
                    border_width,
                    window_padding,
                    create_extra_bitmaps,
                )
                .context("could not create bitmaps")?;

            self.d2d_context = Some(d2d_context);
            self.swap_chain = Some(swap_chain);
            self.d_comp_device = Some(d_comp_device);
            self.d_comp_target = Some(d_comp_target);
            self.d_comp_visual = Some(d_comp_visual);
        }

        Ok(())
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
        self.bitmaps.target_bitmap = None;
        self.bitmaps.border_bitmap = None;
        self.bitmaps.mask_bitmap = None;

        let d2d_context = self.d2d_context()?;
        let swap_chain = self.swap_chain()?;

        unsafe { d2d_context.SetTarget(None) };

        let m_info = get_monitor_info(current_monitor).context("mi")?;
        let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
        let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

        unsafe {
            swap_chain.ResizeBuffers(
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
        self.bitmaps
            .create(
                &d2d_context.clone(),
                &swap_chain.clone(),
                screen_width,
                screen_height,
                border_width,
                window_padding,
                create_extra_bitmaps,
            )
            .context("could not create bitmaps")?;

        Ok(())
    }
}

// TODO: too many arguments warning
#[allow(clippy::too_many_arguments)]
impl Bitmaps {
    fn create(
        &mut self,
        d2d_context: &ID2D1DeviceContext7,
        swap_chain: &IDXGISwapChain1,
        screen_width: u32,
        screen_height: u32,
        border_width: i32,
        window_padding: i32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
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

        self.target_bitmap = Some(target_bitmap);

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
            let border_bitmap = unsafe {
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
            .context("border_bitmap")?;

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
            let mask_bitmap = unsafe {
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
            .context("mask_bitmap")?;

            self.border_bitmap = Some(border_bitmap);
            self.mask_bitmap = Some(mask_bitmap);
        }

        Ok(())
    }
}
