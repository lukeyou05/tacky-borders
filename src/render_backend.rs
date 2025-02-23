use anyhow::{anyhow, Context};
use serde::Deserialize;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::{FALSE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext4, ID2D1HwndRenderTarget, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice3, IDCompositionDesktopDevice, IDCompositionDevice3,
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

use crate::APP_STATE;

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq)]
pub enum RenderBackendConfig {
    #[default]
    V2,
    Legacy,
}

#[derive(Debug, Default)]
pub enum RenderBackend {
    V2(V2RenderBackend),
    Legacy(LegacyRenderBackend),
    #[default]
    None,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct V2RenderBackend {
    pub d2d_context: ID2D1DeviceContext4,
    pub swap_chain: IDXGISwapChain1,
    pub d_comp_device: IDCompositionDevice3,
    pub d_comp_target: IDCompositionTarget,
    pub d_comp_visual: IDCompositionVisual2,
    // target_bitmap will always be created, but the reason I keep it as an Option is because I
    // need to temporarily drop it in update(), and setting it to None is an easy way to do that
    pub target_bitmap: Option<ID2D1Bitmap1>,
    pub border_bitmap: Option<ID2D1Bitmap1>,
    pub mask_bitmap: Option<ID2D1Bitmap1>,
}

#[derive(Debug)]
pub struct LegacyRenderBackend {
    pub render_target: ID2D1HwndRenderTarget,
}

impl RenderBackendConfig {
    pub fn to_render_backend(
        self,
        width: u32,
        height: u32,
        border_window: HWND,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<RenderBackend> {
        match self {
            RenderBackendConfig::V2 => Ok(RenderBackend::V2(V2RenderBackend::new(
                width,
                height,
                border_window,
                create_extra_bitmaps,
            )?)),
            RenderBackendConfig::Legacy => Ok(RenderBackend::Legacy(LegacyRenderBackend::new(
                border_window,
            )?)),
        }
    }
}

impl RenderBackend {
    pub fn update(
        &mut self,
        width: u32,
        height: u32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        match self {
            RenderBackend::V2(backend) => {
                backend.update(width, height, create_extra_bitmaps)?;
            }
            // TODO: We already update/resize the buffers in the render() function within
            // BorderDrawer, but I might want to move it here instead?
            RenderBackend::Legacy(_) => return Ok(()),
            RenderBackend::None => return Err(anyhow!("render backend is None")),
        }

        Ok(())
    }

    pub fn get_pixel_size(&self) -> anyhow::Result<D2D_SIZE_U> {
        match self {
            RenderBackend::V2(backend) => {
                let swap_chain = &backend.swap_chain;
                let swap_chain_desc = unsafe { swap_chain.GetDesc1() }?;

                Ok(D2D_SIZE_U {
                    width: swap_chain_desc.Width,
                    height: swap_chain_desc.Height,
                })
            }
            RenderBackend::Legacy(backend) => {
                let pixel_size = unsafe { backend.render_target.GetPixelSize() };

                Ok(pixel_size)
            }
            RenderBackend::None => Err(anyhow!("render backend is None")),
        }
    }

    pub fn supports_effects(&self) -> bool {
        !matches!(self, RenderBackend::Legacy(_) | RenderBackend::None)
    }
}

impl V2RenderBackend {
    pub fn new(
        width: u32,
        height: u32,
        border_window: HWND,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<Self> {
        let directx_devices_opt = APP_STATE.directx_devices.read().unwrap();
        let directx_devices = directx_devices_opt
            .as_ref()
            .context("could not get direct_devices")?;

        let d2d_context = unsafe {
            directx_devices
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
        }
        .context("d2d_context")?;

        unsafe { d2d_context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
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
            let dxgi_adapter = directx_devices
                .dxgi_device
                .GetAdapter()
                .context("dxgi_adapter")?;
            let dxgi_factory: IDXGIFactory7 = dxgi_adapter.GetParent().context("dxgi_factory")?;

            let swap_chain = dxgi_factory
                .CreateSwapChainForComposition(
                    &directx_devices.d3d11_device,
                    &swap_chain_desc,
                    None,
                )
                .context("swap_chain")?;

            let d_comp_desktop_device: IDCompositionDesktopDevice =
                DCompositionCreateDevice3(&directx_devices.dxgi_device)?;
            let d_comp_target = d_comp_desktop_device
                .CreateTargetForHwnd(border_window, true)
                .context("d_comp_target")?;

            // This is weird, but it's the only way that works afaik. IDCompositionDevice4 doesn't
            // implement CreateTargetForHwnd, so we have to first create an intermediate device
            // like IDCompositionDevice or IDCompositionDesktopDevice, create the target, then
            // .cast() it. Sidenote, you can't create an IDCompositionDevice3 using
            // DCompositionCreateDevice3, but you can create an IDCompositionDevice4... why?
            let d_comp_device: IDCompositionDevice3 = d_comp_desktop_device
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
                width,
                height,
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
        d2d_context: &ID2D1DeviceContext4,
        swap_chain: &IDXGISwapChain1,
        width: u32,
        height: u32,
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
                        D2D_SIZE_U { width, height },
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
                        D2D_SIZE_U { width, height },
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
        width: u32,
        height: u32,
        create_extra_bitmaps: bool,
    ) -> anyhow::Result<()> {
        // Release buffer references
        self.target_bitmap = None;
        self.border_bitmap = None;
        self.mask_bitmap = None;

        unsafe { self.d2d_context.SetTarget(None) };

        unsafe {
            self.swap_chain.ResizeBuffers(
                2,
                width,
                height,
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
            width,
            height,
            create_extra_bitmaps,
        )?;

        Ok(())
    }
}

impl LegacyRenderBackend {
    fn new(border_window: HWND) -> anyhow::Result<Self> {
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
