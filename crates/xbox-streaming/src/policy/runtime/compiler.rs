use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::{RuntimeMode, RuntimePlan, RuntimePreference, TurnPlan};
use crate::policy::types::{CompileError, Owner, TurnSource};

pub fn compile_runtime(config: &Config, context: &Context) -> Result<RuntimePlan, CompileError> {
    let mode = resolve_runtime_mode(config, context)?;
    let owner = match mode {
        RuntimeMode::WebRtcDirect => Owner::Browser,
        RuntimeMode::RustOwned => Owner::Sidecar,
    };

    let custom = config.runtime.custom_turn.clone();
    let fallback = if context.target.is_home() && config.runtime.home_fallback_turn {
        context.turn.fallback.clone()
    } else {
        None
    };
    let (resolved, source) = if custom.is_some() {
        (custom.clone(), TurnSource::Custom)
    } else if fallback.is_some() {
        (fallback.clone(), TurnSource::Fallback)
    } else {
        (None, TurnSource::None)
    };

    Ok(RuntimePlan {
        mode,
        transport: owner,
        decode: owner,
        render: owner,
        input: owner,
        microphone: owner,
        turn: TurnPlan {
            custom,
            fallback,
            resolved,
            source,
        },
    })
}

pub fn resolve_runtime_mode(
    config: &Config,
    context: &Context,
) -> Result<RuntimeMode, CompileError> {
    match config.runtime.mode {
        RuntimePreference::WebRtcDirect => context
            .runtime
            .browser_webrtc
            .then_some(RuntimeMode::WebRtcDirect)
            .ok_or(CompileError::RuntimeUnavailable),
        RuntimePreference::RustOwned => context
            .runtime
            .rust_owned
            .then_some(RuntimeMode::RustOwned)
            .ok_or(CompileError::RuntimeUnavailable),
        RuntimePreference::Auto => {
            if context.runtime.prefer_browser && context.runtime.browser_webrtc {
                return Ok(RuntimeMode::WebRtcDirect);
            }
            if context.runtime.rust_owned {
                return Ok(RuntimeMode::RustOwned);
            }
            if context.runtime.browser_webrtc {
                return Ok(RuntimeMode::WebRtcDirect);
            }
            Err(CompileError::RuntimeUnavailable)
        }
    }
}
