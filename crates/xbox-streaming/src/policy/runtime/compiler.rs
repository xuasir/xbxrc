use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::{
    RuntimeBweMode, RuntimeMode, RuntimePlan, RuntimePreference, RuntimeRecoveryPlan,
    RuntimeVideoPipelinePlan, TurnPlan,
};
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
        bwe_mode: compile_bwe_mode(mode),
        forced_remb_kbps: compile_forced_remb_kbps(mode),
        adaptive_remb_enabled: false,
        remb_floor_kbps: compile_remb_floor_kbps(mode),
        remb_ceiling_kbps: compile_remb_ceiling_kbps(mode),
        remb_ramp_up_step_kbps: compile_remb_ramp_up_step_kbps(mode),
        remb_ramp_down_factor: compile_remb_ramp_down_factor(mode),
        video_pipeline: compile_video_pipeline(mode),
        recovery: compile_recovery(mode),
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

fn compile_forced_remb_kbps(mode: RuntimeMode) -> Option<u32> {
    match mode {
        RuntimeMode::WebRtcDirect => Some(50_000),
        RuntimeMode::RustOwned => Some(100_000),
    }
}

fn compile_bwe_mode(mode: RuntimeMode) -> RuntimeBweMode {
    match mode {
        RuntimeMode::WebRtcDirect => RuntimeBweMode::FixedRemb,
        RuntimeMode::RustOwned => RuntimeBweMode::Hybrid,
    }
}

fn compile_remb_floor_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 8_000,
        RuntimeMode::RustOwned => 12_000,
    }
}

fn compile_remb_ceiling_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 50_000,
        RuntimeMode::RustOwned => 100_000,
    }
}

fn compile_remb_ramp_up_step_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 2_000,
        RuntimeMode::RustOwned => 4_000,
    }
}

fn compile_remb_ramp_down_factor(mode: RuntimeMode) -> u16 {
    match mode {
        RuntimeMode::WebRtcDirect => 850,
        RuntimeMode::RustOwned => 700,
    }
}

fn compile_video_pipeline(mode: RuntimeMode) -> RuntimeVideoPipelinePlan {
    match mode {
        RuntimeMode::WebRtcDirect => RuntimeVideoPipelinePlan {
            feedback_interval_ms: 1_000,
            nack_window_ms: 400,
            nack_burst_count: 12,
            nack_max_age_ms: 200,
            nack_retry_interval_ms: 60,
            nack_max_retry_count: 5,
            jitter_buffer_min_delay_ms: 20,
            jitter_buffer_max_delay_ms: 30,
            jitter_buffer_max_packets: 1024,
            idle_timeout_ms: 150,
            late_frame_drop_threshold_ms: 500,
            backlog_drop_threshold_packets: 10,
        },
        RuntimeMode::RustOwned => RuntimeVideoPipelinePlan {
            feedback_interval_ms: 250,
            nack_window_ms: 200,
            nack_burst_count: 6,
            nack_max_age_ms: 120,
            nack_retry_interval_ms: 40,
            nack_max_retry_count: 3,
            jitter_buffer_min_delay_ms: 5,
            jitter_buffer_max_delay_ms: 10,
            jitter_buffer_max_packets: 512,
            idle_timeout_ms: 100,
            late_frame_drop_threshold_ms: 250,
            backlog_drop_threshold_packets: 6,
        },
    }
}

fn compile_recovery(mode: RuntimeMode) -> RuntimeRecoveryPlan {
    match mode {
        RuntimeMode::WebRtcDirect => RuntimeRecoveryPlan {
            first_frame_grace_ms: 8_000,
            keyframe_request_stall_ms: 1_500,
            keyframe_loss_burst_threshold: 3,
            decoder_reset_after_keyframe_wait_ms: 500,
            decoder_reset_request_cooldown_ms: 1_500,
            reconnect_stall_ms: 4_000,
            stall_recovery_cooldown_ms: 6_000,
        },
        RuntimeMode::RustOwned => RuntimeRecoveryPlan {
            first_frame_grace_ms: 2_500,
            keyframe_request_stall_ms: 450,
            keyframe_loss_burst_threshold: 2,
            decoder_reset_after_keyframe_wait_ms: 150,
            decoder_reset_request_cooldown_ms: 450,
            reconnect_stall_ms: 1_400,
            stall_recovery_cooldown_ms: 2_000,
        },
    }
}
