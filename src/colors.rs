use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_yaml::Value;
use std::collections::HashMap;
use windows::{
    Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*, Win32::Graphics::Direct2D::*,
    Win32::Graphics::Dwm::*,
};

use crate::utils::*;

pub const ANIM_NONE: i32 = 0;
pub const ANIM_FADE_TO_ACTIVE: i32 = 1;
pub const ANIM_FADE_TO_INACTIVE: i32 = 2;
pub const ANIM_FADE_TO_VISIBLE: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimationType {
    Spiral,
    Fade,
}

// Custom deserializer for Option<HashMap<AnimationType, Option<f32>>>
pub fn default_animation_speed<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<AnimationType, f32>>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(map): Option<HashMap<AnimationType, Value>> = Option::deserialize(deserializer)?
    else {
        return Ok(None);
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

    Ok(Some(result))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorConfig {
    SolidConfig(String),
    GradientConfig(GradientConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientConfig {
    pub colors: Vec<String>,
    pub direction: GradientDirectionCoordinates,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientDirectionCoordinates {
    pub start: [f32; 2],
    pub end: [f32; 2],
}

impl ColorConfig {
    pub fn convert_to_color(&self, is_active_color: bool) -> Color {
        match self {
            ColorConfig::SolidConfig(solid_config) => {
                if solid_config == "accent" {
                    // Get the Windows accent color
                    let mut pcr_colorization: u32 = 0;
                    let mut pf_opaqueblend: BOOL = FALSE;
                    let result = unsafe {
                        DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend)
                    };
                    if result.is_err() {
                        println!("Error getting Windows accent color!");
                    }
                    let accent_red = ((pcr_colorization & 0x00FF0000) >> 16) as f32 / 255.0;
                    let accent_green = ((pcr_colorization & 0x0000FF00) >> 8) as f32 / 255.0;
                    let accent_blue = (pcr_colorization & 0x000000FF) as f32 / 255.0;
                    let accent_avg = (accent_red + accent_green + accent_blue) / 3.0;

                    if is_active_color {
                        Color::Solid(Solid {
                            color: D2D1_COLOR_F {
                                r: accent_red,
                                g: accent_green,
                                b: accent_blue,
                                a: 1.0,
                            },
                        })
                    } else {
                        Color::Solid(Solid {
                            color: D2D1_COLOR_F {
                                r: accent_avg / 1.5 + accent_red / 10.0,
                                g: accent_avg / 1.5 + accent_green / 10.0,
                                b: accent_avg / 1.5 + accent_blue / 10.0,
                                a: 1.0,
                            },
                        })
                    }
                } else {
                    Color::Solid(Solid {
                        color: get_color_from_hex(solid_config.as_str()),
                    })
                }
            }
            ColorConfig::GradientConfig(gradient_config) => {
                let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
                let step = 1.0 / (gradient_config.colors.len() - 1) as f32;

                for i in 0..gradient_config.colors.len() {
                    let color = get_color_from_hex(gradient_config.colors[i].as_str());
                    let gradient_stop = D2D1_GRADIENT_STOP {
                        position: i as f32 * step,
                        color,
                    };
                    gradient_stops.push(gradient_stop);
                }

                Color::Gradient(Gradient {
                    gradient_stops,
                    direction: gradient_config.direction.clone(),
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Color {
    Solid(Solid),
    Gradient(Gradient),
}

#[derive(Debug, Clone)]
pub struct Solid {
    pub color: D2D1_COLOR_F,
}

#[derive(Debug, Clone)]
pub struct Gradient {
    pub gradient_stops: Vec<D2D1_GRADIENT_STOP>, // Array of gradient stops
    pub direction: GradientDirectionCoordinates,
}

impl Color {
    pub fn create_brush(
        &mut self,
        render_target: &ID2D1HwndRenderTarget,
        window_rect: &RECT,
        brush_properties: &D2D1_BRUSH_PROPERTIES,
    ) -> Option<ID2D1Brush> {
        match self {
            Color::Solid(solid) => unsafe {
                let Ok(brush) =
                    render_target.CreateSolidColorBrush(&solid.color, Some(brush_properties))
                else {
                    return None;
                };
                Some(brush.into())
            },
            Color::Gradient(gradient) => unsafe {
                //let before = std::time::Instant::now();

                let width = (window_rect.right - window_rect.left) as f32;
                let height = (window_rect.bottom - window_rect.top) as f32;
                let gradient_properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: D2D_POINT_2F {
                        x: gradient.direction.start[0] * width,
                        y: gradient.direction.start[1] * height,
                    },
                    endPoint: D2D_POINT_2F {
                        x: gradient.direction.end[0] * width,
                        y: gradient.direction.end[1] * height,
                    },
                };

                let Ok(gradient_stop_collection) = render_target.CreateGradientStopCollection(
                    &gradient.gradient_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                ) else {
                    // TODO instead of panicking, I should just return a default value
                    panic!("could not create gradient_stop_collection!");
                };

                /*println!(
                    "time it took to set up gradient brush variables: {:?}",
                    before.elapsed()
                );*/

                //let before = std::time::Instant::now();

                let Ok(brush) = render_target.CreateLinearGradientBrush(
                    &gradient_properties,
                    Some(brush_properties),
                    &gradient_stop_collection,
                ) else {
                    return None;
                };

                //println!("time it took to create brush: {:?}", before.elapsed());

                Some(brush.into())
            },
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::Solid(Solid {
            color: D2D1_COLOR_F::default(),
        })
    }
}
