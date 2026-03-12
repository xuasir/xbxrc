use crate::policy::types::{CompileError, Switch};
use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::RuntimePlan;
use crate::policy::input::{InputMode, InputPlan, InputPreference, MicrophonePreference, TouchPreference};

pub fn compile_input(
    config: &Config,
    context: &Context,
    runtime: &RuntimePlan,
) -> Result<InputPlan, CompileError> {
    let mode = resolve_input_mode(config, context)?;
    let touch = resolve_touch_enabled(config, context);
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
) -> Result<InputMode, CompileError> {
    match config.input.mode {
        InputPreference::PhysicalGamepad => Ok(InputMode::PhysicalGamepad),
        InputPreference::VirtualGamepad => Ok(InputMode::VirtualGamepad),
        InputPreference::NativeMkb => {
            if can_use_native_mkb(config, context) {
                Ok(InputMode::NativeMkb)
            } else {
                Err(CompileError::NativeMkbUnavailable)
            }
        }
        InputPreference::Auto => {
            if can_use_native_mkb(config, context) {
                return Ok(InputMode::NativeMkb);
            }
            if config.input.virtual_mkb {
                return Ok(InputMode::VirtualGamepad);
            }
            Ok(InputMode::PhysicalGamepad)
        }
    }
}

fn can_use_native_mkb(config: &Config, context: &Context) -> bool {
    if !context.input.has_mkb || !context.runtime.native_mkb {
        return false;
    }

    matches!(config.input.native_mkb, Switch::On)
        || matches!(
            (config.input.mode, config.input.native_mkb),
            (InputPreference::NativeMkb, _) | (InputPreference::Auto, Switch::Auto)
        )
}

fn resolve_touch_enabled(config: &Config, context: &Context) -> bool {
    if !context.runtime.touch_surface {
        return false;
    }

    match config.input.touch {
        TouchPreference::FollowTitle => context.input.has_touch,
        TouchPreference::On => true,
        TouchPreference::Off => false,
    }
}
