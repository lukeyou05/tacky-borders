pub struct EffectsConfig {
    pub enabled: bool,
}

pub struct EffectParamsConfig {
    pub effect_type: EffectType,
    pub std_dev: f32,
}

pub enum EffectType {
    Glow,
    Shadow,
}
