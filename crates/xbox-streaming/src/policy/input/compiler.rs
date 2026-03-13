use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::input::{
    EffectiveInputCapabilities, InputCapabilitySource, InputMode, InputPlan, InputPreference,
    MicrophonePreference, TouchPreference,
};
use crate::policy::runtime::RuntimePlan;
use crate::policy::types::{CompileError, Switch};

pub fn compile_input(
    config: &Config,
    context: &Context,
    runtime: &RuntimePlan,
) -> Result<InputPlan, CompileError> {
    let capabilities = interpret_input_capabilities(context);
    let mode = resolve_input_mode(config, context, &capabilities)?;
    let touch = resolve_touch_enabled(config, context, &capabilities);
    let max_touch_points = touch.then_some(10);

    Ok(InputPlan {
        owner: runtime.input,
        mode,
        polling_rate_hz: config.input.polling_rate_hz.max(1),
        vibration: config.input.vibration,
        mouse: matches!(mode, InputMode::NativeMkb),
        keyboard: matches!(mode, InputMode::NativeMkb),
        touch,
        max_touch_points,
        microphone_on_play: matches!(
            config.input.microphone,
            MicrophonePreference::StartWithSession
        ),
    })
}

pub fn resolve_input_mode(
    config: &Config,
    context: &Context,
    capabilities: &EffectiveInputCapabilities,
) -> Result<InputMode, CompileError> {
    match config.input.mode {
        InputPreference::PhysicalGamepad => Ok(InputMode::PhysicalGamepad),
        InputPreference::VirtualGamepad => Ok(InputMode::VirtualGamepad),
        InputPreference::NativeMkb => {
            if can_use_native_mkb(config, context, capabilities) {
                Ok(InputMode::NativeMkb)
            } else {
                Err(CompileError::NativeMkbUnavailable)
            }
        }
        InputPreference::Auto => {
            if can_use_native_mkb(config, context, capabilities) {
                return Ok(InputMode::NativeMkb);
            }
            if config.input.virtual_mkb {
                return Ok(InputMode::VirtualGamepad);
            }
            Ok(InputMode::PhysicalGamepad)
        }
    }
}

pub fn interpret_input_capabilities(context: &Context) -> EffectiveInputCapabilities {
    if context.input_capability.input_config_resolved {
        return EffectiveInputCapabilities {
            source: InputCapabilitySource::InputConfig,
            title_supports_mkb: context.input_capability.input_config_supports_mkb,
            title_supports_touch: context.input_capability.input_config_supports_touch,
            title_supports_native_touch: context
                .input_capability
                .input_config_supports_native_touch,
        };
    }

    EffectiveInputCapabilities {
        source: InputCapabilitySource::Fallback,
        title_supports_mkb: context.input.has_mkb,
        title_supports_touch: context.input.has_touch,
        title_supports_native_touch: context.input.has_native_touch,
    }
}

fn can_use_native_mkb(
    config: &Config,
    context: &Context,
    capabilities: &EffectiveInputCapabilities,
) -> bool {
    if !capabilities.title_supports_mkb || !context.runtime.native_mkb {
        return false;
    }

    matches!(config.input.native_mkb, Switch::On)
        || matches!(
            (config.input.mode, config.input.native_mkb),
            (InputPreference::NativeMkb, _) | (InputPreference::Auto, Switch::Auto)
        )
}

fn resolve_touch_enabled(
    config: &Config,
    context: &Context,
    capabilities: &EffectiveInputCapabilities,
) -> bool {
    if !context.runtime.touch_surface {
        return false;
    }

    match config.input.touch {
        TouchPreference::FollowTitle => capabilities.title_supports_touch,
        TouchPreference::On => true,
        TouchPreference::Off => false,
    }
}
