use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use std::time;

use windows::Foundation::Numerics::Matrix3x2;

use crate::utils::cubic_bezier;
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
fn animation<'de, D>(deserializer: D) -> Result<HashMap<AnimationType, f32>, D::Error>
where
    D: Deserializer<'de>,
{
    // If deserialize returns an error, it's possible that an invalid AnimationType was listed
    let hashmap = HashMap::<AnimationType, Value>::deserialize(deserializer)?;

    let mut deserialized = HashMap::new();
    for (key, value) in hashmap {
        let speed = match value {
            Value::Number(n) => n.as_f64().map(|f| f as f32),
            Value::Null => None, // If the value is null, we will assign default speeds later
            _ => None,           // Handle invalid formats
        };

        let default_speed = 100.0;

        // If the speed is None (either null or missing), assign the default speed
        deserialized.insert(key, speed.unwrap_or(default_speed));
    }

    Ok(deserialized)
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
    #[serde(skip)]
    pub fade_progress: f32,
    #[serde(skip)]
    pub fade_only_one_color: bool,
    #[serde(skip)]
    pub spiral_angle: f32,
}

fn default_fps() -> i32 {
    60
}

pub fn animate_spiral(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    border.animations.spiral_angle += anim_elapsed.as_secs_f32() * anim_speed;

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

pub fn animate_fade(border: &mut WindowBorder, anim_elapsed: &time::Duration, anim_speed: f32) {
    // If both are 0, that means the window has been opened for the first time or has been
    // unminimized. If that is the case, only one of the colors should be visible while fading.
    if border.active_color.get_opacity() == 0.0 && border.inactive_color.get_opacity() == 0.0 {
        // Set fade_progress here so we start from 0 opacity for the visible color
        border.animations.fade_progress = match border.is_active_window {
            true => 0.0,
            false => 1.0,
        };

        border.animations.fade_only_one_color = true;
    }

    // Determine which direction we should move fade_progress
    let direction = match border.is_active_window {
        true => 1.0,
        false => -1.0,
    };

    let delta_x = anim_elapsed.as_secs_f32() * anim_speed * direction;
    border.animations.fade_progress += delta_x;

    // Check if the fade animation is finished
    if !(0.0..=1.0).contains(&border.animations.fade_progress) {
        let final_opacity = border.animations.fade_progress.clamp(0.0, 1.0);

        border.active_color.set_opacity(final_opacity);
        border.inactive_color.set_opacity(1.0 - final_opacity);

        border.animations.fade_progress = final_opacity;
        border.animations.fade_only_one_color = false;
        border.event_anim = ANIM_NONE;
        return;
    }

    // TODO perhaps add config options for this
    //
    // Basically EaseInOutQuad
    let ease_in_out_quad = match cubic_bezier(0.42, 0.0, 0.58, 1.0) {
        Ok(func) => func,
        Err(e) => {
            error!("{e}");
            border.event_anim = ANIM_NONE;
            return;
        }
    };

    let y_coord = ease_in_out_quad(border.animations.fade_progress);

    let (new_active_opacity, new_inactive_opacity) = match border.animations.fade_only_one_color {
        true => match border.is_active_window {
            true => (y_coord, 0.0),
            false => (0.0, 1.0 - y_coord),
        },
        false => (y_coord, 1.0 - y_coord),
    };

    border.active_color.set_opacity(new_active_opacity);
    border.inactive_color.set_opacity(new_inactive_opacity);
}
