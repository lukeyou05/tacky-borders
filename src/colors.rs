use core::f32;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use windows::Win32::Foundation::{BOOL, FALSE, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1HwndRenderTarget, D2D1_BRUSH_PROPERTIES, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
};
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorConfig {
    SolidConfig(String),
    GradientConfig(GradientConfig),
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self::SolidConfig("#000000".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientConfig {
    pub colors: Vec<String>,
    pub direction: GradientDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GradientDirection {
    Angle(String),
    Coordinates(GradientCoordinates),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientCoordinates {
    pub start: [f32; 2],
    pub end: [f32; 2],
}

impl ColorConfig {
    // Convert the ColorConfig struct to a Color struct
    pub fn convert_to_color(&self, is_active_color: bool) -> Color {
        match self {
            ColorConfig::SolidConfig(solid_config) => {
                if solid_config == "accent" {
                    Color::Solid(Solid {
                        color: get_accent_color(is_active_color),
                        opacity: 0.0,
                    })
                } else {
                    Color::Solid(Solid {
                        color: get_color_from_hex(solid_config.as_str()),
                        opacity: 0.0,
                    })
                }
            }
            ColorConfig::GradientConfig(gradient_config) => {
                // We use 'step' to calculate the position of each color in the gradient below
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

                let direction = match gradient_config.direction {
                    // If we have an angle, we need to convert it into Coordinates
                    GradientDirection::Angle(ref angle) => {
                        let Some(degree) = angle
                            .strip_suffix("deg")
                            .and_then(|d| d.trim().parse::<f32>().ok())
                        else {
                            error!("Config contains an invalid gradient direction!");
                            return Color::default();
                        };

                        // We multiply degree by -1 to account for the fact that Win32's coordinate
                        // system has its origin at the top left instead of the bottom left
                        let rad = -degree * PI / 180.0;

                        // Calculate the slope of the line whilst accounting for edge cases like 90
                        // and 270 degrees where we would otherwise be dividing by 0 or something
                        // close to 0.
                        let m = match degree.abs() % 360.0 {
                            90.0 | 270.0 => degree.signum() * f32::MAX,
                            _ => rad.sin() / rad.cos(),
                        };

                        // y - y_p = m(x - x_p);
                        // y = m(x - x_p) + y_p;
                        // y = m*x - m*x_p + y_p;
                        // b = -m*x_p + y_p;

                        // Calculate the y-intercept of the line such that it goes through the
                        // center point (0.5, 0.5)
                        let b = -m * 0.5 + 0.5;

                        // Create the line with the given slope and y-intercept
                        let line = Line { m, b };

                        // y = mx + b
                        // 0 = mx + b
                        // mx = -b
                        // x = -b/m

                        // y = mx + b
                        // 1 = mx + b
                        // mx = 1 - b
                        // x = (1 - b)/m

                        // When we cross certain angle thresholds, like 90 degrees, we need to flip
                        // the x values (0.0 and 1.0) that we use to the calculate the start and
                        // end points below due to the slope changing
                        let (x_s, x_e) = match degree.abs() % 360.0 {
                            0.0..90.0 => (0.0, 1.0),
                            90.0..270.0 => (1.0, 0.0),
                            270.0..360.0 => (0.0, 1.0),
                            _ => {
                                debug!("Reached a gradient angle that is not covered by the match statement in colors.rs");
                                (0.0, 1.0)
                            }
                        };

                        // Here, we are checking three cases to make sure the calculated point
                        // lies within the first quadrant:
                        //
                        // Case 1: the y-coordinate at x_s is between 0 and 1
                        // Case 2: the y-coordinate at x_s is greater than 1
                        // Case 3: the y-coordinate at x_s is less than 0
                        let start = match line.plug_in_x(x_s) {
                            0.0..=1.0 => [x_s, line.plug_in_x(x_s)],
                            1.0.. => [(1.0 - line.b) / line.m, 1.0],
                            _ => [-line.b / line.m, 0.0],
                        };

                        let end = match line.plug_in_x(x_e) {
                            0.0..=1.0 => [x_e, line.plug_in_x(x_e)],
                            1.0.. => [(1.0 - line.b) / line.m, 1.0],
                            _ => [-line.b / line.m, 0.0],
                        };

                        GradientCoordinates { start, end }
                    }
                    GradientDirection::Coordinates(ref coordinates) => coordinates.clone(),
                };

                Color::Gradient(Gradient {
                    gradient_stops,
                    direction,
                    opacity: 0.0,
                })
            }
        }
    }
}

#[derive(Debug)]
struct Line {
    m: f32,
    b: f32,
}

impl Line {
    fn plug_in_x(&self, x: f32) -> f32 {
        self.m * x + self.b
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
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub struct Gradient {
    pub gradient_stops: Vec<D2D1_GRADIENT_STOP>, // Array of gradient stops
    pub direction: GradientCoordinates,
    pub opacity: f32,
}

impl Color {
    pub fn create_brush(
        &self,
        render_target: &ID2D1HwndRenderTarget,
        window_rect: &RECT,
        brush_properties: &D2D1_BRUSH_PROPERTIES,
    ) -> windows::core::Result<ID2D1Brush> {
        match self {
            Color::Solid(solid) => unsafe {
                let brush =
                    render_target.CreateSolidColorBrush(&solid.color, Some(brush_properties))?;

                brush.SetOpacity(solid.opacity);

                Ok(brush.into())
            },
            Color::Gradient(gradient) => unsafe {
                let width = (window_rect.right - window_rect.left) as f32;
                let height = (window_rect.bottom - window_rect.top) as f32;

                // The direction/GradientCoordinates only range from 0.0 to 1.0, but we need to
                // convert it into coordinates in terms of pixels
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

                let gradient_stop_collection = render_target.CreateGradientStopCollection(
                    &gradient.gradient_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;

                let brush = render_target.CreateLinearGradientBrush(
                    &gradient_properties,
                    Some(brush_properties),
                    &gradient_stop_collection,
                )?;

                brush.SetOpacity(gradient.opacity);

                Ok(brush.into())
            },
        }
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        match self {
            Color::Gradient(gradient) => gradient.opacity = opacity,
            Color::Solid(solid) => solid.opacity = opacity,
        }
    }

    pub fn get_opacity(&self) -> f32 {
        match self {
            Color::Gradient(gradient) => gradient.opacity,
            Color::Solid(solid) => solid.opacity,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::Solid(Solid {
            color: D2D1_COLOR_F::default(),
            opacity: 0.0,
        })
    }
}

fn get_accent_color(is_active_color: bool) -> D2D1_COLOR_F {
    let mut pcr_colorization: u32 = 0;
    let mut pf_opaqueblend: BOOL = FALSE;

    // DwmGetColorizationColor gets the accent color and places it into 'pcr_colorization'
    if let Err(e) = unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) } {
        error!("Could not retrieve Windows accent color: {e}");
    }

    // Bit-shift the retrieved color to separate out the rgb components
    let accent_red = ((pcr_colorization & 0x00FF0000) >> 16) as f32 / 255.0;
    let accent_green = ((pcr_colorization & 0x0000FF00) >> 8) as f32 / 255.0;
    let accent_blue = (pcr_colorization & 0x000000FF) as f32 / 255.0;
    let accent_avg = (accent_red + accent_green + accent_blue) / 3.0;

    if is_active_color {
        D2D1_COLOR_F {
            r: accent_red,
            g: accent_green,
            b: accent_blue,
            a: 1.0,
        }
    } else {
        D2D1_COLOR_F {
            r: accent_avg / 1.5 + accent_red / 10.0,
            g: accent_avg / 1.5 + accent_green / 10.0,
            b: accent_avg / 1.5 + accent_blue / 10.0,
            a: 1.0,
        }
    }
}

fn get_color_from_hex(hex: &str) -> D2D1_COLOR_F {
    if !matches!(hex.len(), 7 | 9 | 4 | 5) || !hex.starts_with('#') {
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

fn get_color_from_rgba(rgba: &str) -> D2D1_COLOR_F {
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
