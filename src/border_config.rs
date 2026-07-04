use windows::Win32::Foundation::HWND;

use crate::animations::AnimationsConfig;
use crate::border_drawer::BorderDrawer;
use crate::colors::ColorBrushConfig;
use crate::config::{Global, OffsetConfig, RadiusConfig, WidthConfig, WindowRule, ZOrderMode};
use crate::effects::EffectsConfig;
use crate::render_backend::RenderBackendConfig;

/// Resolved border parameters built from the config.yaml
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BorderConfig {
    pub render_backend: RenderBackendConfig,
    pub width: WidthConfig,
    pub offset: OffsetConfig,
    pub radius: RadiusConfig,
    pub z_order: ZOrderMode,
    pub follow_native_border: bool,
    pub active_color: ColorBrushConfig,
    pub inactive_color: ColorBrushConfig,
    pub animations: AnimationsConfig,
    pub effects: EffectsConfig,
    pub initialize_delay: u64,
    pub unminimize_delay: u64,
}

impl BorderConfig {
    pub fn resolve(
        window_rule: &WindowRule,
        global: &Global,
        render_backend: RenderBackendConfig,
        is_initial_window: bool,
    ) -> Self {
        Self {
            render_backend,
            width: window_rule.border_width.unwrap_or(global.border_width),
            offset: window_rule.border_offset.unwrap_or(global.border_offset),
            radius: window_rule.border_radius.unwrap_or(global.border_radius),
            z_order: window_rule.border_z_order.unwrap_or(global.border_z_order),
            follow_native_border: window_rule
                .follow_native_border
                .unwrap_or(global.follow_native_border),
            active_color: window_rule
                .active_color
                .clone()
                .unwrap_or_else(|| global.active_color.clone()),
            inactive_color: window_rule
                .inactive_color
                .clone()
                .unwrap_or_else(|| global.inactive_color.clone()),
            animations: window_rule
                .animations
                .clone()
                .unwrap_or_else(|| global.animations.clone()),
            effects: window_rule
                .effects
                .clone()
                .unwrap_or_else(|| global.effects.clone()),

            // If the tracking window is part of the initial windows list (meaning it was already
            // open when tacky-borders was launched), then there should be no initialize delay.
            initialize_delay: if is_initial_window {
                0
            } else {
                window_rule
                    .initialize_delay
                    .unwrap_or(global.initialize_delay)
            },
            unminimize_delay: window_rule
                .unminimize_delay
                .unwrap_or(global.unminimize_delay),
        }
    }

    pub fn width_at(&self, dpi: u32) -> i32 {
        self.width.to_width(dpi as f32)
    }

    pub fn offset_at(&self, dpi: u32) -> i32 {
        self.offset.to_offset(dpi as f32)
    }

    pub fn radius_at(&self, border_width: i32, dpi: u32, tracking_window: HWND) -> f32 {
        self.radius.to_radius(border_width, dpi, tracking_window)
    }

    pub fn is_radius_auto(&self) -> bool {
        // Custom(-1.0) is also considered Auto for backwards compatibility reasons
        matches!(self.radius, RadiusConfig::Auto | RadiusConfig::Custom(-1.0))
    }

    pub fn apply_appearance(&self, drawer: &mut BorderDrawer, dpi: u32, tracking_window: HWND) {
        let border_width = self.width_at(dpi);
        let border_offset = self.offset_at(dpi);
        let border_radius = self.radius_at(border_width, dpi, tracking_window);

        drawer.configure_appearance(
            border_width,
            border_offset,
            border_radius,
            self.active_color.to_color_brush(true),
            self.inactive_color.to_color_brush(false),
            self.animations.to_animations(),
            self.effects.to_effects(),
        );
    }

    /// This padding is used to adjust the border window such that the border and its effects
    /// don't get clipped.
    pub fn window_padding(&self, drawer: &BorderDrawer, border_offset: i32) -> i32 {
        // Effects are not supported by the Legacy render backend, so we'll just ignore them
        // in that case.
        match self.render_backend {
            RenderBackendConfig::V2 => {
                let max_active_padding = drawer
                    .effects
                    .active
                    .iter()
                    .map(|params| params.required_padding())
                    .max()
                    .unwrap_or(0);
                let max_inactive_padding = drawer
                    .effects
                    .inactive
                    .iter()
                    .map(|params| params.required_padding())
                    .max()
                    .unwrap_or(0);

                i32::max(max_active_padding, max_inactive_padding) + border_offset
            }
            RenderBackendConfig::Legacy => border_offset,
        }
    }

    // TODO: Remove border_offset "dependency" from window_padding
    pub fn adjust_padding_for_offset_change(
        window_padding: i32,
        old_offset: i32,
        new_offset: i32,
    ) -> i32 {
        window_padding - old_offset + new_offset
    }
}
