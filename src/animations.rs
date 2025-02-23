use serde::Deserialize;
use std::sync::Arc;
use std::time;
use windows::Win32::Foundation::{HWND, RECT};

use windows::Foundation::Numerics::Matrix3x2;

use crate::anim_timer::AnimationTimer;
use crate::colors::ColorBrush;
use crate::config::{serde_default_bool, serde_default_i32};
use crate::utils::cubic_bezier;
use crate::window_border::WindowState;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnimationsConfig {
    #[serde(default)]
    active: Vec<AnimParamsConfig>,
    #[serde(default)]
    inactive: Vec<AnimParamsConfig>,
    #[serde(default = "serde_default_i32::<60>")]
    fps: i32,
    #[serde(default = "serde_default_bool::<true>")]
    enabled: bool,
}

impl AnimationsConfig {
    pub fn to_animations(&self) -> Animations {
        if self.enabled {
            Animations {
                active: self
                    .active
                    .iter()
                    .map(|params_config| params_config.to_anim_params())
                    .collect(),
                inactive: self
                    .inactive
                    .iter()
                    .map(|params_config| params_config.to_anim_params())
                    .collect(),
                fps: self.fps,
                ..Default::default()
            }
        } else {
            Animations::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct Animations {
    pub active: Vec<AnimParams>,
    pub inactive: Vec<AnimParams>,
    pub timer: Option<AnimationTimer>,
    pub fps: i32,
    pub fade_progress: f32,
    pub fade_to_visible: bool,
    pub should_fade: bool,
    pub spiral_progress: f32,
    pub spiral_angle: f32,
}

impl Animations {
    pub fn animate_spiral(
        &mut self,
        window_rect: &RECT,
        active_color: &ColorBrush,
        inactive_color: &ColorBrush,
        anim_elapsed: &time::Duration,
        anim_params: &AnimParams,
        reverse: bool,
    ) {
        let direction = match reverse {
            true => -1.0,
            false => 1.0,
        };

        let delta_x = anim_elapsed.as_secs_f32() * 1000.0 / anim_params.duration * direction;
        self.spiral_progress += delta_x;

        if !(0.0..=1.0).contains(&self.spiral_progress) {
            self.spiral_progress = self.spiral_progress.rem_euclid(1.0);
        }

        let y_coord = anim_params.easing_fn.as_ref()(self.spiral_progress);

        self.spiral_angle = 360.0 * y_coord;

        // Calculate the center point of the window
        let center_x = (window_rect.right - window_rect.left) / 2;
        let center_y = (window_rect.bottom - window_rect.top) / 2;

        let transform = Matrix3x2::rotation(self.spiral_angle, center_x as f32, center_y as f32);

        active_color.set_transform(&transform);
        inactive_color.set_transform(&transform);
    }

    pub fn animate_fade(
        &mut self,
        window_state: WindowState,
        active_color: &ColorBrush,
        inactive_color: &ColorBrush,
        anim_elapsed: &time::Duration,
        anim_params: &AnimParams,
    ) {
        // If both are 0, that means the window has been opened for the first time or has been
        // unminimized. If that is the case, only one of the colors should be visible while fading.
        if active_color.get_opacity() == Some(0.0) && inactive_color.get_opacity() == Some(0.0) {
            // Set fade_progress here so we start from 0 opacity for the visible color
            self.fade_progress = match window_state {
                WindowState::Active => 0.0,
                WindowState::Inactive => 1.0,
            };

            self.fade_to_visible = true;
        }

        // Determine which direction we should move fade_progress
        let direction = match window_state {
            WindowState::Active => 1.0,
            WindowState::Inactive => -1.0,
        };

        let delta_x = anim_elapsed.as_secs_f32() * 1000.0 / anim_params.duration * direction;
        self.fade_progress += delta_x;

        // Check if the fade animation is finished
        if !(0.0..=1.0).contains(&self.fade_progress) {
            let final_opacity = self.fade_progress.clamp(0.0, 1.0);

            active_color.set_opacity(final_opacity);
            inactive_color.set_opacity(1.0 - final_opacity);

            self.fade_progress = final_opacity;
            self.fade_to_visible = false;
            self.should_fade = false;
            return;
        }

        let y_coord = anim_params.easing_fn.as_ref()(self.fade_progress);

        let (new_active_opacity, new_inactive_opacity) = match self.fade_to_visible {
            true => match window_state {
                WindowState::Active => (y_coord, 0.0),
                WindowState::Inactive => (0.0, 1.0 - y_coord),
            },
            false => (y_coord, 1.0 - y_coord),
        };

        active_color.set_opacity(new_active_opacity);
        inactive_color.set_opacity(new_inactive_opacity);
    }

    pub fn get_current(&self, window_state: WindowState) -> &Vec<AnimParams> {
        match window_state {
            WindowState::Active => &self.active,
            WindowState::Inactive => &self.inactive,
        }
    }

    pub fn set_timer_if_enabled(
        &mut self,
        border_window: HWND,
        last_anim_time: &mut Option<time::Instant>,
    ) {
        if (!self.active.is_empty() || !self.inactive.is_empty()) && self.timer.is_none() {
            let timer_duration = (1000.0 / self.fps as f32) as u64;
            self.timer = Some(AnimationTimer::start(border_window, timer_duration));

            *last_anim_time = Some(time::Instant::now());
        }
    }

    pub fn destroy_timer(&mut self) {
        if let Some(anim_timer) = self.timer.as_mut() {
            anim_timer.stop();
            self.timer = None;
        }
    }

    pub fn update_fade_progress(&mut self, window_state: WindowState) {
        self.fade_progress = match window_state {
            WindowState::Active => 1.0,
            WindowState::Inactive => 0.0,
        };
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnimParamsConfig {
    #[serde(rename = "type")]
    pub anim_type: AnimType,
    pub duration: Option<f32>,
    pub easing: Option<AnimEasing>,
}

impl AnimParamsConfig {
    fn to_anim_params(&self) -> AnimParams {
        let duration = self.duration.unwrap_or(match self.anim_type {
            AnimType::Spiral | AnimType::ReverseSpiral => 1800.0,
            AnimType::Fade => 200.0,
        });

        let easing = self.easing.unwrap_or_default();
        let easing_function = cubic_bezier(&easing.to_points()).unwrap();

        AnimParams {
            anim_type: self.anim_type,
            duration,
            easing_fn: Arc::new(easing_function),
        }
    }
}

#[derive(Clone)]
pub struct AnimParams {
    pub anim_type: AnimType,
    pub duration: f32,
    pub easing_fn: Arc<dyn Fn(f32) -> f32 + Send + Sync>,
}

// We must manually implement Debug for AnimParams because Fn(f32) -> f32 doesn't implement it
impl std::fmt::Debug for AnimParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimParams")
            .field("type", &self.anim_type)
            .field("duration", &self.duration)
            .field("easing_fn", &Arc::as_ptr(&self.easing_fn))
            .finish()
    }
}

pub trait AnimVec {
    fn contains_type(&self, anim_type: AnimType) -> bool;
}

impl AnimVec for Vec<AnimParams> {
    fn contains_type(&self, anim_type: AnimType) -> bool {
        self.iter()
            .any(|anim_params| anim_params.anim_type == anim_type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AnimType {
    Spiral,
    ReverseSpiral,
    Fade,
}

// Thanks to 0xJWLabs for the AnimEasing enum along with its methods
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq)]
pub enum AnimEasing {
    // Linear
    #[default]
    Linear,

    // EaseIn variants
    EaseIn,
    EaseInSine,
    EaseInQuad,
    EaseInCubic,
    EaseInQuart,
    EaseInQuint,
    EaseInExpo,
    EaseInCirc,
    EaseInBack,

    // EaseOut variants
    EaseOut,
    EaseOutSine,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutQuart,
    EaseOutQuint,
    EaseOutExpo,
    EaseOutCirc,
    EaseOutBack,

    // EaseInOut variants
    EaseInOut,
    EaseInOutSine,
    EaseInOutQuad,
    EaseInOutCubic,
    EaseInOutQuart,
    EaseInOutQuint,
    EaseInOutExpo,
    EaseInOutCirc,
    EaseInOutBack,

    #[serde(untagged)]
    CubicBezier([f32; 4]),
}

impl AnimEasing {
    /// Converts the easing to a corresponding array of points.
    /// Linear and named easing variants will return predefined control points,
    /// while CubicBezier returns its own array.
    pub fn to_points(self) -> [f32; 4] {
        match self {
            // Linear
            AnimEasing::Linear => [0.0, 0.0, 1.0, 1.0],

            // EaseIn variants
            AnimEasing::EaseIn => [0.42, 0.0, 1.0, 1.0],
            AnimEasing::EaseInSine => [0.12, 0.0, 0.39, 0.0],
            AnimEasing::EaseInQuad => [0.11, 0.0, 0.5, 0.0],
            AnimEasing::EaseInCubic => [0.32, 0.0, 0.67, 0.0],
            AnimEasing::EaseInQuart => [0.5, 0.0, 0.75, 0.0],
            AnimEasing::EaseInQuint => [0.64, 0.0, 0.78, 0.0],
            AnimEasing::EaseInExpo => [0.7, 0.0, 0.84, 0.0],
            AnimEasing::EaseInCirc => [0.55, 0.0, 1.0, 0.45],
            AnimEasing::EaseInBack => [0.36, 0.0, 0.66, -0.56],

            // EaseOut variants
            AnimEasing::EaseOut => [0.0, 0.0, 0.58, 1.0],
            AnimEasing::EaseOutSine => [0.61, 1.0, 0.88, 1.0],
            AnimEasing::EaseOutQuad => [0.5, 1.0, 0.89, 1.0],
            AnimEasing::EaseOutCubic => [0.33, 1.0, 0.68, 1.0],
            AnimEasing::EaseOutQuart => [0.25, 1.0, 0.5, 1.0],
            AnimEasing::EaseOutQuint => [0.22, 1.0, 0.36, 1.0],
            AnimEasing::EaseOutExpo => [0.16, 1.0, 0.3, 1.0],
            AnimEasing::EaseOutCirc => [0.0, 0.55, 0.45, 1.0],
            AnimEasing::EaseOutBack => [0.34, 1.56, 0.64, 1.0],

            // EaseInOut variants
            AnimEasing::EaseInOut => [0.42, 0.0, 0.58, 1.0],
            AnimEasing::EaseInOutSine => [0.37, 0.0, 0.63, 1.0],
            AnimEasing::EaseInOutQuad => [0.45, 0.0, 0.55, 1.0],
            AnimEasing::EaseInOutCubic => [0.65, 0.0, 0.35, 1.0],
            AnimEasing::EaseInOutQuart => [0.76, 0.0, 0.24, 1.0],
            AnimEasing::EaseInOutQuint => [0.83, 0.0, 0.17, 1.0],
            AnimEasing::EaseInOutExpo => [0.87, 0.0, 0.13, 1.0],
            AnimEasing::EaseInOutCirc => [0.85, 0.0, 0.15, 1.0],
            AnimEasing::EaseInOutBack => [0.68, -0.6, 0.32, 1.6],

            // CubicBezier variant returns its own points.
            AnimEasing::CubicBezier(bezier) => bezier,
        }
    }
}
