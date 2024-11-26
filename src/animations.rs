use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_yaml::Value;
use std::collections::HashMap;
use std::time;

use windows::Foundation::Numerics::*;

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
    #[serde(default = "default_fps")]
    pub fps: i32,
}

fn default_fps() -> i32 {
    60
}

pub fn animate_spiral(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    if border.spiral_anim_angle >= 360.0 {
        border.spiral_anim_angle -= 360.0;
    }
    border.spiral_anim_angle += (anim_elapsed.as_secs_f32() * anim_speed).min(359.0);

    let center_x = (border.window_rect.right - border.window_rect.left) / 2;
    let center_y = (border.window_rect.bottom - border.window_rect.top) / 2;

    border.brush_properties.transform =
        Matrix3x2::rotation(border.spiral_anim_angle, center_x as f32, center_y as f32);
}

pub fn animate_reverse_spiral(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_speed: f32,
) {
    border.spiral_anim_angle %= 360.0;
    if border.spiral_anim_angle < 0.0 {
        border.spiral_anim_angle += 360.0;
    }
    border.spiral_anim_angle -= (anim_elapsed.as_secs_f32() * anim_speed).min(359.0);

    let center_x = (border.window_rect.right - border.window_rect.left) / 2;
    let center_y = (border.window_rect.bottom - border.window_rect.top) / 2;
    border.brush_properties.transform =
        Matrix3x2::rotation(border.spiral_anim_angle, center_x as f32, center_y as f32);
}

pub fn animate_fade(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    let anim_step = anim_elapsed.as_secs_f32() * anim_speed;

    let (bottom_color, top_color) = match border.is_active_window {
        true => (&mut border.inactive_color, &mut border.active_color),
        false => (&mut border.active_color, &mut border.inactive_color),
    };

    let mut new_top_opacity = top_color.get_opacity() + anim_step;
    let mut new_bottom_opacity = bottom_color.get_opacity() - anim_step;

    if new_top_opacity >= 1.0 {
        new_top_opacity = 1.0;
        new_bottom_opacity = 0.0;
        border.event_anim = ANIM_NONE;
    }

    top_color.set_opacity(new_top_opacity);
    bottom_color.set_opacity(new_bottom_opacity);
}
