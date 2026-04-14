use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::{
    RuntimeBweMode, RuntimeMode, RuntimePlan, RuntimePreference, RuntimeRecoveryPlan,
    RuntimeVideoPipelinePlan, TurnPlan,
};
use crate::policy::types::{CompileError, Owner, Target, TurnSource};
use xbxengine_protocol::{XbxEngineRemoteProfileKindDto, XbxEngineTargetTypeDto};

pub fn compile_runtime(config: &Config, context: &Context) -> Result<RuntimePlan, CompileError> {
    let mode = resolve_runtime_mode(config, context)?;
    let remote_profile = resolve_runtime_remote_profile(context.target);
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
        video_pipeline: compile_video_pipeline(mode, remote_profile),
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

fn resolve_runtime_remote_profile(target: Target) -> XbxEngineRemoteProfileKindDto {
    let target_type = match target {
        Target::Home => XbxEngineTargetTypeDto::Home,
        Target::Cloud => XbxEngineTargetTypeDto::Cloud,
    };
    XbxEngineRemoteProfileKindDto::from_target_type(target_type)
}

fn compile_video_pipeline(
    mode: RuntimeMode,
    remote_profile: XbxEngineRemoteProfileKindDto,
) -> RuntimeVideoPipelinePlan {
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
            jitter_early_emit_enabled: false,
        },
        RuntimeMode::RustOwned => {
            if remote_profile.is_cloud() {
                // Cloud 侧可以放宽 NACK/jitter 等视频管线参数，但 TWCC 反馈节奏仍需保持快反馈。
                // Rust-owned 的 BWE/恢复策略按 100ms 级 feedback 设计，若放慢到 1000ms，
                // 会把 cloud 场景误判成“长期 await/unstable”，进一步放大保守 backoff。
                return RuntimeVideoPipelinePlan {
                    feedback_interval_ms: 100,
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
                    jitter_early_emit_enabled: false,
                };
            }
            RuntimeVideoPipelinePlan {
                feedback_interval_ms: 100,
                // Home + Rust-owned 仍保持低时延，但给 burst loss 留出更可恢复的 NACK/抖动窗口，
                // 避免一次参考链缺口很快坠入长期 await recovery keyframe。
                nack_window_ms: 220,
                nack_burst_count: 6,
                nack_max_age_ms: 64,
                nack_retry_interval_ms: 12,
                nack_max_retry_count: 3,
                jitter_buffer_min_delay_ms: 5,
                jitter_buffer_max_delay_ms: 12,
                jitter_buffer_max_packets: 384,
                idle_timeout_ms: 140,
                late_frame_drop_threshold_ms: 240,
                backlog_drop_threshold_packets: 6,
                jitter_early_emit_enabled: false,
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
            // 适度延后 decoder reset，减少 burst loss 后过快触发 reset/cooldown 抑制链。
            decoder_reset_after_keyframe_wait_ms: 240,
            decoder_reset_request_cooldown_ms: 450,
            reconnect_stall_ms: 2_400,
            stall_recovery_cooldown_ms: 1_600,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{compile_runtime, resolve_runtime_remote_profile};
    use crate::policy::config::Config;
    use crate::policy::context::Context;
    use crate::policy::runtime::RuntimePreference;
    use crate::policy::types::Target;
    use xbxengine_protocol::XbxEngineRemoteProfileKindDto;

    #[test]
    fn runtime_remote_profile_follows_shared_contract_baseline() {
        assert_eq!(
            resolve_runtime_remote_profile(Target::Cloud),
            XbxEngineRemoteProfileKindDto::CloudGaming
        );
        assert_eq!(
            resolve_runtime_remote_profile(Target::Home),
            XbxEngineRemoteProfileKindDto::HomeLanGaming
        );
    }

    #[test]
    fn rust_owned_cloud_aligns_video_pipeline_but_keeps_sidecar_recovery_profile() {
        let mut config = Config::default();
        config.runtime.mode = RuntimePreference::RustOwned;

        let mut context = Context::default();
        context.target = Target::Cloud;
        context.runtime.rust_owned = true;

        let runtime = compile_runtime(&config, &context).expect("compile runtime");

        assert_eq!(runtime.video_pipeline.feedback_interval_ms, 100);
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
        assert_eq!(runtime.video_pipeline.nack_max_age_ms, 64);
        assert_eq!(runtime.video_pipeline.nack_max_retry_count, 3);
        assert_eq!(runtime.video_pipeline.jitter_buffer_max_delay_ms, 12);
        assert_eq!(runtime.video_pipeline.idle_timeout_ms, 140);
        assert_eq!(runtime.video_pipeline.late_frame_drop_threshold_ms, 240);
        assert_eq!(runtime.recovery.first_frame_grace_ms, 1_800);
        assert_eq!(runtime.recovery.keyframe_request_stall_ms, 300);
        assert_eq!(runtime.recovery.decoder_reset_after_keyframe_wait_ms, 240);
        assert_eq!(runtime.recovery.reconnect_stall_ms, 2_400);
    }
}
