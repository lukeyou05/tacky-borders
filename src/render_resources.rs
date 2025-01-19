use anyhow::Context;
use windows::Win32::Graphics::{
    Direct2D::{ID2D1Bitmap1, ID2D1DeviceContext7},
    DirectComposition::{IDCompositionDevice, IDCompositionTarget, IDCompositionVisual},
    Dxgi::IDXGISwapChain1,
};

#[derive(Debug, Default)]
pub struct RenderResources {
    pub d2d_context: Option<ID2D1DeviceContext7>,
    pub swap_chain: Option<IDXGISwapChain1>,
    pub target_bitmap: Option<ID2D1Bitmap1>,
    pub border_bitmap: Option<ID2D1Bitmap1>,
    pub mask_bitmap: Option<ID2D1Bitmap1>,
    pub mask_helper_bitmap: Option<ID2D1Bitmap1>,
    pub d_comp_device: Option<IDCompositionDevice>,
    pub d_comp_target: Option<IDCompositionTarget>,
    pub d_comp_visual: Option<IDCompositionVisual>,
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
        self.target_bitmap
            .as_ref()
            .context("could not get target_bitmap")
    }

    pub fn border_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.border_bitmap
            .as_ref()
            .context("could not get border_bitmap")
    }

    pub fn mask_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.mask_bitmap
            .as_ref()
            .context("could not get mask_bitmap")
    }

    pub fn mask_helper_bitmap(&self) -> anyhow::Result<&ID2D1Bitmap1> {
        self.mask_helper_bitmap
            .as_ref()
            .context("could not get mask_help_bitmap")
    }
}
