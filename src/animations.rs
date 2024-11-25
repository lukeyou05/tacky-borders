use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_yaml::Value;
use std::collections::HashMap;
use std::time;

use windows::Foundation::Numerics::*;
use windows::Win32::Graphics::Direct2D::Common::*;

use crate::colors::*;
use crate::utils::*;
use crate::window_border::WindowBorder;

pub const ANIM_NONE: i32 = 0;
pub const ANIM_FADE_TO_ACTIVE: i32 = 1;
pub const ANIM_FADE_TO_INACTIVE: i32 = 2;
pub const ANIM_FADE_TO_VISIBLE: i32 = 3;

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

pub fn animate_fade_setup(border: &mut WindowBorder) {
    // Reset last_anim_time here because otherwise, anim_elapsed will be
    // too large due to being paused and interpolation won't work correctly
    border.last_anim_time = Some(time::Instant::now());

    border.current_color = if border.is_active_window {
        border.active_color.clone()
    } else {
        border.inactive_color.clone()
    };

    // Set the alpha of the current color to 0 so we can animate from invisible to visible
    if let Color::Gradient(mut current_gradient) = border.current_color.clone() {
        let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
        for i in 0..current_gradient.gradient_stops.len() {
            current_gradient.gradient_stops[i].color.a = 0.0;
            let color = current_gradient.gradient_stops[i].color;
            let position = current_gradient.gradient_stops[i].position;
            gradient_stops.push(D2D1_GRADIENT_STOP { color, position });
        }

        let direction = current_gradient.direction;

        border.current_color = Color::Gradient(Gradient {
            gradient_stops,
            direction,
        })
    } else if let Color::Solid(mut current_solid) = border.current_color.clone() {
        current_solid.color.a = 0.0;
        let color = current_solid.color;

        border.current_color = Color::Solid(Solid { color });
    }
}

pub fn animate_fade_colors(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_speed: f32,
) {
    if let Color::Solid(_) = border.active_color {
        if let Color::Solid(_) = border.inactive_color {
            // If both active and inactive color are solids, use interpolate_solids
            interpolate_solids(border, anim_elapsed, anim_speed);
        }
    } else {
        interpolate_gradients(border, anim_elapsed, anim_speed);
    }
}

pub fn interpolate_solids(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_speed: f32,
) {
    let Color::Solid(current_solid) = border.current_color.clone() else {
        error!("Could not convert current_color for interpolation");
        return;
    };
    let end_solid = match border.is_active_window {
        true => {
            let Color::Solid(active_solid) = border.active_color.clone() else {
                error!("Could not convet active_color for interpolation");
                return;
            };
            active_solid
        }
        false => {
            let Color::Solid(inactive_solid) = border.inactive_color.clone() else {
                error!("Could not convet active_color for interpolation");
                return;
            };
            inactive_solid
        }
    };

    let mut finished = false;
    let color = match border.event_anim {
        ANIM_FADE_TO_VISIBLE | ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
            interpolate_d2d1_to_visible(
                &current_solid.color,
                &end_solid.color,
                anim_elapsed.as_secs_f32(),
                anim_speed,
                &mut finished,
            )
        }
        _ => return,
    };

    if finished {
        border.event_anim = ANIM_NONE;
    } else {
        border.current_color = Color::Solid(Solid { color });
    }
}

pub fn interpolate_gradients(
    border: &mut WindowBorder,
    anim_elapsed: &time::Duration,
    anim_speed: f32,
) {
    let current_gradient = match border.current_color.clone() {
        Color::Gradient(gradient) => gradient,
        Color::Solid(solid) => {
            // If current_color is not a gradient, that means at least one of active or inactive
            // color must be solid, so only one of these if let statements should evaluate true
            let gradient = if let Color::Gradient(active_gradient) = border.active_color.clone() {
                active_gradient
            } else if let Color::Gradient(inactive_gradient) = border.inactive_color.clone() {
                inactive_gradient
            } else {
                error!("Could not convert active_color or inactive_color for interpolation");
                return;
            };

            // Convert current_color to a gradient
            let mut solid_as_gradient = gradient.clone();
            for i in 0..solid_as_gradient.gradient_stops.len() {
                solid_as_gradient.gradient_stops[i].color = solid.color;
            }
            solid_as_gradient
        }
    };

    let mut all_finished = true;
    let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
    for i in 0..current_gradient.gradient_stops.len() {
        let mut current_finished = false;

        let end_color = match border.is_active_window {
            true => {
                match border.active_color.clone() {
                    Color::Gradient(gradient) => {
                        if current_gradient.gradient_stops.len() != gradient.gradient_stops.len() {
                            error!("Gradients are not the same length. Exiting function to prevent panic!");
                            border.current_color = border.active_color.clone();
                            border.event_anim = ANIM_NONE;
                            return;
                        }
                        gradient.gradient_stops[i].color
                    }
                    Color::Solid(solid) => solid.color,
                }
            }
            false => {
                match border.inactive_color.clone() {
                    Color::Gradient(gradient) => {
                        if current_gradient.gradient_stops.len() != gradient.gradient_stops.len() {
                            error!("Gradients are not the same length. Exiting function to prevent panic!");
                            border.current_color = border.inactive_color.clone();
                            border.event_anim = ANIM_NONE;
                            return;
                        }
                        gradient.gradient_stops[i].color
                    }
                    Color::Solid(solid) => solid.color,
                }
            }
        };

        let color = match border.event_anim {
            ANIM_FADE_TO_VISIBLE | ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
                interpolate_d2d1_to_visible(
                    &current_gradient.gradient_stops[i].color,
                    &end_color,
                    anim_elapsed.as_secs_f32(),
                    anim_speed,
                    &mut current_finished,
                )
            }
            _ => return,
        };

        if !current_finished {
            all_finished = false;
        }

        // TODO currently this works well because users cannot adjust the positions of the
        // gradient stops, so both inactive and active gradients will have the same positions,
        // but this might need to be interpolated if we add position configuration.
        let position = current_gradient.gradient_stops[i].position;

        let stop = D2D1_GRADIENT_STOP { color, position };
        gradient_stops.push(stop);
    }

    let direction = current_gradient.direction;

    if all_finished {
        match border.event_anim {
            ANIM_FADE_TO_ACTIVE => border.current_color = border.active_color.clone(),
            ANIM_FADE_TO_INACTIVE => border.current_color = border.inactive_color.clone(),
            ANIM_FADE_TO_VISIBLE => {
                border.current_color = match border.is_active_window {
                    true => border.active_color.clone(),
                    false => border.inactive_color.clone(),
                }
            }
            _ => {}
        }
        border.event_anim = ANIM_NONE;
    } else {
        border.current_color = Color::Gradient(Gradient {
            gradient_stops,
            direction,
        });
    }
}
