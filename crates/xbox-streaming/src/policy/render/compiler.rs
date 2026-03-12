use crate::policy::config::Config;
use crate::policy::render::{RenderDisplayOptions, RenderPlan};

pub fn compile_render(config: &Config) -> RenderPlan {
    RenderPlan {
        enable_audio_control: config.render.enable_audio_control,
        video_format: config
            .render
            .video_format
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        display_options: RenderDisplayOptions {
            sharpness: config.render.display_options.sharpness,
            saturation: config.render.display_options.saturation,
            contrast: config.render.display_options.contrast,
            brightness: config.render.display_options.brightness,
        },
    }
}
