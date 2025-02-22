use anyhow::{anyhow, Context};
use core::f32;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, FALSE, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_POINT_2F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1LinearGradientBrush, ID2D1RenderTarget, ID2D1SolidColorBrush,
    D2D1_BRUSH_PROPERTIES, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
};
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;

use crate::LogIfErr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ColorBrushConfig {
    Solid(String),
    Gradient(GradientBrushConfig),
}

impl Default for ColorBrushConfig {
    fn default() -> Self {
        Self::Solid("accent".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GradientBrushConfig {
    pub colors: Vec<String>,
    pub direction: GradientDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum GradientDirection {
    Angle(String),
    Coordinates(GradientCoordinates),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GradientCoordinates {
    pub start: [f32; 2],
    pub end: [f32; 2],
}

#[derive(Debug, Clone)]
pub enum ColorBrush {
    Solid(SolidBrush),
    Gradient(GradientBrush),
}

impl Default for ColorBrush {
    fn default() -> Self {
        ColorBrush::Solid(SolidBrush {
            color: D2D1_COLOR_F::default(),
            brush: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SolidBrush {
    color: D2D1_COLOR_F,
    brush: Option<ID2D1SolidColorBrush>,
}

#[derive(Debug, Clone)]
pub struct GradientBrush {
    gradient_stops: Vec<D2D1_GRADIENT_STOP>, // Array of gradient stops
    direction: GradientCoordinates,
    brush: Option<ID2D1LinearGradientBrush>,
}

impl ColorBrushConfig {
    // Convert the ColorConfig struct to a Color struct
    pub fn to_color(&self, is_active_color: bool) -> ColorBrush {
        match self {
            ColorBrushConfig::Solid(solid_config) => {
                if solid_config == "accent" {
                    ColorBrush::Solid(SolidBrush {
                        color: get_accent_color(is_active_color),
                        brush: None,
                    })
                } else {
                    ColorBrush::Solid(SolidBrush {
                        color: get_color_from_hex(solid_config.as_str()),
                        brush: None,
                    })
                }
            }
            ColorBrushConfig::Gradient(gradient_config) => {
                // We use 'step' to calculate the position of each color in the gradient below
                let step = 1.0 / (gradient_config.colors.len() - 1) as f32;

                let gradient_stops = gradient_config
                    .clone()
                    .colors
                    .into_iter()
                    .enumerate()
                    .map(|(i, color)| D2D1_GRADIENT_STOP {
                        position: i as f32 * step,
                        color: if color == "accent" {
                            get_accent_color(is_active_color)
                        } else {
                            get_color_from_hex(color.as_str())
                        },
                    })
                    .collect();

                let direction = match gradient_config.direction {
                    // If we have an angle, we need to convert it into Coordinates
                    GradientDirection::Angle(ref angle) => {
                        let Some(degree) = angle
                            .strip_suffix("deg")
                            .and_then(|d| d.trim().parse::<f32>().ok())
                        else {
                            error!("config contains an invalid gradient direction!");
                            return ColorBrush::default();
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
                                debug!("reached a gradient angle that is not covered by the match statement in colors.rs");
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

                ColorBrush::Gradient(GradientBrush {
                    gradient_stops,
                    direction,
                    brush: None,
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

impl ColorBrush {
    // NOTE: ID2D1DeviceContext7 implements From<&ID2D1DeviceContext7> for &ID2D1RenderTarget
    pub fn init_brush(
        &mut self,
        renderer: &ID2D1RenderTarget,
        window_rect: &RECT,
        brush_properties: &D2D1_BRUSH_PROPERTIES,
    ) -> windows::core::Result<()> {
        match self {
            ColorBrush::Solid(solid) => unsafe {
                let id2d1_brush =
                    renderer.CreateSolidColorBrush(&solid.color, Some(brush_properties))?;

                solid.brush = Some(id2d1_brush);

                Ok(())
            },
            ColorBrush::Gradient(gradient) => unsafe {
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

                let gradient_stop_collection = renderer.CreateGradientStopCollection(
                    &gradient.gradient_stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;

                let id2d1_brush = renderer.CreateLinearGradientBrush(
                    &gradient_properties,
                    Some(brush_properties),
                    &gradient_stop_collection,
                )?;

                gradient.brush = Some(id2d1_brush);

                Ok(())
            },
        }
    }

    pub fn get_brush(&self) -> Option<&ID2D1Brush> {
        match self {
            ColorBrush::Solid(solid) => solid.brush.as_ref().map(|id2d1_brush| id2d1_brush.into()),
            ColorBrush::Gradient(gradient) => gradient
                .brush
                .as_ref()
                .map(|id2d1_brush| id2d1_brush.into()),
        }
    }

    pub fn set_opacity(&self, opacity: f32) {
        match self {
            ColorBrush::Gradient(gradient) => {
                if let Some(ref id2d1_brush) = gradient.brush {
                    unsafe { id2d1_brush.SetOpacity(opacity) }
                }
            }
            ColorBrush::Solid(solid) => {
                if let Some(ref id2d1_brush) = solid.brush {
                    unsafe { id2d1_brush.SetOpacity(opacity) }
                }
            }
        }
    }

    pub fn get_opacity(&self) -> Option<f32> {
        match self {
            ColorBrush::Solid(solid) => solid
                .brush
                .as_ref()
                .map(|id2d1_brush| unsafe { id2d1_brush.GetOpacity() }),
            ColorBrush::Gradient(gradient) => gradient
                .brush
                .as_ref()
                .map(|id2d1_brush| unsafe { id2d1_brush.GetOpacity() }),
        }
    }

    pub fn set_transform(&self, transform: &Matrix3x2) {
        match self {
            ColorBrush::Solid(solid) => {
                if let Some(ref id2d1_brush) = solid.brush {
                    unsafe { id2d1_brush.SetTransform(transform) };
                }
            }
            ColorBrush::Gradient(gradient) => {
                if let Some(ref id2d1_brush) = gradient.brush {
                    unsafe { id2d1_brush.SetTransform(transform) };
                }
            }
        }
    }

    pub fn get_transform(&self) -> Option<Matrix3x2> {
        match self {
            ColorBrush::Solid(solid) => solid.brush.as_ref().map(|id2d1_brush| {
                let mut transform = Matrix3x2::default();
                unsafe { id2d1_brush.GetTransform(&mut transform) };

                transform
            }),
            ColorBrush::Gradient(gradient) => gradient.brush.as_ref().map(|id2d1_brush| {
                let mut transform = Matrix3x2::default();
                unsafe { id2d1_brush.GetTransform(&mut transform) };

                transform
            }),
        }
    }
}

impl GradientBrush {
    pub fn update_start_end_points(&self, window_rect: &RECT) {
        let width = (window_rect.right - window_rect.left) as f32;
        let height = (window_rect.bottom - window_rect.top) as f32;

        // The direction/GradientCoordinates only range from 0.0 to 1.0, but we need to
        // convert it into coordinates in terms of pixels
        let start_point = D2D_POINT_2F {
            x: self.direction.start[0] * width,
            y: self.direction.start[1] * height,
        };
        let end_point = D2D_POINT_2F {
            x: self.direction.end[0] * width,
            y: self.direction.end[1] * height,
        };

        if let Some(ref id2d1_brush) = self.brush {
            unsafe {
                id2d1_brush.SetStartPoint(start_point);
                id2d1_brush.SetEndPoint(end_point)
            };
        }
    }
}

fn get_accent_color(is_active_color: bool) -> D2D1_COLOR_F {
    let mut pcr_colorization: u32 = 0;
    let mut pf_opaqueblend: BOOL = FALSE;

    // DwmGetColorizationColor gets the accent color and places it into 'pcr_colorization'
    unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) }
        .context("could not retrieve windows accent color")
        .log_if_err();

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
    let s = hex.strip_prefix("#").unwrap_or_default();
    parse_hex(s).unwrap_or_else(|err| {
        error!("could not parse hex: {err}");
        D2D1_COLOR_F::default()
    })
}

fn parse_hex(s: &str) -> anyhow::Result<D2D1_COLOR_F> {
    if !matches!(s.len(), 3 | 4 | 6 | 8) || !s[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("invalid hex: {s}"));
    }

    let n = s.len();

    let parse_digit = |digit: &str, single: bool| -> anyhow::Result<f32> {
        u8::from_str_radix(digit, 16)
            .map(|n| {
                if single {
                    ((n << 4) | n) as f32 / 255.0
                } else {
                    n as f32 / 255.0
                }
            })
            .map_err(|_| anyhow!("invalid hex: {s}"))
    };

    if n == 3 || n == 4 {
        let r = parse_digit(&s[0..1], true)?;
        let g = parse_digit(&s[1..2], true)?;
        let b = parse_digit(&s[2..3], true)?;

        let a = if n == 4 {
            parse_digit(&s[3..4], true)?
        } else {
            1.0
        };

        Ok(D2D1_COLOR_F { r, g, b, a })
    } else if n == 6 || n == 8 {
        let r = parse_digit(&s[0..2], false)?;
        let g = parse_digit(&s[2..4], false)?;
        let b = parse_digit(&s[4..6], false)?;

        let a = if n == 8 {
            parse_digit(&s[6..8], false)?
        } else {
            1.0
        };

        Ok(D2D1_COLOR_F { r, g, b, a })
    } else {
        Err(anyhow!("invalid hex: {s}"))
    }
}
