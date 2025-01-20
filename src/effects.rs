use anyhow::Context;
use serde::Deserialize;
use std::slice;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COMPOSITE_MODE_DESTINATION_OUT;
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D12DAffineTransform, CLSID_D2D1Composite, CLSID_D2D1GaussianBlur, CLSID_D2D1Opacity,
    CLSID_D2D1Shadow, Common::D2D1_COMPOSITE_MODE_SOURCE_OVER, ID2D1Bitmap1, ID2D1CommandList,
    ID2D1DeviceContext7, ID2D1Effect, D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX,
    D2D1_DIRECTIONALBLUR_OPTIMIZATION_SPEED, D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_OPACITY_PROP_OPACITY, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
    D2D1_PROPERTY_TYPE_MATRIX_3X2, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION,
    D2D1_SHADOW_PROP_OPTIMIZATION,
};

use crate::config::{serde_default_bool, serde_default_f32};

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EffectsConfig {
    #[serde(default)]
    pub active: Vec<EffectParamsConfig>,
    #[serde(default)]
    pub inactive: Vec<EffectParamsConfig>,
    #[serde(default = "serde_default_bool::<true>")]
    pub enabled: bool,
}

impl EffectsConfig {
    pub fn to_effects(&self) -> Effects {
        if self.enabled {
            Effects {
                active: self
                    .active
                    .iter()
                    .map(|config| config.to_effect_params())
                    .collect(),
                inactive: self
                    .inactive
                    .clone()
                    .iter()
                    .map(|config| config.to_effect_params())
                    .collect(),
                ..Default::default()
            }
        } else {
            Effects::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct Effects {
    pub active: Vec<EffectParams>,
    pub inactive: Vec<EffectParams>,
    pub active_command_list: Option<ID2D1CommandList>,
    pub inactive_command_list: Option<ID2D1CommandList>,
}

impl Effects {
    pub fn is_enabled(&self) -> bool {
        !self.active.is_empty() || !self.inactive.is_empty()
    }

    pub fn get_current_vec(&self, is_active_window: bool) -> &Vec<EffectParams> {
        match is_active_window {
            true => &self.active,
            false => &self.inactive,
        }
    }

    pub fn get_current_command_list(
        &self,
        is_active_window: bool,
    ) -> anyhow::Result<&ID2D1CommandList> {
        match is_active_window {
            true => self
                .active_command_list
                .as_ref()
                .context("could not get active_command_list"),
            false => self
                .inactive_command_list
                .as_ref()
                .context("could not get inactive_command_list"),
        }
    }

    pub fn create_command_lists_if_enabled(
        &mut self,
        d2d_context: &ID2D1DeviceContext7,
        border_bitmap: &ID2D1Bitmap1,
        mask_bitmap: &ID2D1Bitmap1,
    ) -> anyhow::Result<()> {
        // If not enabled, then ignore everything here
        if !self.is_enabled() {
            return Ok(());
        }

        let create_single_list =
            |effect_params_vec: &Vec<EffectParams>| -> anyhow::Result<ID2D1CommandList> {
                unsafe {
                    // Open a command list to record draw operations
                    let command_list = d2d_context
                        .CreateCommandList()
                        .context("d2d_context.CreateCommandList()")?;

                    // Set the command list as the target so we can begin recording
                    d2d_context.SetTarget(&command_list);

                    // Create a vec to store the output effects
                    let mut effects_vec: Vec<ID2D1Effect> = Vec::new();

                    for effect_params in effect_params_vec.iter() {
                        let effect = match effect_params.effect_type {
                            EffectType::Glow => {
                                let blur_effect = d2d_context
                                    .CreateEffect(&CLSID_D2D1GaussianBlur)
                                    .context("blur_effect")?;
                                blur_effect.SetInput(0, border_bitmap, false);
                                blur_effect
                                    .SetValue(
                                        D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                                        D2D1_PROPERTY_TYPE_FLOAT,
                                        &effect_params.std_dev.to_le_bytes(),
                                    )
                                    .context("blur_effect.SetValue() std deviation")?;
                                blur_effect
                                    .SetValue(
                                        D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
                                        D2D1_PROPERTY_TYPE_ENUM,
                                        &D2D1_DIRECTIONALBLUR_OPTIMIZATION_SPEED.0.to_le_bytes(),
                                    )
                                    .context("blur_effect.SetValue() optimization")?;

                                blur_effect
                            }
                            EffectType::Shadow => {
                                let shadow_effect = d2d_context
                                    .CreateEffect(&CLSID_D2D1Shadow)
                                    .context("shadow_effect")?;
                                shadow_effect.SetInput(0, border_bitmap, false);
                                shadow_effect
                                    .SetValue(
                                        D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
                                        D2D1_PROPERTY_TYPE_FLOAT,
                                        &effect_params.std_dev.to_le_bytes(),
                                    )
                                    .context("shadow_effect.SetValue() std deviation")?;
                                shadow_effect
                                    .SetValue(
                                        D2D1_SHADOW_PROP_OPTIMIZATION.0 as u32,
                                        D2D1_PROPERTY_TYPE_ENUM,
                                        &D2D1_DIRECTIONALBLUR_OPTIMIZATION_SPEED.0.to_le_bytes(),
                                    )
                                    .context("shadow_effect.SetValue() optimization")?;

                                shadow_effect
                            }
                        };

                        let effect_with_opacity = d2d_context
                            .CreateEffect(&CLSID_D2D1Opacity)
                            .context("effect_with_opacity")?;
                        effect_with_opacity.SetInput(
                            0,
                            &effect
                                .GetOutput()
                                .context("could not get _ effect output")?,
                            false,
                        );
                        effect_with_opacity
                            .SetValue(
                                D2D1_OPACITY_PROP_OPACITY.0 as u32,
                                D2D1_PROPERTY_TYPE_FLOAT,
                                &effect_params.opacity.to_le_bytes(),
                            )
                            .context("effect_with_opacity.SetValue()")?;

                        let effect_with_opacity_translation = d2d_context
                            .CreateEffect(&CLSID_D2D12DAffineTransform)
                            .context("effect_with_opacity_translation")?;
                        effect_with_opacity_translation.SetInput(
                            0,
                            &effect_with_opacity
                                .GetOutput()
                                .context("could not get effect_with_opacity output")?,
                            false,
                        );
                        let translation_matrix = Matrix3x2::translation(
                            effect_params.translation.x,
                            effect_params.translation.y,
                        );
                        let translation_matrix_bytes: &[u8] = slice::from_raw_parts(
                            &translation_matrix as *const Matrix3x2 as *const u8,
                            size_of::<Matrix3x2>(),
                        );
                        effect_with_opacity_translation
                            .SetValue(
                                D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX.0 as u32,
                                D2D1_PROPERTY_TYPE_MATRIX_3X2,
                                translation_matrix_bytes,
                            )
                            .context("effect_with_opacity_translation.SetValue()")?;

                        effects_vec.push(effect_with_opacity_translation);
                    }

                    // Create a composite effect and link it to the above effects
                    let composite_effect = d2d_context
                        .CreateEffect(&CLSID_D2D1Composite)
                        .context("composite_effect")?;
                    composite_effect
                        .SetInputCount(effects_vec.len() as u32 + 1)
                        .context("could not set composite effect input count")?;

                    for (index, effect) in effects_vec.iter().enumerate() {
                        composite_effect.SetInput(
                            index as u32,
                            &effect
                                .GetOutput()
                                .context("could not get effect output: {index}")?,
                            false,
                        );
                    }
                    composite_effect.SetInput(effects_vec.len() as u32, border_bitmap, false);

                    // Begin recording commands to the command list
                    d2d_context.BeginDraw();
                    d2d_context.Clear(None);

                    // Record the composite effect
                    d2d_context.DrawImage(
                        &composite_effect
                            .GetOutput()
                            .context("could not get composite output")?,
                        None,
                        None,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    );

                    // Use COMPOSITE_MODE_DESTINATION_OUT to inverse mask out the inner rect
                    d2d_context.DrawImage(
                        mask_bitmap,
                        None,
                        None,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        D2D1_COMPOSITE_MODE_DESTINATION_OUT,
                    );

                    d2d_context.EndDraw(None, None)?;

                    // Close the command list to tell it we are done recording
                    command_list.Close().context("command_list.Close()")?;

                    Ok(command_list)
                }
            };

        let active_command_list =
            create_single_list(&self.active).context("active_command_list")?;
        let inactive_command_list =
            create_single_list(&self.inactive).context("inactive_command_list")?;

        self.active_command_list = Some(active_command_list);
        self.inactive_command_list = Some(inactive_command_list);

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EffectParamsConfig {
    #[serde(alias = "type")]
    pub effect_type: EffectType,
    #[serde(default = "serde_default_f32::<8>")]
    pub std_dev: f32,
    #[serde(default = "serde_default_f32::<1>")]
    pub opacity: f32,
    #[serde(default)]
    pub translation: Translation,
}

impl EffectParamsConfig {
    pub fn to_effect_params(&self) -> EffectParams {
        EffectParams {
            effect_type: self.effect_type,
            opacity: self.opacity,
            std_dev: self.std_dev,
            translation: self.translation,
        }
    }
}

// TODO: if i dont add any additional struct fields, then i don't need EffectParamsConfig
#[derive(Debug, Clone)]
pub struct EffectParams {
    pub effect_type: EffectType,
    pub std_dev: f32,
    pub opacity: f32,
    pub translation: Translation,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub enum EffectType {
    Glow,
    Shadow,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq)]
pub struct Translation {
    pub x: f32,
    pub y: f32,
}
