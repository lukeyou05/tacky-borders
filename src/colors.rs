use anyhow::{Context, anyhow};
use core::f32;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use windows::Win32::Foundation::{FALSE, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BRUSH_PROPERTIES, D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, ID2D1Brush, ID2D1LinearGradientBrush, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows::core::BOOL;
use windows_numerics::{Matrix3x2, Vector2};

use crate::LogIfErr;
use crate::theme::is_light_theme;
use crate::utils::WindowsCompatibleResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ColorBrushConfig {
    Solid(String),
    Gradient(GradientBrushConfig),
    ThemeAware(ThemeAwareColor),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ThemeAwareColor {
    pub dark: Box<ColorBrushConfig>,
    pub light: Box<ColorBrushConfig>,
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
    gradient_stops: Vec<D2D1_GRADIENT_STOP>,
    direction: GradientCoordinates,
    brush: Option<ID2D1LinearGradientBrush>,
}

impl ColorBrushConfig {
    pub fn to_color_brush(&self, is_active_color: bool) -> ColorBrush {
        match self {
            ColorBrushConfig::ThemeAware(theme) => {
                let resolved = if is_light_theme() {
                    &theme.light
                } else {
                    &theme.dark
                };
                resolved.to_color_brush(is_active_color)
            }
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
                    GradientDirection::Angle(ref angle) => {
                        let Some(degree) = angle
                            .strip_suffix("deg")
                            .and_then(|d| d.trim().parse::<f32>().ok())
                        else {
                            error!("config contains an invalid gradient direction!");
                            return ColorBrush::default();
                        };

                        // Calculate x and y distance from the center point [0.5, 0.5].
                        // We multiply rad.sin() by -1 (flip the y-component) because Direct2D uses
                        // the top-left as the origin, but most people expect bottom-left.
                        let rad = degree as f64 * PI / 180.0;
                        let (x_raw, y_raw) = (rad.cos(), -rad.sin());
                        let scalar = (1.0 / f64::max(x_raw.abs(), y_raw.abs())) * 0.5;
                        let (x_dist, y_dist) = (x_raw * scalar, y_raw * scalar);

                        let start = [0.5 - x_dist as f32, 0.5 - y_dist as f32];
                        let end = [0.5 + x_dist as f32, 0.5 + y_dist as f32];

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

impl ColorBrush {
    // NOTE: ID2D1DeviceContext implements From<&ID2D1DeviceContext> for &ID2D1RenderTarget
    pub fn init_brush(
        &mut self,
        renderer: &ID2D1RenderTarget,
        window_rect: &RECT,
        brush_properties: &D2D1_BRUSH_PROPERTIES,
    ) -> WindowsCompatibleResult<()> {
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
                // convert it into coordinates in terms of the screen's pixels
                let gradient_properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: Vector2 {
                        X: gradient.direction.start[0] * width,
                        Y: gradient.direction.start[1] * height,
                    },
                    endPoint: Vector2 {
                        X: gradient.direction.end[0] * width,
                        Y: gradient.direction.end[1] * height,
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

    pub fn take_brush(&mut self) -> Option<ID2D1Brush> {
        match self {
            ColorBrush::Solid(solid) => solid.brush.take().map(|id2d1_brush| id2d1_brush.into()),
            ColorBrush::Gradient(gradient) => {
                gradient.brush.take().map(|id2d1_brush| id2d1_brush.into())
            }
        }
    }

    pub fn set_opacity(&self, opacity: f32) -> anyhow::Result<()> {
        match self {
            ColorBrush::Solid(solid) => {
                let id2d1_brush = solid
                    .brush
                    .as_ref()
                    .context("brush has not been created yet")?;

                unsafe { id2d1_brush.SetOpacity(opacity) };
            }
            ColorBrush::Gradient(gradient) => {
                let id2d1_brush = gradient
                    .brush
                    .as_ref()
                    .context("brush has not been created yet")?;

                unsafe { id2d1_brush.SetOpacity(opacity) };
            }
        }

        Ok(())
    }

    pub fn get_opacity(&self) -> anyhow::Result<f32> {
        match self {
            ColorBrush::Solid(solid) => {
                let id2d1_brush = solid
                    .brush
                    .as_ref()
                    .context("brush has not been created yet")?;

                Ok(unsafe { id2d1_brush.GetOpacity() })
            }
            ColorBrush::Gradient(gradient) => {
                let id2d1_brush = gradient
                    .brush
                    .as_ref()
                    .context("brush has not been created yet")?;

                Ok(unsafe { id2d1_brush.GetOpacity() })
            }
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
        let start_point = Vector2 {
            X: self.direction.start[0] * width,
            Y: self.direction.start[1] * height,
        };
        let end_point = Vector2 {
            X: self.direction.end[0] * width,
            Y: self.direction.end[1] * height,
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
    // Check if the string is in with_opacity(color,X%) format
    if let Some(with_opacity_content) = hex.strip_prefix("with_opacity(").and_then(|s| s.strip_suffix(")")) {
        return parse_with_opacity(with_opacity_content).unwrap_or_else(|err| {
            error!("could not parse with_opacity: {err}");
            D2D1_COLOR_F::default()
        });
    }
    
    let s = hex.strip_prefix("#").unwrap_or_default();
    parse_hex(s).unwrap_or_else(|err| {
        error!("could not parse hex: {err:#}");
        D2D1_COLOR_F::default()
    })
}

fn parse_with_opacity(s: &str) -> anyhow::Result<D2D1_COLOR_F> {
    // Expected format: "color,X%" where color is "accent" or hex code, X is opacity percentage
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(anyhow!("invalid with_opacity format, expected 'color,X%': {s}"));
    }
    
    // Extract opacity percentage
    let alpha_str = parts[1].trim();
    if !alpha_str.ends_with('%') {
        return Err(anyhow!("invalid alpha format, expected percentage: {alpha_str}"));
    }
    
    let alpha_percent = alpha_str
        .strip_suffix('%')
        .and_then(|s| s.parse::<f32>().ok())
        .ok_or_else(|| anyhow!("invalid alpha percentage: {alpha_str}"))?;
    
    // Get color based on the first parameter
    let color_part = parts[0].trim();
    let mut color = if color_part == "accent" {
        // If accent is specified, get Windows accent color
        get_accent_color(true)
    } else {
        // Otherwise parse as hex code
        let hex = color_part.strip_prefix("#").unwrap_or(color_part);
        parse_hex(hex)?
    };
    
    // Override transparency
    color.a = alpha_percent / 100.0;
    
    Ok(color)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertical_gradient_90() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Gradient(GradientBrushConfig {
            colors: vec!["#ffffff".to_string(), "#000000".to_string()],
            direction: GradientDirection::Angle("90deg".to_string()),
        });
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Gradient(ref gradient) = color_brush {
            assert!(gradient.direction.start == [0.5, 1.0]);
            assert!(gradient.direction.end == [0.5, 0.0]);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }

    #[test]
    fn test_vertical_gradient_neg90() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Gradient(GradientBrushConfig {
            colors: vec!["#ffffff".to_string(), "#000000".to_string()],
            direction: GradientDirection::Angle("-90deg".to_string()),
        });
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Gradient(ref gradient) = color_brush {
            assert!(gradient.direction.start == [0.5, 0.0]);
            assert!(gradient.direction.end == [0.5, 1.0]);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }

    #[test]
    fn test_gradient_excess_angle() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Gradient(GradientBrushConfig {
            colors: vec!["#ffffff".to_string(), "#000000".to_string()],
            direction: GradientDirection::Angle("-540deg".to_string()),
        });
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Gradient(ref gradient) = color_brush {
            assert!(gradient.direction.start == [1.0, 0.5]);
            assert!(gradient.direction.end == [0.0, 0.5]);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }

    #[test]
    fn test_color_parser_translucent() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Solid("#ffffff80".to_string());
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Solid(ref solid) = color_brush {
            assert!(
                solid.color
                    == D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 128.0 / 255.0
                    }
            );
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }
    
    #[test]
    fn test_with_opacity_accent() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Solid("with_opacity(accent,40%)".to_string());
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Solid(ref solid) = color_brush {
            // We can't test exact color values since accent color depends on Windows settings
            // But we can test that alpha is set correctly
            assert_eq!(solid.color.a, 0.4);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }
    
    #[test]
    fn test_with_opacity_hex_code() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Solid("with_opacity(#ff0000,60%)".to_string());
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Solid(ref solid) = color_brush {
            assert_eq!(solid.color.r, 1.0);
            assert_eq!(solid.color.g, 0.0);
            assert_eq!(solid.color.b, 0.0);
            assert_eq!(solid.color.a, 0.6);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }
    
    #[test]
    fn test_with_opacity_hex_with_alpha_override() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Solid("with_opacity(#ff000080,30%)".to_string());
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Solid(ref solid) = color_brush {
            assert_eq!(solid.color.r, 1.0);
            assert_eq!(solid.color.g, 0.0);
            assert_eq!(solid.color.b, 0.0);
            // Alpha should be overridden to 30%
            assert_eq!(solid.color.a, 0.3);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }
    
    #[test]
    fn test_with_opacity_without_hash() -> anyhow::Result<()> {
        let color_brush_config = ColorBrushConfig::Solid("with_opacity(00ff00,50%)".to_string());
        let color_brush = color_brush_config.to_color_brush(true);

        if let ColorBrush::Solid(ref solid) = color_brush {
            assert_eq!(solid.color.r, 0.0);
            assert_eq!(solid.color.g, 1.0);
            assert_eq!(solid.color.b, 0.0);
            assert_eq!(solid.color.a, 0.5);
        } else {
            panic!("created incorrect color brush");
        }

        Ok(())
    }
}
