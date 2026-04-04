    use super::{
        classify_scenario_bitrate_band, resolve_target_remb_kbps,
        resolve_transport_policy_profile_kind, resolve_twcc_gcc_target, SessionPhase,
    };
    use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;
    use crate::{XbxEngineVideoTwccObservation, XbxEngineWebRtcRuntimeConfig};
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    #[test]
    fn profile_kind_prioritizes_session_target_type_over_transport_path() {
        assert_eq!(
            resolve_transport_policy_profile_kind(
                None,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)")
            )
            .as_str(),
            "cloudGaming"
        );
        assert_eq!(
            resolve_transport_policy_profile_kind(
                None,
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)")
            )
            .as_str(),
            "homeLanGaming"
        );
        assert_eq!(
            resolve_transport_policy_profile_kind(
                None,
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay")
            )
            .as_str(),
            "relayGaming"
        );
    }

    #[test]
    fn scenario_bitrate_band_matches_home_and_cloud_operating_ranges() {
        assert_eq!(
            classify_scenario_bitrate_band(
                None,
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                Some(16_000.0),
            ),
            Some("operatingRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                None,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                Some(22_000.0),
            ),
            Some("operatingRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                None,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                Some(28_000.0),
            ),
            Some("peakRange")
        );
        assert_eq!(
            classify_scenario_bitrate_band(
                None,
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay"),
                Some(16_000.0),
            ),
            None
        );
    }

    #[test]
    fn profile_kind_prefers_runtime_baseline_remote_profile() {
        assert_eq!(
            resolve_transport_policy_profile_kind(
                Some("cloudGaming"),
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Relay")
            )
            .as_str(),
            "cloudGaming"
        );
    }

    #[test]
    fn direct_high_rtt_holds_ramp_up_even_when_twcc_is_clean() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 2,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 180_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(25_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            20_000,
            21_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(90.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 20_000);
        assert_eq!(reason, "twcc-gcc-direct-high-rtt-hold");
        assert_eq!(cooldown, 1);
    }

    #[test]
    fn cloud_normal_rtt_does_not_hold_clean_twcc_feedback() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 3,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 220_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(1_000.0),
            arrival_span_ms: Some(1_000.0),
            receive_bitrate_kbps: Some(24_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(200.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 27_360);
        assert_eq!(reason, "twcc-gcc-cloud-ramp-up");
        assert_eq!(cooldown, 0);
    }

    #[test]
    fn cloud_very_high_rtt_still_holds_clean_twcc_feedback() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 4,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 160,
            observed_byte_count: 220_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(1_000.0),
            arrival_span_ms: Some(1_000.0),
            receive_bitrate_kbps: Some(24_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(330.0),
            }),
            &mut cooldown,
        );

        assert_eq!(target, 25_000);
        assert_eq!(reason, "twcc-gcc-cloud-high-rtt-hold");
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn cloud_long_feedback_interval_can_still_be_treated_as_stable() {
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 5,
            source: "local-feedback".to_string(),
            feedback_packet_count: 40,
            covered_sequence_start: 1,
            covered_sequence_end: 826,
            covered_sequence_span: 826,
            observed_packet_count: 798,
            observed_byte_count: 957_600,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(4_042.0),
            arrival_span_ms: Some(999.0),
            receive_bitrate_kbps: Some(1_895.299356754082),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 0.9661016949152542,
            packet_loss_ratio: 0.03389830508474578,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &XbxEngineWebRtcRuntimeConfig::default(),
            28_500,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &XbxEngineWebRtcRuntimeConfig::default(),
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(194.0),
            }),
            &mut cooldown,
        );

        assert_ne!(reason, "twcc-gcc-cloud-unstable-hold");
        assert_eq!(target, 28_500);
    }

    #[test]
    fn direct_feedback_interval_small_spike_does_not_trigger_unstable_hold() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.video_pipeline.feedback_interval_ms = 100;
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 9,
            source: "local-feedback".to_string(),
            feedback_packet_count: 20,
            covered_sequence_start: 1,
            covered_sequence_end: 80,
            covered_sequence_span: 80,
            observed_packet_count: 80,
            observed_byte_count: 96_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(260.0),
            arrival_span_ms: Some(120.0),
            receive_bitrate_kbps: Some(18_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &config,
            20_000,
            18_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &config,
                Some(&XbxEngineTargetTypeDto::Home),
                Some("Direct (host->host)"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(35.0),
            }),
            &mut cooldown,
        );
        assert_ne!(reason, "twcc-gcc-direct-unstable-hold");
        assert!(target >= 20_000);
    }

    #[test]
    fn cloud_delayed_feedback_uses_optimistic_cap_instead_of_severe_backoff() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.video_pipeline.feedback_interval_ms = 100;
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 6,
            source: "local-feedback".to_string(),
            feedback_packet_count: 16,
            covered_sequence_start: 1,
            covered_sequence_end: 160,
            covered_sequence_span: 160,
            observed_packet_count: 64,
            observed_byte_count: 96_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(200.0),
            arrival_span_ms: Some(180.0),
            receive_bitrate_kbps: Some(18_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 0.40,
            packet_loss_ratio: 0.60,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &config,
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &config,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(180.0),
            }),
            &mut cooldown,
        );

        assert_eq!(reason, "twcc-gcc-cloud-severe-optimistic-cap");
        assert!(
            target >= 20_000,
            "target should stay at cloud floor: {target}"
        );
        assert!(
            target <= 25_000,
            "target should be capped, not ramped up: {target}"
        );
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn cloud_first_sparse_local_feedback_without_interval_uses_optimistic_cap() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.video_pipeline.feedback_interval_ms = 100;
        let observation = XbxEngineVideoTwccObservation {
            observation_id: 7,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 100,
            covered_sequence_end: 192,
            covered_sequence_span: 93,
            observed_packet_count: 17,
            observed_byte_count: 20_400,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: None,
            arrival_span_ms: Some(91.0),
            receive_bitrate_kbps: Some(0.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 0.1827956989247312,
            packet_loss_ratio: 0.8172043010752688,
            observed_at_ms: 2.0,
        };
        let mut cooldown = 0;
        let (target, reason) = resolve_twcc_gcc_target(
            &config,
            25_000,
            22_000,
            ScenarioPolicyResolver::resolve_transport_bwe_profile(
                &config,
                Some(&XbxEngineTargetTypeDto::Cloud),
                Some("Direct (host->host)"),
                SessionPhase::Steady,
            ),
            Some(&super::TwccGccInput {
                observation: &observation,
                rtt_ms: Some(180.0),
            }),
            &mut cooldown,
        );

        assert_eq!(reason, "twcc-gcc-cloud-severe-optimistic-cap");
        assert!(
            (20_000..=25_000).contains(&target),
            "target should stay in cloud floor/cap window: {target}"
        );
        assert_eq!(cooldown, 2);
    }

    #[test]
    fn fixed_mode_resolve_target_remb_returns_forced_value() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "fixed".to_string();
        config.forced_remb_kbps = Some(22_000);
        let mut last_sent = 18_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            12_000.0,
            0.0,
            None,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 22_000);
        assert_eq!(decision.reason, "fixed-remb");
        assert_eq!(last_sent, 22_000);
    }

    #[test]
    fn hybrid_without_twcc_high_rtt_holds_current_target() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 18_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            None,
            16_000.0,
            0.0,
            Some(90.0),
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 18_000);
        assert_eq!(decision.reason, "hybrid-high-rtt-hold");
        assert_eq!(last_sent, 18_000);
        assert_eq!(cooldown, 1);
    }

    #[test]
    fn hybrid_without_twcc_sustained_loss_caps_to_actual_headroom() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 20_000;
        let mut cooldown = 0;

        let decision = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            12_000.0,
            0.02,
            None,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            &mut last_sent,
            &mut cooldown,
        );

        assert_eq!(decision.target_kbps, 15_000);
        assert_eq!(decision.reason, "hybrid-sustained-loss-cap");
        assert_eq!(last_sent, 15_000);
        assert_eq!(cooldown, 10);
    }

    #[test]
    fn hybrid_without_twcc_cooldown_then_ramps_up_to_observed_remb() {
        let mut config = XbxEngineWebRtcRuntimeConfig::default();
        config.bwe_mode = "hybrid".to_string();
        let mut last_sent = 14_000;
        let mut cooldown = 1;

        let first = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            14_000.0,
            0.0,
            None,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            &mut last_sent,
            &mut cooldown,
        );
        assert_eq!(first.target_kbps, 14_000);
        assert_eq!(first.reason, "hybrid-ramp-cooldown");
        assert_eq!(cooldown, 0);

        let second = resolve_target_remb_kbps(
            &config,
            Some(24_000),
            14_000.0,
            0.0,
            None,
            None,
            Some(&XbxEngineTargetTypeDto::Home),
            Some("Direct (host->host)"),
            SessionPhase::Steady,
            None,
            &mut last_sent,
            &mut cooldown,
        );
        assert_eq!(second.target_kbps, 14_000 + config.remb_ramp_up_step_kbps);
        assert_eq!(second.reason, "hybrid-ramp-up-observed");
        assert_eq!(last_sent, second.target_kbps);
    }
