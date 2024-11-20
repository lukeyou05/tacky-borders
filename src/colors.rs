use serde::Deserialize;
use serde::Serialize;
use windows::{
    Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*, Win32::Graphics::Direct2D::*,
    Win32::Graphics::Dwm::*,
};

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
                        error!("Could not retrieve Windows accent color!");
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
                let step = 1.0 / (gradient_config.colors.len() - 1) as f32;

                let gradient_stops = gradient_config
                    .clone()
                    .colors
                    .into_iter()
                    .enumerate()
                    .map(|(i, color)| D2D1_GRADIENT_STOP {
                        position: i as f32 * step,
                        color: get_color_from_hex(color.as_str()),
                    })
                    .collect();

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

                let Ok(brush) = render_target.CreateLinearGradientBrush(
                    &gradient_properties,
                    Some(brush_properties),
                    &gradient_stop_collection,
                ) else {
                    return None;
                };

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

pub fn get_color_from_hex(hex: &str) -> D2D1_COLOR_F {
    if hex.len() != 7 && hex.len() != 9 && hex.len() != 4 && hex.len() != 5 || !hex.starts_with('#')
    {
        error!("Invalid hex color format: {}", hex);
        return D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
    }
    // Expand shorthand hex formats (#RGB or #RGBA to #RRGGBB or #RRGGBBAA)
    let expanded_hex = match hex.len() {
        4 => format!(
            "#{}{}{}{}{}{}",
            &hex[1..2],
            &hex[1..2],
            &hex[2..3],
            &hex[2..3],
            &hex[3..4],
            &hex[3..4]
        ),
        5 => format!(
            "#{}{}{}{}{}{}{}{}",
            &hex[1..2],
            &hex[1..2],
            &hex[2..3],
            &hex[2..3],
            &hex[3..4],
            &hex[3..4],
            &hex[4..5],
            &hex[4..5]
        ),
        _ => hex.to_string(),
    };

    // Convert each color component to f32 between 0.0 and 1.0, handling errors
    let parse_component = |s: &str| -> f32 {
        match u8::from_str_radix(s, 16) {
            Ok(val) => val as f32 / 255.0,
            Err(_) => {
                error!("Invalid component '{}' in hex: {}", s, expanded_hex);
                0.0
            }
        }
    };

    // Parse RGB values
    let r = parse_component(&expanded_hex[1..3]);
    let g = parse_component(&expanded_hex[3..5]);
    let b = parse_component(&expanded_hex[5..7]);

    // Parse alpha value if present
    let a = if expanded_hex.len() == 9 {
        parse_component(&expanded_hex[7..9])
    } else {
        1.0
    };

    D2D1_COLOR_F { r, g, b, a }
}
pub fn get_color_from_rgba(rgba: &str) -> D2D1_COLOR_F {
    let rgba = rgba
        .trim_start_matches("rgb(")
        .trim_start_matches("rgba(")
        .trim_end_matches(')');
    let components: Vec<&str> = rgba.split(',').map(|s| s.trim()).collect();
    // Check for correct number of components
    if components.len() == 3 || components.len() == 4 {
        // Parse red, green, and blue values
        let red: f32 = components[0].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let green: f32 = components[1].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let blue: f32 = components[2].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let alpha: f32 = if components.len() == 4 {
            components[3].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        return D2D1_COLOR_F {
            r: red,
            g: green,
            b: blue,
            a: alpha, // Default alpha value for rgb()
        };
    }
    // Return a default color if parsing fails
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
