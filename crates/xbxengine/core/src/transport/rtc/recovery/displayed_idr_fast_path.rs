//! displayed IDR 已 serving 但供给断裂时的精准快路径（PATH-A/B/C）。
//!
//! 5s 滚动窗 + 硬排除，避免 submit 尖峰 / NACK 前兆误触发 PLI。

use std::collections::VecDeque;

use crate::transport::rtc::recovery::contract::{
    displayed_idr_serving_from_stats, displayed_idr_serving_relaxation_blocked_from_stats,
    fresh_h264_idr_admission_from_stats, is_soft_missing_idr_bootstrap_reject_reason,
    recovery_timed_fallback_active_from_stats,
};
use crate::transport::rtc::recovery::startup::{resolve_session_phase_from_stats, SessionPhase};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) const DISPLAYED_IDR_FAST_PATH_WINDOW_MS: f64 = 5_000.0;
const DISPLAYED_IDR_FAST_PATH_COOLDOWN_MS: f64 = 4_000.0;
const DISPLAYED_IDR_FAST_PATH_GAP_RTP_MIN: u32 = 60_000;
const DISPLAYED_IDR_FAST_PATH_PATH_D_SUBMIT_AGE_MS: f64 = 2_500.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplayedIdrFastPathKind {
    PathA,
    PathB,
    PathC,
    /// recovering + waiting-keyframe 供给断裂：只请求 PLI，不触发 decoder reset。
    PathD,
}

impl DisplayedIdrFastPathKind {
    pub(crate) fn reason_label(self) -> &'static str {
        match self {
            Self::PathA => "displayedIdrFastPathPathA",
            Self::PathB => "displayedIdrFastPathPathB",
            Self::PathC => "displayedIdrFastPathPathC",
            Self::PathD => "displayedIdrFastPathPathD",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DisplayedIdrFastPathBucket {
    pub(crate) host_accept: u32,
    pub(crate) boot_rej_false: u32,
    pub(crate) dec_boot: u32,
    pub(crate) wkf: u32,
    pub(crate) nack_max: u32,
    pub(crate) gap_val: u32,
    pub(crate) submit_max: f64,
}

#[derive(Clone, Debug, Default)]
struct FastPathEdgeState {
    last_host_present_epoch: u64,
    last_h264_inspection_at_ms: Option<f64>,
    last_decoder_boot_at_ms: Option<f64>,
    last_nack_at_ms: Option<f64>,
    last_decoder_waiting: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DisplayedIdrFastPathWindow {
    host_accept_events: VecDeque<f64>,
    boot_rej_false_events: VecDeque<f64>,
    dec_boot_events: VecDeque<f64>,
    wkf_events: VecDeque<f64>,
    nack_events: VecDeque<(f64, u32)>,
    gap_max_in_window: u32,
    edges: FastPathEdgeState,
    path_c_prior_bucket_host_zero: bool,
    last_trigger_at_ms: Option<f64>,
}

impl DisplayedIdrFastPathWindow {
    pub(crate) fn sync_from_stats(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        self.prune(now_ms);
        self.sync_host_present(stats, now_ms);
        self.sync_h264_inspection(stats, now_ms);
        self.sync_decoder_bootstrap(stats, now_ms);
        self.sync_decoder_waiting(stats, now_ms);
        self.sync_nack(stats, now_ms);
        self.sync_gap(stats);
    }

    pub(crate) fn bucket(&self, stats: &XbxEngineMediaRuntimeStats) -> DisplayedIdrFastPathBucket {
        DisplayedIdrFastPathBucket {
            host_accept: self.host_accept_events.len() as u32,
            boot_rej_false: self.boot_rej_false_events.len() as u32,
            dec_boot: self.dec_boot_events.len() as u32,
            wkf: self.wkf_events.len() as u32,
            nack_max: self.nack_max(),
            gap_val: self.gap_max_in_window,
            submit_max: stats.submit_age_ms.unwrap_or(0.0),
        }
    }

    pub(crate) fn evaluate(
        &mut self,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        stream_started_at: std::time::Instant,
        startup_grace: std::time::Duration,
    ) -> Option<DisplayedIdrFastPathKind> {
        if !displayed_idr_serving_from_stats(stats) {
            self.path_c_prior_bucket_host_zero = false;
            return None;
        }
        if self
            .last_trigger_at_ms
            .is_some_and(|at| (now_ms - at).max(0.0) < DISPLAYED_IDR_FAST_PATH_COOLDOWN_MS)
        {
            return None;
        }

        let bucket = self.bucket(stats);
        if path_d(stats, &bucket, now_ms) {
            self.last_trigger_at_ms = Some(now_ms);
            return Some(DisplayedIdrFastPathKind::PathD);
        }

        if !displayed_idr_serving_relaxation_blocked_from_stats(stats, now_ms) {
            self.path_c_prior_bucket_host_zero = false;
            return None;
        }
        if resolve_session_phase_from_stats(Some(stats), stream_started_at, startup_grace)
            != SessionPhase::Steady
        {
            return None;
        }

        let path_c_prior = self.path_c_prior_bucket_host_zero;
        self.path_c_prior_bucket_host_zero = bucket.host_accept == 0;
        let kind = resolve_displayed_idr_fast_path_kind_with_path_c_prior(&bucket, path_c_prior)?;
        self.last_trigger_at_ms = Some(now_ms);
        Some(kind)
    }

    fn prune(&mut self, now_ms: f64) {
        let floor = now_ms - DISPLAYED_IDR_FAST_PATH_WINDOW_MS;
        prune_deque(&mut self.host_accept_events, floor);
        prune_deque(&mut self.boot_rej_false_events, floor);
        prune_deque(&mut self.dec_boot_events, floor);
        prune_deque(&mut self.wkf_events, floor);
        prune_nack_events(&mut self.nack_events, floor);
        self.gap_max_in_window = 0;
    }

    fn nack_max(&self) -> u32 {
        self.nack_events
            .iter()
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(0)
    }

    fn sync_host_present(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        let epoch = stats.host_frame_present_epoch;
        if epoch > self.edges.last_host_present_epoch {
            let delta = epoch.saturating_sub(self.edges.last_host_present_epoch);
            for _ in 0..delta.min(32) {
                self.host_accept_events.push_back(now_ms);
            }
            self.edges.last_host_present_epoch = epoch;
        } else if epoch < self.edges.last_host_present_epoch {
            self.edges.last_host_present_epoch = epoch;
        }
    }

    fn sync_h264_inspection(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() else {
            return;
        };
        if inspection.observed_at_ms <= self.edges.last_h264_inspection_at_ms.unwrap_or(0.0) {
            return;
        }
        self.edges.last_h264_inspection_at_ms = Some(inspection.observed_at_ms);
        if !inspection.admission_accepted
            && inspection
                .bootstrap_reject_reason
                .as_deref()
                .is_some_and(|reason| {
                    is_soft_missing_idr_bootstrap_reject_reason(Some(reason))
                        || matches!(
                            reason,
                            "bootstrapMissingSps"
                                | "bootstrapMissingPps"
                                | "bootstrapInvalidSliceHeader"
                        )
                })
        {
            self.boot_rej_false_events.push_back(now_ms);
        }
    }

    fn sync_decoder_bootstrap(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        let Some(observation) = stats
            .latest_video_decoder_bootstrap_gate_observation
            .as_ref()
        else {
            return;
        };
        if observation.observed_at_ms <= self.edges.last_decoder_boot_at_ms.unwrap_or(0.0) {
            return;
        }
        self.edges.last_decoder_boot_at_ms = Some(observation.observed_at_ms);
        if !observation.bootstrap_ready {
            self.dec_boot_events.push_back(now_ms);
        }
    }

    fn sync_decoder_waiting(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        let waiting = stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
        if waiting && !self.edges.last_decoder_waiting {
            self.wkf_events.push_back(now_ms);
        }
        self.edges.last_decoder_waiting = waiting;
    }

    fn sync_nack(&mut self, stats: &XbxEngineMediaRuntimeStats, now_ms: f64) {
        let Some(observation) = stats.latest_video_nack_observation.as_ref() else {
            return;
        };
        if observation.action != "sent" {
            return;
        }
        if observation.observed_at_ms <= self.edges.last_nack_at_ms.unwrap_or(0.0) {
            return;
        }
        self.edges.last_nack_at_ms = Some(observation.observed_at_ms);
        let count = u32::from(observation.packet_count);
        self.nack_events.push_back((now_ms, count));
    }

    fn sync_gap(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        self.gap_max_in_window = self
            .gap_max_in_window
            .max(estimate_gap_rtp_delta_from_stats(stats));
    }
}

pub(crate) fn resolve_displayed_idr_fast_path_kind(
    bucket: &DisplayedIdrFastPathBucket,
) -> Option<DisplayedIdrFastPathKind> {
    if hard_exclude_fast_path(bucket) {
        return None;
    }
    if path_a(bucket) {
        return Some(DisplayedIdrFastPathKind::PathA);
    }
    if path_b(bucket) {
        return Some(DisplayedIdrFastPathKind::PathB);
    }
    None
}

pub(crate) fn resolve_displayed_idr_fast_path_kind_with_path_c_prior(
    bucket: &DisplayedIdrFastPathBucket,
    path_c_prior_bucket_host_zero: bool,
) -> Option<DisplayedIdrFastPathKind> {
    if hard_exclude_fast_path(bucket) {
        return None;
    }
    if path_a(bucket) {
        return Some(DisplayedIdrFastPathKind::PathA);
    }
    if path_b(bucket) {
        return Some(DisplayedIdrFastPathKind::PathB);
    }
    if path_c(bucket) && path_c_prior_bucket_host_zero {
        return Some(DisplayedIdrFastPathKind::PathC);
    }
    None
}

fn hard_exclude_fast_path(bucket: &DisplayedIdrFastPathBucket) -> bool {
    if bucket.host_accept >= 2
        && bucket.boot_rej_false <= 1
        && bucket.dec_boot <= 1
        && bucket.submit_max < 400.0
    {
        return true;
    }
    bucket.nack_max >= 80 && bucket.boot_rej_false == 0 && bucket.host_accept >= 2
}

fn path_a(bucket: &DisplayedIdrFastPathBucket) -> bool {
    bucket.gap_val >= DISPLAYED_IDR_FAST_PATH_GAP_RTP_MIN
        && bucket.host_accept == 0
        && bucket.submit_max >= 800.0
}

fn path_b(bucket: &DisplayedIdrFastPathBucket) -> bool {
    bucket.host_accept == 0
        && bucket.boot_rej_false >= 4
        && bucket.dec_boot >= 6
        && bucket.wkf >= 5
        && (bucket.nack_max >= 40 || bucket.submit_max >= 1_000.0)
}

fn path_c(bucket: &DisplayedIdrFastPathBucket) -> bool {
    bucket.host_accept == 0
        && bucket.dec_boot >= 8
        && bucket.boot_rej_false <= 2
        && bucket.submit_max >= 3_000.0
}

fn path_d(
    stats: &XbxEngineMediaRuntimeStats,
    bucket: &DisplayedIdrFastPathBucket,
    now_ms: f64,
) -> bool {
    if !displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return false;
    }
    if recovery_timed_fallback_active_from_stats(stats, now_ms) {
        return true;
    }
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return false;
    }
    if bucket.submit_max < DISPLAYED_IDR_FAST_PATH_PATH_D_SUBMIT_AGE_MS {
        return false;
    }
    if bucket.wkf == 0 {
        return false;
    }
    matches!(
        stats.session_phase.as_deref(),
        Some(
            "recovering"
                | "observing"
                | "local-self-healing"
                | "recovery-eligible"
                | "active-recovery"
                | "recovery-blocked"
                | "steady"
        )
    )
}

fn estimate_gap_rtp_delta_from_stats(stats: &XbxEngineMediaRuntimeStats) -> u32 {
    let mut max_gap = 0u32;
    if let (Some(arrival), Some(decoded)) = (
        stats.latest_video_packet_arrival_rtp_timestamp,
        stats.latest_video_decode_ok_rtp_timestamp,
    ) {
        max_gap = max_gap.max(arrival.wrapping_sub(decoded));
    }
    if let Some(timeline) = stats.latest_video_timeline_observation.as_ref() {
        if let (Some(frame_rtp), Some(decoded)) = (
            timeline
                .gap
                .as_ref()
                .and_then(|gap| gap.frame_rtp_timestamp),
            stats.latest_video_decode_ok_rtp_timestamp,
        ) {
            max_gap = max_gap.max(frame_rtp.wrapping_sub(decoded));
        }
    }
    max_gap
}

fn prune_deque(queue: &mut VecDeque<f64>, floor_ms: f64) {
    while queue.front().is_some_and(|timestamp| *timestamp < floor_ms) {
        queue.pop_front();
    }
}

fn prune_nack_events(queue: &mut VecDeque<(f64, u32)>, floor_ms: f64) {
    while queue
        .front()
        .is_some_and(|(timestamp, _)| *timestamp < floor_ms)
    {
        queue.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(
        host_accept: u32,
        boot_rej_false: u32,
        dec_boot: u32,
        wkf: u32,
        nack_max: u32,
        gap_val: u32,
        submit_max: f64,
    ) -> DisplayedIdrFastPathBucket {
        DisplayedIdrFastPathBucket {
            host_accept,
            boot_rej_false,
            dec_boot,
            wkf,
            nack_max,
            gap_val,
            submit_max,
        }
    }

    #[test]
    fn path_b_hits_freeze_onset_bucket() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(0, 5, 8, 6, 110, 0, 4_238.0)),
            Some(DisplayedIdrFastPathKind::PathB)
        );
    }

    #[test]
    fn path_b_hits_second_freeze_with_lower_nack() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(0, 5, 6, 8, 46, 0, 2_000.0)),
            Some(DisplayedIdrFastPathKind::PathB)
        );
    }

    #[test]
    fn spike_and_precursor_do_not_hit() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(2, 1, 0, 0, 44, 0, 305.0)),
            None
        );
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(2, 0, 1, 0, 97, 0, 50.0)),
            None
        );
    }

    #[test]
    fn path_a_requires_host_stop_and_submit() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(0, 4, 8, 6, 50, 65_529, 14_000.0)),
            Some(DisplayedIdrFastPathKind::PathA)
        );
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(2, 1, 0, 0, 46, 65_529, 50.0)),
            None
        );
    }

    #[test]
    fn path_c_hits_decode_only_stall() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind_with_path_c_prior(
                &bucket(0, 0, 9, 0, 200, 0, 5_000.0),
                true
            ),
            Some(DisplayedIdrFastPathKind::PathC)
        );
    }

    #[test]
    fn healthy_bucket_excluded() {
        assert_eq!(
            resolve_displayed_idr_fast_path_kind(&bucket(2, 0, 0, 0, 6, 0, 105.0)),
            None
        );
    }
}
