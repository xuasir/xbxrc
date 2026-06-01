//! Receive-local picture recovery ledger：WebRTC-like `keyframe_required` 闩锁与 response/terminal 事实。

use crate::transport::rtc::recovery::contract::displayed_idr_serving_allows_relaxed_controls_from_stats;
use crate::transport::rtc::recovery::contract::{ReferenceChainObservation, ReferenceChainState};
use crate::XbxEngineMediaRuntimeStats;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeyframeRequiredCause {
    #[default]
    None,
    FirstDelta,
    NackExhausted,
    H264BootstrapMissing,
    DecoderWaitingKeyframe,
    DecoderInvalidData,
    DecoderNoOutputStreak,
    KeyframeSentNonIdrOnly,
}

impl KeyframeRequiredCause {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FirstDelta => "first-delta",
            Self::NackExhausted => "nack-exhausted",
            Self::H264BootstrapMissing => "h264-bootstrap-missing",
            Self::DecoderWaitingKeyframe => "decoder-waiting-keyframe",
            Self::DecoderInvalidData => "decoder-invalid-data",
            Self::DecoderNoOutputStreak => "decoder-no-output-streak",
            Self::KeyframeSentNonIdrOnly => "keyframe-sent-non-idr-only",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeyframeResponseState {
    #[default]
    NoPacket,
    NonIdrOnly,
    IdrUnusable,
    UsableIdr,
}

impl KeyframeResponseState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoPacket => "no-packet",
            Self::NonIdrOnly => "non-idr-only",
            Self::IdrUnusable => "idr-unusable",
            Self::UsableIdr => "usable-idr",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RecoveryNackState {
    #[default]
    None,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RecoveryDecoderResult {
    #[default]
    Unknown,
    Ok,
    InvalidData,
    WaitingKeyframe,
    NoOutputStreak,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CleanAnchorLedgerState {
    #[default]
    None,
    Committed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DisplayLedgerState {
    #[default]
    None,
    DisplayStable,
}

impl DisplayLedgerState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::DisplayStable => "display-stable",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PictureRecoveryTerminalReason {
    #[default]
    None,
    RemoteNoResponse,
    RemoteContinuationOnly,
    RemoteIdrUnusable,
    DecoderRejectedIdr,
    NoCleanAnchorAfterDecode,
}

impl PictureRecoveryTerminalReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RemoteNoResponse => "remote-no-response",
            Self::RemoteContinuationOnly => "remote-continuation-only",
            Self::RemoteIdrUnusable => "remote-idr-unusable",
            Self::DecoderRejectedIdr => "decoder-rejected-idr",
            Self::NoCleanAnchorAfterDecode => "no-clean-anchor-after-decode",
        }
    }
}

/// Receive 层 picture recovery 主事实源。
#[derive(Clone, Debug, Default)]
pub(crate) struct ReceiveRecoveryLedger {
    pub(crate) keyframe_required: bool,
    pub(crate) keyframe_required_cause: KeyframeRequiredCause,
    pub(crate) response_state: KeyframeResponseState,
    pub(crate) nack_state: RecoveryNackState,
    pub(crate) decoder_result: RecoveryDecoderResult,
    pub(crate) clean_anchor_state: CleanAnchorLedgerState,
    pub(crate) terminal_reason: PictureRecoveryTerminalReason,
    pub(crate) terminal_candidate: bool,
    pub(crate) ledger_generation: u64,
    pub(crate) last_keyframe_request_sent_at_ms: Option<f64>,
    pub(crate) unresolved_keyframe_request_count: u32,
    pub(crate) last_usable_keyframe_rtp: Option<u32>,
    pub(crate) last_clean_anchor_rtp: Option<u32>,
    pub(crate) last_usable_idr_accepted_at_ms: Option<f64>,
    pub(crate) last_decoder_reference_synced_at_ms: Option<f64>,
    pub(crate) has_established_usable_anchor: bool,
    pub(crate) non_idr_continuation_after_sent: u32,
    pub(crate) display_state: DisplayLedgerState,
    pub(crate) last_display_stable_rtp: Option<u32>,
    pub(crate) last_display_stable_at_ms: Option<f64>,
    /// 与 `transport_recovery_epoch` 绑定；新轮次必须 reset，避免上轮 display/response 污染。
    pub(crate) bound_transport_recovery_epoch: u64,
}

impl ReceiveRecoveryLedger {
    pub(crate) fn bump_ledger_generation_only(&mut self) {
        self.ledger_generation = self.ledger_generation.saturating_add(1);
    }

    /// terminal 判定与 `keyframe_required` 闩锁解耦：恢复轮次未闭合即可评估。
    pub(crate) fn picture_recovery_terminal_eligible(&self) -> bool {
        self.keyframe_required
            || self.unresolved_keyframe_request_count > 0
            || self.last_keyframe_request_sent_at_ms.is_some()
            || matches!(
                self.response_state,
                KeyframeResponseState::UsableIdr
                    | KeyframeResponseState::NonIdrOnly
                    | KeyframeResponseState::IdrUnusable
            )
            || self.terminal_candidate
    }

    pub(crate) fn set_keyframe_required(&mut self, cause: KeyframeRequiredCause) {
        let was_required = self.keyframe_required;
        self.keyframe_required = true;
        self.keyframe_required_cause = cause;
        if !was_required {
            self.bump_ledger_generation_only();
        }
    }

    pub(crate) fn clear_keyframe_required(&mut self) {
        self.keyframe_required = false;
        self.keyframe_required_cause = KeyframeRequiredCause::None;
        self.unresolved_keyframe_request_count = 0;
        self.non_idr_continuation_after_sent = 0;
        self.terminal_candidate = false;
        if self.terminal_reason != PictureRecoveryTerminalReason::None {
            self.terminal_reason = PictureRecoveryTerminalReason::None;
        }
    }

    pub(crate) fn note_keyframe_request_sent(&mut self, now_ms: f64) {
        self.bump_ledger_generation_only();
        self.last_keyframe_request_sent_at_ms = Some(now_ms);
        self.unresolved_keyframe_request_count =
            self.unresolved_keyframe_request_count.saturating_add(1);
        if self.response_state == KeyframeResponseState::UsableIdr {
            self.response_state = KeyframeResponseState::NoPacket;
        }
    }

    pub(crate) fn note_non_idr_continuation(&mut self) {
        if self.unresolved_keyframe_request_count > 0 || self.keyframe_required {
            self.response_state = KeyframeResponseState::NonIdrOnly;
            self.non_idr_continuation_after_sent =
                self.non_idr_continuation_after_sent.saturating_add(1);
            if self.non_idr_continuation_after_sent >= 3 {
                self.set_keyframe_required(KeyframeRequiredCause::KeyframeSentNonIdrOnly);
            }
        }
    }

    pub(crate) fn note_idr_unusable(&mut self) {
        self.response_state = KeyframeResponseState::IdrUnusable;
        self.set_keyframe_required(KeyframeRequiredCause::H264BootstrapMissing);
    }

    pub(crate) fn note_display_stable(&mut self, rtp_timestamp: Option<u32>, now_ms: f64) {
        self.display_state = DisplayLedgerState::DisplayStable;
        self.last_display_stable_at_ms = Some(now_ms);
        if let Some(rtp) = rtp_timestamp {
            self.last_display_stable_rtp = Some(rtp);
        }
    }

    /// Insert 准入接受 usable IDR：仅记 response 事实，不清 `keyframe_required`、不标 decoder synced。
    pub(crate) fn note_usable_idr_packet_accepted(&mut self, rtp_timestamp: u32, now_ms: f64) {
        self.response_state = KeyframeResponseState::UsableIdr;
        self.last_usable_keyframe_rtp = Some(rtp_timestamp);
        self.last_usable_idr_accepted_at_ms = Some(now_ms);
        self.has_established_usable_anchor = true;
        self.unresolved_keyframe_request_count = 0;
        self.terminal_candidate = false;
        if self.terminal_reason != PictureRecoveryTerminalReason::None {
            self.terminal_reason = PictureRecoveryTerminalReason::None;
        }
    }

    /// usable IDR 已到达但 decode/anchor 未闭合时的 terminal reason。
    pub(crate) fn terminal_reason_after_usable_idr(
        &self,
        now_ms: f64,
        clean_anchor_patience_ms: f64,
    ) -> Option<PictureRecoveryTerminalReason> {
        match self.decoder_result {
            RecoveryDecoderResult::InvalidData => {
                return Some(PictureRecoveryTerminalReason::DecoderRejectedIdr);
            }
            RecoveryDecoderResult::WaitingKeyframe | RecoveryDecoderResult::NoOutputStreak => {
                return Some(PictureRecoveryTerminalReason::DecoderRejectedIdr);
            }
            RecoveryDecoderResult::Ok => {
                if matches!(self.clean_anchor_state, CleanAnchorLedgerState::Committed) {
                    return None;
                }
                if self.last_decoder_reference_synced_at_ms.is_some()
                    && self
                        .last_usable_idr_accepted_at_ms
                        .is_some_and(|accepted_at| {
                            now_ms - accepted_at >= clean_anchor_patience_ms.max(1.0)
                        })
                {
                    return Some(PictureRecoveryTerminalReason::NoCleanAnchorAfterDecode);
                }
            }
            RecoveryDecoderResult::Unknown => {}
        }
        None
    }

    /// `sent / RTT / response_state` 共同决定的远端 usable IDR 终端诊断。
    pub(crate) fn terminal_reason_for_remote_no_usable_idr(
        &self,
        now_ms: f64,
        effective_rtt_ms: f64,
        clean_anchor_patience_ms: f64,
    ) -> Option<PictureRecoveryTerminalReason> {
        let first_sent_at_ms = self.last_keyframe_request_sent_at_ms?;
        let sent_count = self.unresolved_keyframe_request_count;
        let elapsed_ms = (now_ms - first_sent_at_ms).max(0.0);
        let elapsed_ok = elapsed_ms >= 3.0 * effective_rtt_ms.max(1.0);
        let sent_exhausted = sent_count >= 5;
        match self.response_state {
            KeyframeResponseState::NoPacket if sent_exhausted || elapsed_ok => {
                Some(PictureRecoveryTerminalReason::RemoteNoResponse)
            }
            KeyframeResponseState::NonIdrOnly if sent_exhausted || elapsed_ok => {
                Some(PictureRecoveryTerminalReason::RemoteContinuationOnly)
            }
            KeyframeResponseState::IdrUnusable => {
                Some(PictureRecoveryTerminalReason::RemoteIdrUnusable)
            }
            KeyframeResponseState::UsableIdr => {
                self.terminal_reason_after_usable_idr(now_ms, clean_anchor_patience_ms)
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn note_usable_idr(&mut self, rtp_timestamp: u32, now_ms: f64) {
        self.note_usable_idr_packet_accepted(rtp_timestamp, now_ms);
    }

    pub(crate) fn note_decoder_reference_synced(&mut self, now_ms: f64) {
        self.decoder_result = RecoveryDecoderResult::Ok;
        self.last_decoder_reference_synced_at_ms = Some(now_ms);
        if matches!(self.clean_anchor_state, CleanAnchorLedgerState::Committed) {
            self.clear_keyframe_required();
        }
    }

    pub(crate) fn note_clean_anchor_committed(&mut self, rtp_timestamp: Option<u32>) {
        self.clean_anchor_state = CleanAnchorLedgerState::Committed;
        if let Some(rtp) = rtp_timestamp {
            self.last_clean_anchor_rtp = Some(rtp);
        }
        self.clear_keyframe_required();
    }

    pub(crate) fn note_decoder_waiting_keyframe(&mut self) {
        self.decoder_result = RecoveryDecoderResult::WaitingKeyframe;
        self.set_keyframe_required(KeyframeRequiredCause::DecoderWaitingKeyframe);
    }

    pub(crate) fn note_decoder_invalid_data(&mut self) {
        self.decoder_result = RecoveryDecoderResult::InvalidData;
        self.set_keyframe_required(KeyframeRequiredCause::DecoderInvalidData);
        self.terminal_candidate = true;
    }

    pub(crate) fn note_decoder_no_output_streak(&mut self) {
        self.decoder_result = RecoveryDecoderResult::NoOutputStreak;
        self.set_keyframe_required(KeyframeRequiredCause::DecoderNoOutputStreak);
    }

    pub(crate) fn note_nack_exhausted(&mut self) {
        self.nack_state = RecoveryNackState::Exhausted;
        self.set_keyframe_required(KeyframeRequiredCause::NackExhausted);
    }

    pub(crate) fn note_first_delta(&mut self) {
        if !self.has_established_usable_anchor {
            self.set_keyframe_required(KeyframeRequiredCause::FirstDelta);
        }
    }

    /// 新 `transport_recovery_epoch` 开局：清空上轮 picture/display/response 事实。
    pub(crate) fn reset_for_transport_recovery_epoch(&mut self, transport_recovery_epoch: u64) {
        if self.bound_transport_recovery_epoch == transport_recovery_epoch {
            return;
        }
        self.bound_transport_recovery_epoch = transport_recovery_epoch;
        self.reset_picture_recovery_round_state();
    }

    fn reset_picture_recovery_round_state(&mut self) {
        self.bump_ledger_generation_only();
        self.clear_keyframe_required();
        self.response_state = KeyframeResponseState::NoPacket;
        self.nack_state = RecoveryNackState::None;
        self.decoder_result = RecoveryDecoderResult::Unknown;
        self.clean_anchor_state = CleanAnchorLedgerState::None;
        self.display_state = DisplayLedgerState::None;
        self.last_display_stable_rtp = None;
        self.last_display_stable_at_ms = None;
        self.has_established_usable_anchor = false;
        self.last_usable_keyframe_rtp = None;
        self.last_clean_anchor_rtp = None;
        self.last_usable_idr_accepted_at_ms = None;
        self.last_decoder_reference_synced_at_ms = None;
        self.last_keyframe_request_sent_at_ms = None;
        self.non_idr_continuation_after_sent = 0;
        self.terminal_reason = PictureRecoveryTerminalReason::None;
        self.terminal_candidate = false;
    }

    pub(crate) fn emit_terminal(&mut self, reason: PictureRecoveryTerminalReason) {
        self.terminal_reason = reason;
        self.terminal_candidate = false;
        self.bump_ledger_generation_only();
    }

    pub(crate) fn sparse_active(&self) -> bool {
        self.keyframe_required
            || matches!(
                self.response_state,
                KeyframeResponseState::NonIdrOnly | KeyframeResponseState::IdrUnusable
            )
    }

    /// 从 ledger 投影 ReferenceChainState；返回 None 表示应 fallback stats。
    pub(crate) fn project_reference_chain(
        &self,
        has_unresolved_hard_gap: bool,
        nack_exhausted: bool,
        stats_observation: &ReferenceChainObservation,
    ) -> Option<ReferenceChainObservation> {
        let base = || ReferenceChainObservation {
            decoder_reference_synced: stats_observation.decoder_reference_synced
                || self.last_decoder_reference_synced_at_ms.is_some(),
            bootstrap_ready: stats_observation.bootstrap_ready,
            has_active_gap: stats_observation.has_active_gap || has_unresolved_hard_gap,
            nack_exhausted: stats_observation.nack_exhausted || nack_exhausted,
            submit_age_ms: stats_observation.submit_age_ms,
            ..Default::default()
        };

        if matches!(self.clean_anchor_state, CleanAnchorLedgerState::Committed)
            || (self.last_decoder_reference_synced_at_ms.is_some() && !self.keyframe_required)
        {
            return Some(ReferenceChainObservation {
                state: ReferenceChainState::Continuous,
                cause: if matches!(self.clean_anchor_state, CleanAnchorLedgerState::Committed) {
                    "ledger-clean-anchor-committed"
                } else {
                    "ledger-decoder-reference-synced"
                },
                ..base()
            });
        }

        if self.keyframe_required {
            return Some(ReferenceChainObservation {
                state: ReferenceChainState::NeedKeyframe,
                cause: match self.keyframe_required_cause {
                    KeyframeRequiredCause::KeyframeSentNonIdrOnly => "keyframe-sent-non-idr-only",
                    KeyframeRequiredCause::DecoderNoOutputStreak => "decoder-no-output-streak",
                    KeyframeRequiredCause::NackExhausted => "nack-exhausted",
                    KeyframeRequiredCause::DecoderWaitingKeyframe => "decoder-waiting-keyframe",
                    KeyframeRequiredCause::DecoderInvalidData => "decoder-invalid-data",
                    KeyframeRequiredCause::FirstDelta => "first-delta",
                    other => other.as_str(),
                },
                ..base()
            });
        }

        if has_unresolved_hard_gap {
            return Some(ReferenceChainObservation {
                state: if nack_exhausted {
                    ReferenceChainState::NeedKeyframe
                } else {
                    ReferenceChainState::Repairing
                },
                cause: if nack_exhausted {
                    "receive-ledger-hard-gap-nack-exhausted"
                } else {
                    "receive-ledger-hard-gap-repairing"
                },
                ..base()
            });
        }

        if !self.has_established_usable_anchor {
            return Some(ReferenceChainObservation {
                state: ReferenceChainState::Unknown,
                cause: "ledger-bootstrap-missing-priming",
                ..base()
            });
        }

        if self.last_decoder_reference_synced_at_ms.is_none() {
            return Some(ReferenceChainObservation {
                state: ReferenceChainState::NeedKeyframe,
                cause: match self.response_state {
                    KeyframeResponseState::UsableIdr => "ledger-usable-idr-awaiting-decode-sync",
                    KeyframeResponseState::NonIdrOnly | KeyframeResponseState::IdrUnusable => {
                        "ledger-awaiting-usable-idr"
                    }
                    KeyframeResponseState::NoPacket => "ledger-awaiting-first-usable-idr",
                },
                ..base()
            });
        }

        Some(ReferenceChainObservation {
            state: ReferenceChainState::Continuous,
            cause: "ledger-reference-established",
            ..base()
        })
    }

    /// packet-local action stage：供 Insert/Decode 与 feedback arbiter 共用。
    pub(crate) fn derive_packet_recovery_action_stage(
        &self,
        has_active_gap: bool,
        nack_in_flight: bool,
        gap_age_ms: Option<f64>,
        effective_rtt_ms: f64,
    ) -> crate::transport::rtc::recovery::contract::PacketRecoveryActionStage {
        use crate::transport::rtc::recovery::contract::PacketRecoveryActionStage;
        use crate::transport::rtc::recovery::contract::GAP_KEYFRAME_ONLY_MAX_AGE_MS;

        if self.keyframe_required {
            return PacketRecoveryActionStage::RequestIdr;
        }
        if matches!(
            self.decoder_result,
            RecoveryDecoderResult::WaitingKeyframe
                | RecoveryDecoderResult::NoOutputStreak
                | RecoveryDecoderResult::InvalidData
        ) {
            return PacketRecoveryActionStage::RequestIdr;
        }
        if gap_age_ms
            .is_some_and(|age| age >= GAP_KEYFRAME_ONLY_MAX_AGE_MS.max(effective_rtt_ms * 2.0))
        {
            return PacketRecoveryActionStage::WaitKeyframe;
        }
        if has_active_gap {
            if nack_in_flight {
                return PacketRecoveryActionStage::NackPending;
            }
            if matches!(self.nack_state, RecoveryNackState::Exhausted) {
                return PacketRecoveryActionStage::WaitKeyframe;
            }
            return PacketRecoveryActionStage::NackMissed;
        }
        PacketRecoveryActionStage::Steady
    }

    /// 从 runtime stats 回写 decoder 结果，设置或清除 `keyframe_required`。
    pub(crate) fn apply_decoder_facts_from_stats(
        &mut self,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) {
        use crate::transport::rtc::recovery::contract::{
            decoder_reference_synced_from_stats, CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK,
        };

        let no_output_streak = stats
            .latest_decode_output_path_observation
            .as_ref()
            .and_then(|obs| obs.backend_no_output_streak)
            .unwrap_or(0);
        let displayed_idr_relaxed =
            displayed_idr_serving_allows_relaxed_controls_from_stats(stats, now_ms);
        if no_output_streak >= CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK {
            if displayed_idr_relaxed {
                return;
            }
            self.note_decoder_no_output_streak();
            return;
        }
        if stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe") {
            if displayed_idr_relaxed {
                return;
            }
            self.note_decoder_waiting_keyframe();
            return;
        }
        if stats
            .latest_decode_output_path_observation
            .as_ref()
            .is_some_and(|obs| obs.verdict == "ffmpegInvalidData")
        {
            self.note_decoder_invalid_data();
            return;
        }
        if decoder_reference_synced_from_stats(stats, now_ms) {
            self.note_decoder_reference_synced(now_ms);
        }
    }

    pub(crate) fn sync_to_stats(&self, stats: &mut XbxEngineMediaRuntimeStats) {
        stats.receive_keyframe_required = Some(self.keyframe_required);
        stats.receive_keyframe_required_cause =
            Some(self.keyframe_required_cause.as_str().to_string());
        stats.receive_keyframe_response_state = Some(self.response_state.as_str().to_string());
        stats.receive_recovery_ledger_generation = Some(self.ledger_generation);
        stats.receive_picture_recovery_terminal_candidate = Some(self.terminal_candidate);
        if self.terminal_reason != PictureRecoveryTerminalReason::None {
            stats.latest_receive_picture_recovery_terminal_reason =
                Some(self.terminal_reason.as_str().to_string());
        } else {
            stats.latest_receive_picture_recovery_terminal_reason = None;
        }
        stats.receive_keyframe_sent_count_unresolved = self.unresolved_keyframe_request_count;
        stats.receive_keyframe_last_sent_at_ms = self.last_keyframe_request_sent_at_ms;
        stats.receive_display_state = Some(self.display_state.as_str().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::recovery::contract::ReferenceChainState;
    use crate::transport::rtc::recovery::contract::{
        displayed_idr_serving_allows_relaxed_controls_from_stats, displayed_idr_serving_from_stats,
    };

    fn empty_stats_obs() -> ReferenceChainObservation {
        ReferenceChainObservation::default()
    }

    #[test]
    fn first_delta_sets_keyframe_required() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_first_delta();
        assert!(ledger.keyframe_required);
        assert_eq!(
            ledger.keyframe_required_cause,
            KeyframeRequiredCause::FirstDelta
        );
    }

    #[test]
    fn nack_exhausted_sets_keyframe_required() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_nack_exhausted();
        assert!(ledger.keyframe_required);
        assert_eq!(
            ledger.keyframe_required_cause,
            KeyframeRequiredCause::NackExhausted
        );
    }

    #[test]
    fn decoder_invalid_sets_keyframe_required_and_terminal_candidate() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_decoder_invalid_data();
        assert!(ledger.keyframe_required);
        assert!(ledger.terminal_candidate);
    }

    #[test]
    fn usable_idr_packet_keeps_keyframe_required_until_decoder_synced() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.set_keyframe_required(KeyframeRequiredCause::FirstDelta);
        ledger.note_keyframe_request_sent(1_000.0);
        ledger.note_usable_idr_packet_accepted(90_001, 2_000.0);
        assert!(ledger.keyframe_required);
        assert_eq!(ledger.response_state, KeyframeResponseState::UsableIdr);
        assert_eq!(ledger.unresolved_keyframe_request_count, 0);
        assert!(ledger.last_decoder_reference_synced_at_ms.is_none());
        let obs = ledger
            .project_reference_chain(false, false, &empty_stats_obs())
            .expect("projection");
        assert_eq!(obs.state, ReferenceChainState::NeedKeyframe);

        ledger.note_decoder_reference_synced(1_500.0);
        assert!(
            ledger.keyframe_required,
            "decode synced 后 clean anchor 未提交应保留恢复闩锁"
        );
        assert!(ledger.picture_recovery_terminal_eligible());
        ledger.note_clean_anchor_committed(Some(90_001));
        assert!(!ledger.keyframe_required);
        let obs = ledger
            .project_reference_chain(false, false, &empty_stats_obs())
            .expect("projection");
        assert_eq!(obs.state, ReferenceChainState::Continuous);
    }

    #[test]
    fn consecutive_keyframe_sends_bump_ledger_generation() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.set_keyframe_required(KeyframeRequiredCause::FirstDelta);
        assert_eq!(ledger.ledger_generation, 1);
        ledger.note_keyframe_request_sent(1_000.0);
        assert_eq!(ledger.ledger_generation, 2);
        ledger.note_keyframe_request_sent(2_000.0);
        assert_eq!(ledger.ledger_generation, 3);
    }

    #[test]
    fn decoder_synced_without_clean_anchor_stays_terminal_eligible() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_usable_idr_packet_accepted(90_001, 1_000.0);
        ledger.note_decoder_reference_synced(1_500.0);
        assert!(ledger.picture_recovery_terminal_eligible());
        assert_eq!(
            ledger.terminal_reason_after_usable_idr(2_500.0, 500.0),
            Some(PictureRecoveryTerminalReason::NoCleanAnchorAfterDecode)
        );
    }

    #[test]
    fn sync_to_stats_clears_terminal_reason_after_clean_anchor() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.emit_terminal(PictureRecoveryTerminalReason::RemoteNoResponse);
        let mut stats = crate::XbxEngineMediaRuntimeStats::default();
        ledger.sync_to_stats(&mut stats);
        assert_eq!(
            stats
                .latest_receive_picture_recovery_terminal_reason
                .as_deref(),
            Some("remote-no-response")
        );

        ledger.note_clean_anchor_committed(Some(90_001));
        ledger.sync_to_stats(&mut stats);
        assert!(stats
            .latest_receive_picture_recovery_terminal_reason
            .is_none());
    }

    #[test]
    fn clean_anchor_committed_projects_continuous() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.set_keyframe_required(KeyframeRequiredCause::DecoderWaitingKeyframe);
        ledger.note_clean_anchor_committed(Some(90_001));
        assert!(!ledger.keyframe_required);
        let obs = ledger
            .project_reference_chain(false, false, &empty_stats_obs())
            .expect("projection");
        assert_eq!(obs.state, ReferenceChainState::Continuous);
    }

    #[test]
    fn non_idr_after_sent_promotes_need_keyframe() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_keyframe_request_sent(1_000.0);
        for _ in 0..3 {
            ledger.note_non_idr_continuation();
        }
        assert!(ledger.keyframe_required);
        assert_eq!(
            ledger.keyframe_required_cause,
            KeyframeRequiredCause::KeyframeSentNonIdrOnly
        );
        let obs = ledger
            .project_reference_chain(false, false, &empty_stats_obs())
            .expect("projection");
        assert_eq!(obs.state, ReferenceChainState::NeedKeyframe);
        assert_eq!(obs.cause, "keyframe-sent-non-idr-only");
    }

    #[test]
    fn usable_idr_with_decoder_invalid_emits_decoder_rejected_terminal() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_usable_idr_packet_accepted(90_001, 1_000.0);
        ledger.note_decoder_invalid_data();
        assert_eq!(
            ledger.terminal_reason_after_usable_idr(2_000.0, 500.0),
            Some(PictureRecoveryTerminalReason::DecoderRejectedIdr)
        );
    }

    #[test]
    fn decoded_without_clean_anchor_emits_no_clean_anchor_terminal() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_usable_idr_packet_accepted(90_001, 1_000.0);
        ledger.note_decoder_reference_synced(1_500.0);
        assert_eq!(
            ledger.terminal_reason_after_usable_idr(2_500.0, 500.0),
            Some(PictureRecoveryTerminalReason::NoCleanAnchorAfterDecode)
        );
    }

    #[test]
    fn displayed_idr_serving_skips_waiting_keyframe_decoder_fact() {
        let mut ledger = ReceiveRecoveryLedger::default();
        let mut stats = crate::XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(1_000.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());

        assert!(displayed_idr_serving_from_stats(&stats));
        assert!(displayed_idr_serving_allows_relaxed_controls_from_stats(
            &stats, 1_500.0
        ));

        ledger.apply_decoder_facts_from_stats(&stats, 1_500.0);

        assert_eq!(ledger.decoder_result, RecoveryDecoderResult::Unknown);
        assert!(!ledger.keyframe_required);
    }

    #[test]
    fn usable_idr_without_decode_sync_projects_need_keyframe_not_continuous() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_usable_idr_packet_accepted(90_001, 1_000.0);
        let obs = ledger
            .project_reference_chain(false, false, &empty_stats_obs())
            .expect("projection");
        assert_eq!(obs.state, ReferenceChainState::NeedKeyframe);
        assert_eq!(obs.cause, "ledger-usable-idr-awaiting-decode-sync");
    }

    #[test]
    fn reset_for_transport_recovery_epoch_clears_display_and_response_state() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_display_stable(Some(90_001), 500.0);
        ledger.note_usable_idr_packet_accepted(90_001, 500.0);
        ledger.note_decoder_reference_synced(600.0);
        ledger.bound_transport_recovery_epoch = 1;

        ledger.reset_for_transport_recovery_epoch(2);

        assert_eq!(ledger.bound_transport_recovery_epoch, 2);
        assert_eq!(ledger.display_state, DisplayLedgerState::None);
        assert_eq!(ledger.response_state, KeyframeResponseState::NoPacket);
        assert!(!ledger.has_established_usable_anchor);
        assert!(ledger.last_decoder_reference_synced_at_ms.is_none());
    }

    #[test]
    fn sync_after_epoch_reset_does_not_restore_display_stable() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_display_stable(Some(90_001), 500.0);
        ledger.bound_transport_recovery_epoch = 1;
        ledger.reset_for_transport_recovery_epoch(2);
        let mut stats = crate::XbxEngineMediaRuntimeStats::default();
        ledger.sync_to_stats(&mut stats);
        assert_ne!(
            stats.receive_display_state.as_deref(),
            Some("display-stable")
        );
        assert_ne!(
            stats.receive_keyframe_response_state.as_deref(),
            Some("usable-idr")
        );
    }

    #[test]
    fn sync_after_epoch_reset_clears_sent_at_projection() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_keyframe_request_sent(1_000.0);
        ledger.bound_transport_recovery_epoch = 1;
        ledger.reset_for_transport_recovery_epoch(2);
        let mut stats = crate::XbxEngineMediaRuntimeStats {
            receive_keyframe_last_sent_at_ms: Some(1_000.0),
            ..Default::default()
        };
        ledger.sync_to_stats(&mut stats);
        assert_eq!(stats.receive_keyframe_last_sent_at_ms, None);
        assert_eq!(stats.receive_keyframe_sent_count_unresolved, 0);
    }

    #[test]
    fn same_epoch_picture_recovery_closure_complete() {
        use crate::transport::rtc::recovery::contract::receive_picture_recovery_complete_at;

        let stats = crate::XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 4,
            transport_recovery_episode_opened_at_ms: Some(800.0),
            receive_keyframe_required: Some(false),
            receive_keyframe_response_state: Some("usable-idr".to_string()),
            video_anchor_clean_epoch: Some(4),
            video_anchor_clean_observed_at_ms: Some(900.0),
            recovery_decoder_reference_synced_at_ms: Some(900.0),
            latest_video_decode_ok_time_ms: Some(900.0),
            latest_video_decode_ok_rtp_timestamp: Some(90_001),
            ..Default::default()
        };
        assert!(receive_picture_recovery_complete_at(4, &stats, 950.0));
    }

    #[test]
    fn stale_epoch_display_stable_does_not_complete_next_round() {
        use crate::transport::rtc::recovery::contract::receive_picture_recovery_complete_at;

        let stats = crate::XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 5,
            receive_keyframe_required: Some(false),
            receive_keyframe_response_state: None,
            receive_display_state: Some("display-stable".to_string()),
            recovery_displayed_idr_at_ms: Some(500.0),
            video_anchor_clean_epoch: Some(4),
            ..Default::default()
        };
        assert!(!receive_picture_recovery_complete_at(5, &stats, 600.0));
    }

    #[test]
    fn terminal_reason_for_remote_no_usable_idr_uses_ledger_window() {
        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_keyframe_request_sent(1_000.0);
        ledger.note_keyframe_request_sent(1_500.0);
        ledger.note_non_idr_continuation();
        ledger.note_non_idr_continuation();
        ledger.note_non_idr_continuation();
        assert_eq!(
            ledger.terminal_reason_for_remote_no_usable_idr(4_200.0, 700.0, 500.0),
            Some(PictureRecoveryTerminalReason::RemoteContinuationOnly)
        );
    }
}
