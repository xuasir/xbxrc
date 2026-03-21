use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::{
    RuntimeBweMode, RuntimeMode, RuntimePlan, RuntimePreference, RuntimeRecoveryPlan,
    RuntimeVideoPipelinePlan, TurnPlan,
};
use crate::policy::types::{CompileError, Owner, Target, TurnSource};

pub fn compile_runtime(config: &Config, context: &Context) -> Result<RuntimePlan, CompileError> {
    let mode = resolve_runtime_mode(config, context)?;
    let owner = match mode {
        RuntimeMode::WebRtcDirect => Owner::Browser,
        RuntimeMode::RustOwned => Owner::Sidecar,
    };

    let custom = config.runtime.custom_turn.clone();
    let fallback = if config.runtime.home_fallback_turn {
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
        video_pipeline: compile_video_pipeline(mode, context.target),
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
        // 内网主机串流优先榨高码率上限，不让 target 过早贴着保守天花板。
        RuntimeMode::RustOwned => Some(150_000),
    }
}

fn compile_bwe_mode(mode: RuntimeMode) -> RuntimeBweMode {
    match mode {
        RuntimeMode::WebRtcDirect => RuntimeBweMode::FixedRemb,
        RuntimeMode::RustOwned => RuntimeBweMode::TwccGcc,
    }
}

fn compile_remb_floor_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 8_000,
        RuntimeMode::RustOwned => 25_000,
    }
}

fn compile_remb_ceiling_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 50_000,
        RuntimeMode::RustOwned => 150_000,
    }
}

fn compile_remb_ramp_up_step_kbps(mode: RuntimeMode) -> u32 {
    match mode {
        RuntimeMode::WebRtcDirect => 2_000,
        RuntimeMode::RustOwned => 12_000,
    }
}

fn compile_remb_ramp_down_factor(mode: RuntimeMode) -> u16 {
    match mode {
        RuntimeMode::WebRtcDirect => 850,
        RuntimeMode::RustOwned => 900,
    }
}

fn compile_video_pipeline(mode: RuntimeMode, target: Target) -> RuntimeVideoPipelinePlan {
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
        RuntimeMode::RustOwned => {
            if matches!(target, Target::Cloud) {
                // 云游戏基础 RTT 明显高于 home/直连场景，沿用浏览器宽档还不够；
                // 这里继续在浏览器档之上补一个 cloud floor，避免 Rust-owned 在几十毫秒级
                // packet policy 下过早进入 transportExpiredDeadline。
                return RuntimeVideoPipelinePlan {
                    feedback_interval_ms: 1_000,
                    nack_window_ms: 700,
                    nack_burst_count: 16,
                    nack_max_age_ms: 420,
                    nack_retry_interval_ms: 90,
                    nack_max_retry_count: 6,
                    jitter_buffer_min_delay_ms: 28,
                    jitter_buffer_max_delay_ms: 48,
                    jitter_buffer_max_packets: 1536,
                    idle_timeout_ms: 260,
                    late_frame_drop_threshold_ms: 900,
                    backlog_drop_threshold_packets: 14,
                };
            }
            RuntimeVideoPipelinePlan {
                feedback_interval_ms: 100,
                nack_window_ms: 160,
                nack_burst_count: 4,
                nack_max_age_ms: 24,
                nack_retry_interval_ms: 10,
                nack_max_retry_count: 2,
                jitter_buffer_min_delay_ms: 3,
                jitter_buffer_max_delay_ms: 8,
                jitter_buffer_max_packets: 384,
                idle_timeout_ms: 80,
                late_frame_drop_threshold_ms: 180,
                backlog_drop_threshold_packets: 4,
            }
        }
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
            // Rust-owned 首帧/坏参考链恢复仍然需要 sidecar 自己更积极地拉 keyframe/reset，
            // 只放宽 video pipeline，不能把 recovery 也拖到浏览器节奏，否则首帧坏数据会卡太久。
            first_frame_grace_ms: 1_800,
            keyframe_request_stall_ms: 300,
            keyframe_loss_burst_threshold: 2,
            decoder_reset_after_keyframe_wait_ms: 120,
            decoder_reset_request_cooldown_ms: 450,
            reconnect_stall_ms: 2_400,
            stall_recovery_cooldown_ms: 1_600,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::compile_runtime;
    use crate::policy::config::Config;
    use crate::policy::context::Context;
    use crate::policy::runtime::RuntimePreference;
    use crate::policy::types::Target;

    #[test]
    fn rust_owned_cloud_aligns_video_pipeline_but_keeps_sidecar_recovery_profile() {
        let mut config = Config::default();
        config.runtime.mode = RuntimePreference::RustOwned;

        let mut context = Context::default();
        context.target = Target::Cloud;
        context.runtime.rust_owned = true;

        let runtime = compile_runtime(&config, &context).expect("compile runtime");

        assert_eq!(runtime.video_pipeline.feedback_interval_ms, 1_000);
        assert_eq!(runtime.video_pipeline.nack_max_age_ms, 420);
        assert_eq!(runtime.video_pipeline.jitter_buffer_max_delay_ms, 48);
        assert_eq!(runtime.video_pipeline.late_frame_drop_threshold_ms, 900);
        assert_eq!(runtime.recovery.first_frame_grace_ms, 1_800);
        assert_eq!(runtime.recovery.keyframe_request_stall_ms, 300);
        assert_eq!(runtime.recovery.reconnect_stall_ms, 2_400);
    }

    #[test]
    fn rust_owned_home_keeps_low_latency_sidecar_profile() {
        let mut config = Config::default();
        config.runtime.mode = RuntimePreference::RustOwned;

        let mut context = Context::default();
        context.target = Target::Home;
        context.runtime.rust_owned = true;

        let runtime = compile_runtime(&config, &context).expect("compile runtime");

        assert_eq!(runtime.video_pipeline.feedback_interval_ms, 100);
        assert_eq!(runtime.video_pipeline.nack_max_age_ms, 24);
        assert_eq!(runtime.video_pipeline.jitter_buffer_max_delay_ms, 8);
        assert_eq!(runtime.video_pipeline.late_frame_drop_threshold_ms, 180);
        assert_eq!(runtime.recovery.first_frame_grace_ms, 1_800);
        assert_eq!(runtime.recovery.keyframe_request_stall_ms, 300);
        assert_eq!(runtime.recovery.reconnect_stall_ms, 2_400);
    }
}
