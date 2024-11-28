use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_yaml::Value;
use std::collections::HashMap;
use std::time;

use windows::Foundation::Numerics::*;

use crate::utils::bezier;
use crate::window_border::WindowBorder;

pub const ANIM_NONE: i32 = 0;
pub const ANIM_FADE: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimationType {
    Spiral,
    ReverseSpiral,
    Fade,
}

// Custom deserializer for HashMap<AnimationType, Option<f32>>
pub fn animation<'de, D>(deserializer: D) -> Result<HashMap<AnimationType, f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(map): Option<HashMap<AnimationType, Value>> = Option::deserialize(deserializer)?
    else {
        return Ok(HashMap::default());
    };

    let mut result = HashMap::new();
    for (key, value) in map {
        // Default speed is 100 if the value is missing or null
        let speed = match value {
            Value::Number(n) => n.as_f64().map(|f| f as f32),
            Value::Null => None, // If the value is null, we will assign default speeds later
            _ => None,           // Handle invalid formats
        };

        // Apply the default speed for each animation type if it's null or missing
        let default_speed = 100.0;

        // If the speed is None (either null or missing), assign the default speed
        result.insert(key, speed.unwrap_or(default_speed));
    }

    Ok(result)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct Animations {
    #[serde(default, deserialize_with = "animation")]
    pub active: HashMap<AnimationType, f32>,
    #[serde(default, deserialize_with = "animation")]
    pub inactive: HashMap<AnimationType, f32>,
    #[serde(skip)]
    pub current: HashMap<AnimationType, f32>,
    #[serde(default = "default_fps")]
    pub fps: i32,
    // fade_progress is used for fade animations
    #[serde(skip)]
    pub fade_progress: f32,
    // spiral_angle is used for spiral animations
    #[serde(skip)]
    pub spiral_angle: f32,
}

fn default_fps() -> i32 {
    60
}

pub fn animate_spiral(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    if border.animations.spiral_angle >= 360.0 {
        border.animations.spiral_angle -= 360.0;
    }
    border.animations.spiral_angle += (anim_elapsed.as_secs_f32() * anim_speed).min(359.0);

    let center_x = (border.window_rect.right - border.window_rect.left) / 2;
    let center_y = (border.window_rect.bottom - border.window_rect.top) / 2;

    border.brush_properties.transform = Matrix3x2::rotation(
        border.animations.spiral_angle,
        center_x as f32,
        center_y as f32,
    );
}

pub fn animate_reverse_spiral(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_speed: f32,
) {
    border.animations.spiral_angle %= 360.0;
    if border.animations.spiral_angle < 0.0 {
        border.animations.spiral_angle += 360.0;
    }
    border.animations.spiral_angle -= (anim_elapsed.as_secs_f32() * anim_speed).min(359.0);

    let center_x = (border.window_rect.right - border.window_rect.left) / 2;
    let center_y = (border.window_rect.bottom - border.window_rect.top) / 2;
    border.brush_properties.transform = Matrix3x2::rotation(
        border.animations.spiral_angle,
        center_x as f32,
        center_y as f32,
    );
}

pub fn animate_fade(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    let (bottom_color, top_color) = match border.is_active_window {
        true => (&mut border.inactive_color, &mut border.active_color),
        false => (&mut border.active_color, &mut border.inactive_color),
    };

    let delta_x = anim_elapsed.as_secs_f32() * anim_speed;
    border.animations.fade_progress += delta_x;

    let bottom_opacity = bottom_color.get_opacity();
    let top_opacity = top_color.get_opacity();

    // Check if the fade animation is finished. This also prevents the animation from playing when
    // windows are shown/uncloaked and the opacity is already 1.0.
    if border.animations.fade_progress >= 1.0 || top_opacity >= 1.0 {
        top_color.set_opacity(1.0);
        bottom_color.set_opacity(0.0);

        // Reset fade_progress so we can reuse it next time
        border.animations.fade_progress = 0.0;
        border.event_anim = ANIM_NONE;
        return;
    }

    // TODO perhaps add config options for this
    //
    // Basically EaseInOutQuad
    let Ok(ease_in_out_quad) = bezier(0.42, 0.0, 0.58, 1.0) else {
        return;
    };

    let new_top_opacity = ease_in_out_quad(border.animations.fade_progress);

    // I do the following because I want this to work when a window is first opened (when only the
    // top color should be visible) without having to write a separate function for it lol.
    let new_bottom_opacity = match bottom_opacity == 0.0 {
        true => 0.0,
        false => 1.0 - new_top_opacity,
    };

    top_color.set_opacity(new_top_opacity);
    bottom_color.set_opacity(new_bottom_opacity);
}
