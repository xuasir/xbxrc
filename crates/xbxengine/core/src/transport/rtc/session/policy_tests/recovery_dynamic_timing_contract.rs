//! 动态 RTT 恢复时序合同测试：NACK 首发窗与 survival、profile 静态回退、FIR 仅 Cloud、H264 参数集缓存 bootstrap。
//! 对应设计说明见 `docs/rfcs/2026-05-14-dynamic-rtt-aware-recovery-timing.md`。

use hex_literal::hex;

use crate::media::video::h264::inspection::H264AccessUnitInspector;
use crate::transport::rtc::recovery::policy::{
    RecoveryTimingRttParams, ScenarioPolicyProfileKind, ScenarioPolicyResolver,
};
use crate::transport::rtc::recovery::timing::{
    merge_nack_admission_deadline_with_dynamic_timeout, resolve_recovery_dynamic_timing_with_rtt,
};
#[test]
fn home_wan_supply_gap_does_not_escalate_before_dynamic_first_attempt_timeout() {
    let merged = merge_nack_admission_deadline_with_dynamic_timeout(
        1_000.0,
        1_020.0,
        "supply",
        120.0,
        Some(2_500.0),
    );
    assert!(
        merged >= 1_120.0,
        "高价值缺口 admission deadline 至少覆盖一轮动态 NACK 超时"
    );
}

#[test]
fn continuation_only_waits_dynamic_patience_window_before_pli_refresh() {
    let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::RelayGaming,
    );
    let t100 = resolve_recovery_dynamic_timing_with_rtt(100.0, profile);
    let t200 = resolve_recovery_dynamic_timing_with_rtt(200.0, profile);
    assert!(t200.continuation_patience_window_ms > t100.continuation_patience_window_ms);
}

#[test]
fn timing_rtt_shape_static_pli_when_dim_absent() {
    let mut profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::HomeLanGaming,
    );
    profile.timing_rtt = Some(RecoveryTimingRttParams::default());
    let t = resolve_recovery_dynamic_timing_with_rtt(10.0, profile);
    assert!(
        (t.pli_refresh_interval_ms - profile.pli_refresh_interval_ms).abs() < 0.01,
        "timing_rtt 存在但 pli_refresh 维度未配置时回退 profile 静态 PLI 间隔"
    );
}

#[test]
fn recovery_profile_enables_dynamic_pli_and_fir_timing_dimensions() {
    let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::CloudGaming,
    );
    let t100 = resolve_recovery_dynamic_timing_with_rtt(100.0, profile);
    let t200 = resolve_recovery_dynamic_timing_with_rtt(200.0, profile);
    assert!(t200.pli_refresh_interval_ms > t100.pli_refresh_interval_ms);
    assert!(t200.fir_retry_interval_ms > t100.fir_retry_interval_ms);
}

#[test]
fn bootstrap_missing_sps_uses_cached_parameter_sets_when_config_unchanged() {
    let inspector = H264AccessUnitInspector::new();
    let bootstrap_payload = hex!(
        "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
         03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
         01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
         4E 7F 87 DF"
    );
    let bootstrap = inspector
        .inspect_access_unit(&bootstrap_payload)
        .expect("bootstrap inspection");
    bootstrap.commit();

    let idr_without_sets = hex!("00 00 00 01 65 88 81 00 05 4E 7F 87 DF");
    let inspection = inspector
        .inspect_access_unit(&idr_without_sets)
        .expect("idr inspection");

    assert!(inspection.bootstrap_ready);
    assert_eq!(inspection.bootstrap_reject_reason, None);
    assert!(inspection.parameter_sets.is_some());
    assert!(!inspection.has_inband_sps);
    assert!(!inspection.has_inband_pps);
}
