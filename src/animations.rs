use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::time;

use windows::Foundation::Numerics::Matrix3x2;

use crate::utils::cubic_bezier;
use crate::window_border::WindowBorder;

pub const ANIM_NONE: i32 = 0;
pub const ANIM_FADE: i32 = 1;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Animations {
    #[serde(default, deserialize_with = "animation")]
    pub active: HashMap<AnimType, AnimParams>,
    #[serde(default, deserialize_with = "animation")]
    pub inactive: HashMap<AnimType, AnimParams>,
    #[serde(skip)]
    pub current: HashMap<AnimType, AnimParams>,
    #[serde(default = "default_fps")]
    pub fps: i32,
    #[serde(skip)]
    pub fade_progress: f32,
    #[serde(skip)]
    pub fade_to_visible: bool,
    #[serde(skip)]
    pub spiral_angle: f32,
}

fn default_fps() -> i32 {
    60
}

// Custom deserializer for HashMap<AnimationType, Option<AnimValues>>
fn animation<'de, D>(deserializer: D) -> Result<HashMap<AnimType, AnimParams>, D::Error>
where
    D: Deserializer<'de>,
{
    let deserialized = HashMap::<AnimType, serde_yaml::Value>::deserialize(deserializer)?;
    let mut hashmap = HashMap::new();

    for (anim_type, value) in deserialized {
        let duration = match value.get("duration") {
            Some(val) => serde_yaml::from_value(val.clone())
                .map_err(|e| serde::de::Error::custom(format!("{e}")))?,
            None => match anim_type {
                AnimType::Spiral | AnimType::ReverseSpiral => 1800.0,
                AnimType::Fade => 200.0,
            },
        };
        let easing = match value.get("easing") {
            Some(val) => serde_yaml::from_value(val.clone())
                .map_err(|e| serde::de::Error::custom(format!("{e}")))?,
            None => AnimEasing::default(),
        };

        let anim_params = AnimParams {
            duration,
            easing: easing.to_points(),
        };

        hashmap.insert(anim_type, anim_params);
    }

    Ok(hashmap)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub enum AnimType {
    Spiral,
    ReverseSpiral,
    Fade,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnimParams {
    pub duration: f32,
    // 'easing' is specified in the config.yaml as one of the values from the AnimEasing enum, but
    // we immediately convert it to points here to avoid unnecessarily processing it later
    pub easing: [f32; 4],
}

// Thanks to 0xJWLabs for the AnimEasing enum along with its methods
#[derive(Debug, Default, Deserialize)]
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
    pub fn to_points(&self) -> [f32; 4] {
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
            AnimEasing::CubicBezier(bezier) => *bezier,
        }
    }
}

pub fn animate_spiral(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_duration: f32,
) {
    // We do 1000.0 / anim_duration because anim_duration is in miliseconds, and anim_elapsed is in
    // seconds, so if we multiply those three numbers together, the units work out or smth idk
    //
    // TODO i could pre-calculate 360.0 * 1000.0 / anim_duration so we don't have to keep
    // performing that calculation everytime this function is run, but it isn't that big of a deal
    border.animations.spiral_angle += 360.0 * anim_elapsed.as_secs_f32() * 1000.0 / anim_duration;

    if border.animations.spiral_angle.abs() >= 360.0 {
        border.animations.spiral_angle %= 360.0;
    }

    // Calculate the center point of the window
    let center_x = (border.window_rect.right - border.window_rect.left) / 2;
    let center_y = (border.window_rect.bottom - border.window_rect.top) / 2;

    border.brush_properties.transform = Matrix3x2::rotation(
        border.animations.spiral_angle,
        center_x as f32,
        center_y as f32,
    );
}

pub fn animate_fade(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_duration: f32) {
    // If both are 0, that means the window has been opened for the first time or has been
    // unminimized. If that is the case, only one of the colors should be visible while fading.
    if border.active_color.get_opacity() == 0.0 && border.inactive_color.get_opacity() == 0.0 {
        // Set fade_progress here so we start from 0 opacity for the visible color
        border.animations.fade_progress = match border.is_active_window {
            true => 0.0,
            false => 1.0,
        };

        border.animations.fade_to_visible = true;
    }

    // Determine which direction we should move fade_progress
    let direction = match border.is_active_window {
        true => 1.0,
        false => -1.0,
    };

    let delta_x = anim_elapsed.as_secs_f32() * 1000.0 / anim_duration * direction;
    border.animations.fade_progress += delta_x;

    // Check if the fade animation is finished
    if !(0.0..=1.0).contains(&border.animations.fade_progress) {
        let final_opacity = border.animations.fade_progress.clamp(0.0, 1.0);

        border.active_color.set_opacity(final_opacity);
        border.inactive_color.set_opacity(1.0 - final_opacity);

        border.animations.fade_progress = final_opacity;
        border.animations.fade_to_visible = false;
        border.event_anim = ANIM_NONE;
        return;
    }

    let bezier_control_points = match border.animations.current.get(&AnimType::Fade) {
        Some(anim_params) => anim_params.easing,
        // This 'None' arm should rarely, if ever be reached because this 'animate_fade' function
        // generally won't be called unless AnimType::Fade exists in the current animations hashmap
        None => AnimEasing::default().to_points(),
    };

    let easing_function = match cubic_bezier(&bezier_control_points) {
        Ok(func) => func,
        Err(e) => {
            error!("{e}");
            border.event_anim = ANIM_NONE;
            return;
        }
    };

    let y_coord = easing_function(border.animations.fade_progress);

    let (new_active_opacity, new_inactive_opacity) = match border.animations.fade_to_visible {
        true => match border.is_active_window {
            true => (y_coord, 0.0),
            false => (0.0, 1.0 - y_coord),
        },
        false => (y_coord, 1.0 - y_coord),
    };

    border.active_color.set_opacity(new_active_opacity);
    border.inactive_color.set_opacity(new_inactive_opacity);
}
