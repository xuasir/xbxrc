use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::{
    api::runtime::XbxEngineNegotiationRuntimeConfig,
    transport::webrtc::startup_recovery::SessionPhase, XbxEngineWebRtcRuntimeConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenarioPolicyProfileKind {
    HomeLanGaming,
    CloudGaming,
    RelayGaming,
}

impl ScenarioPolicyProfileKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ScenarioPolicyProfileKind::HomeLanGaming => "homeLanGaming",
            ScenarioPolicyProfileKind::CloudGaming => "cloudGaming",
            ScenarioPolicyProfileKind::RelayGaming => "relayGaming",
        }
    }

    pub(crate) fn reason_prefix(self) -> &'static str {
        match self {
            // 保留 home 侧既有 reason 前缀，避免 trace/测试把“home 语义切换”
            // 误判成策略行为回归。
            ScenarioPolicyProfileKind::HomeLanGaming => "direct",
            ScenarioPolicyProfileKind::CloudGaming => "cloud",
            ScenarioPolicyProfileKind::RelayGaming => "relay",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveryScenarioProfile {
    pub(crate) kind: ScenarioPolicyProfileKind,
    pub(crate) startup_fast_reset_enabled: bool,
    pub(crate) startup_low_quality_retry_delay_ms: u64,
    pub(crate) startup_low_quality_floor_kbps: f64,
    pub(crate) startup_low_quality_recovered_kbps: f64,
    pub(crate) decoder_backend_failure_min_consecutive_failures: u32,
    pub(crate) decoder_backend_failure_recent_window_ms: f64,
    pub(crate) decoder_backend_failure_max_packet_age_ms: f64,
    pub(crate) decoder_backend_failure_min_twcc_delivery_ratio: f64,
    pub(crate) decoder_backend_failure_max_twcc_loss_ratio: f64,
    pub(crate) decoder_backend_failure_min_reset_spacing_ms: f64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TransportBweScenarioProfile {
    pub(crate) kind: ScenarioPolicyProfileKind,
    pub(crate) stable_feedback_interval_ms: f64,
    pub(crate) stable_feedback_min_packets: u16,
    pub(crate) receive_headroom_factor: f64,
    pub(crate) preferred_floor_kbps: Option<u32>,
    pub(crate) operating_ceiling_kbps: Option<u32>,
    pub(crate) peak_enter_kbps: Option<u32>,
    pub(crate) peak_ceiling_kbps: Option<u32>,
    pub(crate) ramp_up_step_kbps: u32,
    pub(crate) fast_ramp_up_step_kbps: u32,
    pub(crate) severe_loss_threshold: f64,
    pub(crate) severe_delivery_threshold: f64,
    pub(crate) congestion_loss_threshold: f64,
    pub(crate) congestion_delivery_threshold: f64,
    pub(crate) mild_loss_threshold: f64,
    pub(crate) mild_delivery_threshold: f64,
    pub(crate) severe_cooldown_ticks: u8,
    pub(crate) congestion_cooldown_ticks: u8,
    pub(crate) mild_cooldown_ticks: u8,
    pub(crate) cooldown_recovery_step_kbps: Option<u32>,
}

impl TransportBweScenarioProfile {
    pub(crate) fn reason(self, suffix: &str) -> String {
        format!("twcc-gcc-{}-{suffix}", self.kind.reason_prefix())
    }

    pub(crate) fn preferred_floor(self, fallback_floor_kbps: u32) -> u32 {
        self.preferred_floor_kbps.unwrap_or(fallback_floor_kbps)
    }

    pub(crate) fn peak_ceiling(self, fallback_ceiling_kbps: u32) -> u32 {
        self.peak_ceiling_kbps.unwrap_or(fallback_ceiling_kbps)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OfferVideoScenarioProfile {
    pub(crate) low_min_bitrate_kbps: u32,
    pub(crate) low_start_bitrate_cap_kbps: u32,
    pub(crate) mid_min_bitrate_kbps: u32,
    pub(crate) mid_start_bitrate_cap_kbps: u32,
    pub(crate) high_min_bitrate_kbps: u32,
    pub(crate) high_start_bitrate_cap_kbps: u32,
}

pub(crate) struct ScenarioPolicyResolver;

impl ScenarioPolicyResolver {
    // `session_target_type` 是主语义来源；transport_path 只负责把 Home 场景细分成
    // LAN 直连或 Relay，不能再反向决定 Cloud/Home。
    pub(crate) fn resolve_kind(
        session_target_type: Option<&XbxEngineTargetTypeDto>,
        transport_path: Option<&str>,
    ) -> ScenarioPolicyProfileKind {
        match session_target_type {
            Some(XbxEngineTargetTypeDto::Cloud) => ScenarioPolicyProfileKind::CloudGaming,
            Some(XbxEngineTargetTypeDto::Home) => {
                if is_relay_path(transport_path) {
                    ScenarioPolicyProfileKind::RelayGaming
                } else {
                    ScenarioPolicyProfileKind::HomeLanGaming
                }
            }
            None => {
                if is_relay_path(transport_path) {
                    ScenarioPolicyProfileKind::RelayGaming
                } else {
                    ScenarioPolicyProfileKind::HomeLanGaming
                }
            }
        }
    }

    pub(crate) fn resolve_recovery_profile(
        session_target_type: Option<&XbxEngineTargetTypeDto>,
        transport_path: Option<&str>,
    ) -> RecoveryScenarioProfile {
        match Self::resolve_kind(session_target_type, transport_path) {
            ScenarioPolicyProfileKind::HomeLanGaming => RecoveryScenarioProfile {
                kind: ScenarioPolicyProfileKind::HomeLanGaming,
                startup_fast_reset_enabled: true,
                startup_low_quality_retry_delay_ms: 320,
                startup_low_quality_floor_kbps: 8_000.0,
                startup_low_quality_recovered_kbps: 12_000.0,
                decoder_backend_failure_min_consecutive_failures: 3,
                decoder_backend_failure_recent_window_ms: 1_200.0,
                decoder_backend_failure_max_packet_age_ms: 400.0,
                decoder_backend_failure_min_twcc_delivery_ratio: 0.92,
                decoder_backend_failure_max_twcc_loss_ratio: 0.08,
                decoder_backend_failure_min_reset_spacing_ms: 800.0,
            },
            ScenarioPolicyProfileKind::CloudGaming => RecoveryScenarioProfile {
                kind: ScenarioPolicyProfileKind::CloudGaming,
                startup_fast_reset_enabled: false,
                startup_low_quality_retry_delay_ms: 650,
                startup_low_quality_floor_kbps: 14_000.0,
                startup_low_quality_recovered_kbps: 20_000.0,
                decoder_backend_failure_min_consecutive_failures: 3,
                decoder_backend_failure_recent_window_ms: 1_200.0,
                decoder_backend_failure_max_packet_age_ms: 400.0,
                decoder_backend_failure_min_twcc_delivery_ratio: 0.92,
                decoder_backend_failure_max_twcc_loss_ratio: 0.08,
                decoder_backend_failure_min_reset_spacing_ms: 800.0,
            },
            ScenarioPolicyProfileKind::RelayGaming => RecoveryScenarioProfile {
                kind: ScenarioPolicyProfileKind::RelayGaming,
                startup_fast_reset_enabled: false,
                startup_low_quality_retry_delay_ms: 650,
                startup_low_quality_floor_kbps: 6_000.0,
                startup_low_quality_recovered_kbps: 10_000.0,
                decoder_backend_failure_min_consecutive_failures: 3,
                decoder_backend_failure_recent_window_ms: 1_200.0,
                decoder_backend_failure_max_packet_age_ms: 400.0,
                decoder_backend_failure_min_twcc_delivery_ratio: 0.92,
                decoder_backend_failure_max_twcc_loss_ratio: 0.08,
                decoder_backend_failure_min_reset_spacing_ms: 800.0,
            },
        }
    }

    pub(crate) fn resolve_transport_bwe_profile(
        config: &XbxEngineWebRtcRuntimeConfig,
        session_target_type: Option<&XbxEngineTargetTypeDto>,
        transport_path: Option<&str>,
        phase: SessionPhase,
    ) -> TransportBweScenarioProfile {
        match Self::resolve_kind(session_target_type, transport_path) {
            ScenarioPolicyProfileKind::HomeLanGaming => TransportBweScenarioProfile {
                kind: ScenarioPolicyProfileKind::HomeLanGaming,
                stable_feedback_interval_ms: 220.0,
                stable_feedback_min_packets: match phase {
                    SessionPhase::Startup => 6,
                    SessionPhase::Steady | SessionPhase::Recovering => 8,
                },
                receive_headroom_factor: match phase {
                    SessionPhase::Startup => 1.18,
                    SessionPhase::Steady => 1.28,
                    SessionPhase::Recovering => 1.24,
                },
                preferred_floor_kbps: Some(15_000),
                operating_ceiling_kbps: Some(23_000),
                peak_enter_kbps: Some(26_000),
                peak_ceiling_kbps: Some(40_000),
                ramp_up_step_kbps: match phase {
                    SessionPhase::Startup => 4_000,
                    SessionPhase::Steady => 5_000,
                    SessionPhase::Recovering => 6_000,
                },
                fast_ramp_up_step_kbps: match phase {
                    SessionPhase::Startup => 6_000,
                    SessionPhase::Steady => 8_000,
                    SessionPhase::Recovering => 8_000,
                },
                severe_loss_threshold: 0.08,
                severe_delivery_threshold: 0.92,
                congestion_loss_threshold: 0.03,
                congestion_delivery_threshold: 0.96,
                mild_loss_threshold: 0.01,
                mild_delivery_threshold: 0.985,
                severe_cooldown_ticks: 2,
                congestion_cooldown_ticks: 1,
                mild_cooldown_ticks: 1,
                cooldown_recovery_step_kbps: Some(match phase {
                    SessionPhase::Startup => 1_000,
                    SessionPhase::Steady => 2_000,
                    SessionPhase::Recovering => 3_000,
                }),
            },
            ScenarioPolicyProfileKind::CloudGaming => TransportBweScenarioProfile {
                kind: ScenarioPolicyProfileKind::CloudGaming,
                stable_feedback_interval_ms: 230.0,
                stable_feedback_min_packets: match phase {
                    SessionPhase::Startup => 8,
                    SessionPhase::Steady | SessionPhase::Recovering => 10,
                },
                receive_headroom_factor: match phase {
                    SessionPhase::Startup => 1.10,
                    SessionPhase::Steady => 1.14,
                    SessionPhase::Recovering => 1.12,
                },
                preferred_floor_kbps: Some(20_000),
                operating_ceiling_kbps: Some(25_000),
                peak_enter_kbps: Some(26_000),
                peak_ceiling_kbps: Some(30_000),
                ramp_up_step_kbps: match phase {
                    SessionPhase::Startup => 2_500,
                    SessionPhase::Steady => 3_000,
                    SessionPhase::Recovering => 3_500,
                },
                fast_ramp_up_step_kbps: match phase {
                    SessionPhase::Startup => 4_000,
                    SessionPhase::Steady => 5_000,
                    SessionPhase::Recovering => 5_000,
                },
                severe_loss_threshold: 0.12,
                severe_delivery_threshold: 0.85,
                congestion_loss_threshold: 0.05,
                congestion_delivery_threshold: 0.92,
                mild_loss_threshold: 0.02,
                mild_delivery_threshold: 0.97,
                severe_cooldown_ticks: 4,
                congestion_cooldown_ticks: 2,
                mild_cooldown_ticks: 1,
                cooldown_recovery_step_kbps: Some(match phase {
                    SessionPhase::Startup => 1_000,
                    SessionPhase::Steady => 1_500,
                    SessionPhase::Recovering => 2_000,
                }),
            },
            ScenarioPolicyProfileKind::RelayGaming => TransportBweScenarioProfile {
                kind: ScenarioPolicyProfileKind::RelayGaming,
                stable_feedback_interval_ms: 200.0,
                stable_feedback_min_packets: 12,
                receive_headroom_factor: 1.08,
                preferred_floor_kbps: None,
                operating_ceiling_kbps: None,
                peak_enter_kbps: None,
                peak_ceiling_kbps: None,
                ramp_up_step_kbps: config.remb_ramp_up_step_kbps,
                fast_ramp_up_step_kbps: config.remb_ramp_up_step_kbps,
                severe_loss_threshold: 0.12,
                severe_delivery_threshold: 0.82,
                congestion_loss_threshold: 0.05,
                congestion_delivery_threshold: 0.92,
                mild_loss_threshold: 0.02,
                mild_delivery_threshold: 0.97,
                severe_cooldown_ticks: 12,
                congestion_cooldown_ticks: 8,
                mild_cooldown_ticks: 4,
                cooldown_recovery_step_kbps: None,
            },
        }
    }

    pub(crate) fn classify_bitrate_band(
        session_target_type: Option<&XbxEngineTargetTypeDto>,
        transport_path: Option<&str>,
        actual_video_bitrate_kbps: Option<f64>,
    ) -> Option<&'static str> {
        let bitrate_kbps = actual_video_bitrate_kbps?;
        match Self::resolve_kind(session_target_type, transport_path) {
            ScenarioPolicyProfileKind::HomeLanGaming => Some(if bitrate_kbps <= 0.0 {
                "paused"
            } else if bitrate_kbps < 8_000.0 {
                "startupLow"
            } else if bitrate_kbps < 15_000.0 {
                "belowOperatingRange"
            } else if bitrate_kbps <= 23_000.0 {
                "operatingRange"
            } else if bitrate_kbps <= 35_000.0 {
                "peakRange"
            } else {
                "abovePeakRange"
            }),
            ScenarioPolicyProfileKind::CloudGaming => Some(if bitrate_kbps <= 0.0 {
                "paused"
            } else if bitrate_kbps < 14_000.0 {
                "startupLow"
            } else if bitrate_kbps < 20_000.0 {
                "belowOperatingRange"
            } else if bitrate_kbps <= 25_000.0 {
                "operatingRange"
            } else if bitrate_kbps <= 30_000.0 {
                "peakRange"
            } else {
                "abovePeakRange"
            }),
            ScenarioPolicyProfileKind::RelayGaming => None,
        }
    }

    pub(crate) fn resolve_offer_profile(
        session_target_type: Option<&XbxEngineTargetTypeDto>,
    ) -> OfferVideoScenarioProfile {
        match Self::resolve_kind(session_target_type, None) {
            ScenarioPolicyProfileKind::HomeLanGaming => OfferVideoScenarioProfile {
                low_min_bitrate_kbps: 3_000,
                low_start_bitrate_cap_kbps: 10_000,
                mid_min_bitrate_kbps: 5_000,
                mid_start_bitrate_cap_kbps: 20_000,
                high_min_bitrate_kbps: 8_000,
                high_start_bitrate_cap_kbps: 35_000,
            },
            ScenarioPolicyProfileKind::CloudGaming => OfferVideoScenarioProfile {
                low_min_bitrate_kbps: 4_000,
                low_start_bitrate_cap_kbps: 10_000,
                mid_min_bitrate_kbps: 8_000,
                mid_start_bitrate_cap_kbps: 20_000,
                high_min_bitrate_kbps: 12_000,
                high_start_bitrate_cap_kbps: 25_000,
            },
            ScenarioPolicyProfileKind::RelayGaming => OfferVideoScenarioProfile {
                low_min_bitrate_kbps: 3_000,
                low_start_bitrate_cap_kbps: 8_000,
                mid_min_bitrate_kbps: 5_000,
                mid_start_bitrate_cap_kbps: 15_000,
                high_min_bitrate_kbps: 8_000,
                high_start_bitrate_cap_kbps: 20_000,
            },
        }
    }

    pub(crate) fn resolve_offer_video_constraint_tier(
        negotiation_config: &XbxEngineNegotiationRuntimeConfig,
        session_target_type: Option<&XbxEngineTargetTypeDto>,
    ) -> crate::transport::webrtc::sdp_policy::OfferVideoConstraintTier {
        let profile = Self::resolve_offer_profile(session_target_type);
        let width = negotiation_config.target_resolution_width.max(16);
        let height = negotiation_config.target_resolution_height.max(16);
        let max_frame_size = width.div_ceil(16).saturating_mul(height.div_ceil(16));
        let configured_max_bitrate_kbps = negotiation_config.video_bitrate_kbps.max(1);

        if height <= 720 {
            return crate::transport::webrtc::sdp_policy::OfferVideoConstraintTier {
                max_frame_size,
                min_bitrate_kbps: profile.low_min_bitrate_kbps,
                start_bitrate_kbps: configured_max_bitrate_kbps
                    .min(profile.low_start_bitrate_cap_kbps),
                max_bitrate_kbps: configured_max_bitrate_kbps,
            };
        }

        if height > 1080 || width > 1920 {
            return crate::transport::webrtc::sdp_policy::OfferVideoConstraintTier {
                max_frame_size,
                min_bitrate_kbps: profile.high_min_bitrate_kbps,
                start_bitrate_kbps: configured_max_bitrate_kbps
                    .min(profile.high_start_bitrate_cap_kbps),
                max_bitrate_kbps: configured_max_bitrate_kbps,
            };
        }

        crate::transport::webrtc::sdp_policy::OfferVideoConstraintTier {
            max_frame_size,
            min_bitrate_kbps: profile.mid_min_bitrate_kbps,
            start_bitrate_kbps: configured_max_bitrate_kbps.min(profile.mid_start_bitrate_cap_kbps),
            max_bitrate_kbps: configured_max_bitrate_kbps,
        }
    }
}

fn is_relay_path(transport_path: Option<&str>) -> bool {
    transport_path
        .map(|path| path.to_ascii_lowercase().starts_with("relay"))
        .unwrap_or(false)
}
