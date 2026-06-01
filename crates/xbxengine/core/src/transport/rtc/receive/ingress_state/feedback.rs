//! Receive feedback arbiter 挂接：plan / execute / stats / trace 投影。

use crate::transport::rtc::capability::KeyframeSendOutcome;
use crate::transport::rtc::receive::feedback_arbiter::{
    decide, NackPollSnapshot, ReceiveFeedbackAction, ReceiveFeedbackArbiterInput,
    ReceiveFeedbackCoalescing, ReceiveFeedbackDecision,
};
use crate::transport::rtc::receive::insert_gate::InsertContext;
use crate::transport::rtc::receive::insert_gate::InsertDecision;
use crate::transport::rtc::receive::keyframe_requester::KeyframeRequestDispatch;
use crate::transport::rtc::receive::recovery_ledger::PictureRecoveryTerminalReason;
use crate::transport::rtc::recovery::contract::{
    sparse_idr_rhythm_from_recovery_ledger, sparse_must_idr_projection_mismatch, SparseIdrRhythm,
};

use super::RtcVideoFrameSource;

impl RtcVideoFrameSource {
    pub(crate) fn build_insert_context(
        &self,
        decode: crate::transport::rtc::receive::decode_gate::ReceiverDecodeContext,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> InsertContext {
        self.runtime_stats
            .read(|stats| {
                let ledger = self.trace_ledger.recovery_ledger();
                let reference =
                    self.trace_ledger
                        .reference_chain_observation(stats, now_ms, effective_rtt_ms);
                let gap_age_ms = stats
                    .latest_video_timeline_observation
                    .as_ref()
                    .and_then(|timeline| timeline.gap.as_ref())
                    .map(|gap| (now_ms - gap.observed_at_ms).max(0.0));
                let has_active_gap = self.trace_ledger.has_unresolved_hard_gap_for_internal()
                    || stats
                        .latest_video_timeline_observation
                        .as_ref()
                        .and_then(|timeline| timeline.gap.as_ref())
                        .is_some();
                let nack_in_flight = stats
                    .latest_video_receiver_observation
                    .as_ref()
                    .is_some_and(|obs| obs.nack_in_flight);
                let action_stage = ledger.derive_packet_recovery_action_stage(
                    has_active_gap,
                    nack_in_flight,
                    gap_age_ms,
                    effective_rtt_ms,
                );
                InsertContext::from_ledger_inputs(
                    decode,
                    reference,
                    action_stage,
                    ledger.keyframe_required,
                    stats,
                    now_ms,
                    effective_rtt_ms,
                )
            })
            .unwrap_or(InsertContext::from_runtime(
                decode,
                &crate::XbxEngineMediaRuntimeStats::default(),
                now_ms,
                effective_rtt_ms,
            ))
    }
}

impl RtcVideoFrameSource {
    pub(crate) fn feedback_target_available_from_stats(
        stats: &crate::XbxEngineMediaRuntimeStats,
    ) -> bool {
        !matches!(
            stats.latest_feedback_target_availability_state.as_deref(),
            Some("unavailable" | "pending")
        )
    }

    pub(crate) fn build_receive_feedback_input(
        &self,
        _source: &'static str,
        now_ms: f64,
        effective_rtt_ms: f64,
        nack: NackPollSnapshot,
        insert_decision: Option<InsertDecision>,
        force_keyframe: bool,
        soft_keyframe: bool,
    ) -> ReceiveFeedbackArbiterInput {
        let (sparse_idr, reference_state, feedback_target_available, ledger) = self
            .runtime_stats
            .read(|stats| {
                let reference =
                    self.trace_ledger
                        .reference_chain_observation(stats, now_ms, effective_rtt_ms);
                let ledger = self.trace_ledger.recovery_ledger().clone();
                let sparse = sparse_idr_rhythm_from_recovery_ledger(
                    &ledger,
                    stats,
                    now_ms,
                    effective_rtt_ms,
                );
                (
                    sparse,
                    reference.state,
                    Self::feedback_target_available_from_stats(stats),
                    ledger,
                )
            })
            .unwrap_or_default();

        let requester = &self.receive_core().receive_engine.keyframe_requester;
        let interval = requester.pli_interval_for_rhythm_public(sparse_idr);
        let pli_coalesced = requester.pli_sent_within_interval_public(interval);
        let pli_throttled = !force_keyframe
            && ((sparse_idr.active && !sparse_idr.pli_due)
                || !requester.should_request_keyframe_with_interval_public(interval));

        ReceiveFeedbackArbiterInput {
            sparse_idr,
            nack,
            insert_decision,
            reference_state,
            feedback_target_available,
            force_keyframe,
            soft_keyframe,
            consecutive_pli_without_idr: requester.consecutive_pli_without_idr_public(),
            fir_after_pli_count: requester.fir_after_pli_count_public(),
            pli_coalesced,
            pli_throttled,
            keyframe_required: ledger.keyframe_required,
            keyframe_required_cause: ledger.keyframe_required_cause,
            response_state: ledger.response_state.as_str(),
            terminal_candidate: ledger.terminal_candidate,
            ledger_generation: ledger.ledger_generation,
        }
    }

    pub(crate) fn maybe_align_recovery_ledger_transport_epoch(&mut self) {
        let epoch = self
            .runtime_stats
            .read(|stats| stats.transport_recovery_epoch)
            .unwrap_or(0);
        let epoch_changed = self
            .trace_ledger
            .recovery_ledger()
            .bound_transport_recovery_epoch
            != epoch;
        self.trace_ledger
            .recovery_ledger_mut()
            .reset_for_transport_recovery_epoch(epoch);
        if epoch_changed {
            self.last_consumed_clean_anchor_epoch = 0;
        }
    }

    pub(crate) fn sync_recovery_ledger_to_stats(&mut self) {
        self.maybe_align_recovery_ledger_transport_epoch();
        self.runtime_stats.update(|stats| {
            self.trace_ledger.recovery_ledger().sync_to_stats(stats);
        });
    }

    pub(crate) fn sparse_idr_rhythm_for_receive(&self, now_ms: f64) -> SparseIdrRhythm {
        self.runtime_stats
            .read(|stats| {
                let effective_rtt_ms = stats.recovery_effective_rtt_ms.unwrap_or(200.0);
                sparse_idr_rhythm_from_recovery_ledger(
                    self.trace_ledger.recovery_ledger(),
                    stats,
                    now_ms,
                    effective_rtt_ms,
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn refresh_recovery_ledger_decoder_facts(&mut self, now_ms: f64) {
        self.runtime_stats.update(|stats| {
            let ledger = self.trace_ledger.recovery_ledger_mut();
            // decoder 观测 → ledger（单向）；不再从 displayed-idr 诊断投影回写 ledger。
            ledger.apply_decoder_facts_from_stats(stats, now_ms);
        });
        self.sync_recovery_ledger_to_stats();
    }

    pub(crate) fn record_reference_chain_state(&mut self, now_ms: f64, effective_rtt_ms: f64) {
        let observation = self
            .runtime_stats
            .read(|stats| {
                self.trace_ledger
                    .reference_chain_observation(stats, now_ms, effective_rtt_ms)
            })
            .unwrap_or_default();
        self.sync_recovery_ledger_to_stats();
        let sparse_mismatch = self
            .runtime_stats
            .read(|stats| sparse_must_idr_projection_mismatch(stats, now_ms))
            .unwrap_or(false);
        self.runtime_stats.update(|stats| {
            stats.latest_reference_chain_observation_source = Some("ledger".to_string());
            let next = observation.state.as_str().to_string();
            let state_changed = stats.reference_chain_state.as_deref() != Some(next.as_str());
            if state_changed {
                stats.reference_chain_state = Some(next);
                stats.reference_chain_state_cause = Some(observation.cause.to_string());
            }
            stats.reference_chain_decoder_reference_synced =
                Some(observation.decoder_reference_synced);
            stats.reference_chain_bootstrap_ready = Some(observation.bootstrap_ready);
            stats.reference_chain_has_active_gap = Some(observation.has_active_gap);
            stats.reference_chain_nack_exhausted = Some(observation.nack_exhausted);
            stats.reference_chain_submit_age_ms = observation.submit_age_ms;
            if sparse_mismatch {
                stats.receive_sparse_must_idr_mismatch_total = stats
                    .receive_sparse_must_idr_mismatch_total
                    .saturating_add(1);
            }
            if sparse_mismatch {
                stats.latest_reference_chain_sparse_must_idr_mismatch = Some(true);
            }
        });
    }

    pub(crate) fn record_receive_feedback_decision(
        &self,
        decision: ReceiveFeedbackDecision,
        source: &'static str,
        actual_action: Option<ReceiveFeedbackAction>,
    ) {
        self.runtime_stats.update(|stats| {
            stats.receive_feedback_decision_seq =
                stats.receive_feedback_decision_seq.saturating_add(1);
            stats.latest_receive_feedback_action = Some(decision.action.as_str().to_string());
            stats.latest_receive_feedback_reason = Some(decision.reason.to_string());
            stats.latest_receive_feedback_coalescing =
                Some(decision.coalescing.as_str().to_string());
            stats.latest_receive_feedback_source = Some(source.to_string());
            stats.latest_receive_feedback_sparse_active = decision.sparse_active;
            match decision.coalescing {
                ReceiveFeedbackCoalescing::SameInterval => {
                    stats.receive_feedback_coalesced_total =
                        stats.receive_feedback_coalesced_total.saturating_add(1);
                }
                ReceiveFeedbackCoalescing::RateLimited => {
                    stats.receive_feedback_throttled_total =
                        stats.receive_feedback_throttled_total.saturating_add(1);
                }
                _ => {}
            }
            if let Some(actual) = actual_action {
                if actual != decision.action {
                    stats.receive_feedback_arbiter_mismatch_total = stats
                        .receive_feedback_arbiter_mismatch_total
                        .saturating_add(1);
                }
            }
            if let Some(outcome) = stats.latest_keyframe_request_outcome.as_deref() {
                stats.latest_receive_feedback_executor_outcome = Some(outcome.to_string());
            }
        });
    }

    pub(crate) fn plan_receive_feedback(
        &mut self,
        source: &'static str,
        now_ms: f64,
        effective_rtt_ms: f64,
        nack: NackPollSnapshot,
        insert_decision: Option<InsertDecision>,
        force_keyframe: bool,
        soft_keyframe: bool,
    ) -> ReceiveFeedbackDecision {
        self.refresh_recovery_ledger_decoder_facts(now_ms);
        self.record_reference_chain_state(now_ms, effective_rtt_ms);
        let input = self.build_receive_feedback_input(
            source,
            now_ms,
            effective_rtt_ms,
            nack,
            insert_decision,
            force_keyframe,
            soft_keyframe,
        );
        decide(&input)
    }

    pub(crate) fn execute_receive_feedback_keyframe(
        &mut self,
        decision: ReceiveFeedbackDecision,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
        force: bool,
    ) -> KeyframeRequestDispatch {
        let capability = self.receive_core().transport_capability.clone();
        let sparse_idr_rhythm = self
            .runtime_stats
            .read(|stats| {
                sparse_idr_rhythm_from_recovery_ledger(
                    self.trace_ledger.recovery_ledger(),
                    stats,
                    now_ms,
                    stats.recovery_effective_rtt_ms.unwrap_or(200.0),
                )
            })
            .unwrap_or_default();
        let dispatch = self
            .receive_core_mut()
            .receive_engine
            .keyframe_requester
            .request_dispatch(capability.as_ref(), force, sparse_idr_rhythm);
        let actual = match dispatch {
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::Sent) => Some(
                if matches!(decision.action, ReceiveFeedbackAction::RequestFir) {
                    ReceiveFeedbackAction::RequestFir
                } else {
                    ReceiveFeedbackAction::RequestPli
                },
            ),
            KeyframeRequestDispatch::Throttled => Some(ReceiveFeedbackAction::None),
            KeyframeRequestDispatch::Coalesced => Some(ReceiveFeedbackAction::None),
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::FeedbackUnavailable) => {
                Some(ReceiveFeedbackAction::None)
            }
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::FeedbackWarming) => {
                Some(ReceiveFeedbackAction::None)
            }
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::TransportNotReady) => {
                Some(ReceiveFeedbackAction::None)
            }
        };
        self.write_keyframe_request_outcome_stats(
            source_event,
            frame_rtp_timestamp,
            now_ms,
            dispatch,
            sparse_idr_rhythm,
        );
        self.record_receive_feedback_decision(decision, source_event, actual);
        let effective_rtt_ms = self
            .runtime_stats
            .read(|stats| stats.recovery_effective_rtt_ms.unwrap_or(200.0))
            .unwrap_or(200.0);
        self.maybe_emit_picture_recovery_terminal(now_ms, effective_rtt_ms);
        dispatch
    }

    const REMOTE_NO_USABLE_IDR_SENT_MIN: u32 = 5;
    const REMOTE_NO_USABLE_IDR_RTT_MULT: f64 = 3.0;

    pub(crate) fn note_usable_idr_for_picture_recovery_terminal(
        &mut self,
        rtp_timestamp: u32,
        now_ms: f64,
    ) {
        self.trace_ledger
            .recovery_ledger_mut()
            .note_usable_idr_packet_accepted(rtp_timestamp, now_ms);
        self.sync_recovery_ledger_to_stats();
        self.runtime_stats.update(|stats| {
            stats.receive_keyframe_sent_count_unresolved = 0;
            stats.latest_receive_picture_recovery_terminal_reason = None;
        });
    }

    pub(crate) fn note_non_idr_continuation_for_recovery_ledger(&mut self) {
        self.trace_ledger
            .recovery_ledger_mut()
            .note_non_idr_continuation();
        self.sync_recovery_ledger_to_stats();
    }

    pub(crate) fn note_first_delta_for_recovery_ledger(&mut self) {
        self.trace_ledger.recovery_ledger_mut().note_first_delta();
        self.sync_recovery_ledger_to_stats();
    }

    pub(crate) fn maybe_emit_picture_recovery_terminal(
        &mut self,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) {
        let ledger = self.trace_ledger.recovery_ledger().clone();
        if !ledger.picture_recovery_terminal_eligible() {
            return;
        }
        if ledger.terminal_reason != PictureRecoveryTerminalReason::None {
            return;
        }
        let first_sent_at_ms = ledger.last_keyframe_request_sent_at_ms;
        let sent_count = ledger.unresolved_keyframe_request_count;
        let elapsed_rtt_count = first_sent_at_ms
            .map(|first| ((now_ms - first) / effective_rtt_ms.max(1.0)).floor() as u32)
            .unwrap_or(0);
        let clean_anchor_patience_ms = self
            .runtime_stats
            .read(|stats| {
                stats
                    .recovery_dynamic_clean_anchor_patience_ms
                    .unwrap_or(3.0 * effective_rtt_ms.max(1.0))
            })
            .unwrap_or(3.0 * effective_rtt_ms.max(1.0));
        let reason = ledger
            .terminal_reason_for_remote_no_usable_idr(
                now_ms,
                effective_rtt_ms,
                clean_anchor_patience_ms,
            )
            .unwrap_or(PictureRecoveryTerminalReason::None);
        if reason == PictureRecoveryTerminalReason::None {
            return;
        }
        if matches!(
            reason,
            PictureRecoveryTerminalReason::RemoteNoResponse
                | PictureRecoveryTerminalReason::RemoteContinuationOnly
        ) && sent_count < Self::REMOTE_NO_USABLE_IDR_SENT_MIN
            && first_sent_at_ms.is_none_or(|first| {
                now_ms - first < Self::REMOTE_NO_USABLE_IDR_RTT_MULT * effective_rtt_ms.max(1.0)
            })
        {
            return;
        }
        let reason_str = reason.as_str().to_string();
        self.trace_ledger
            .recovery_ledger_mut()
            .emit_terminal(reason);
        self.runtime_stats.update(|stats| {
            stats.receive_picture_recovery_terminal_total = stats
                .receive_picture_recovery_terminal_total
                .saturating_add(1);
            stats.latest_receive_picture_recovery_terminal_reason = Some(reason_str.clone());
            stats.latest_receive_feedback_reason = Some(reason_str);
            stats.receive_picture_recovery_terminal_elapsed_rtt_count = Some(elapsed_rtt_count);
        });
        self.sync_recovery_ledger_to_stats();
        let _ = now_ms;
    }

    #[allow(dead_code)]
    pub(crate) fn maybe_emit_remote_no_usable_idr_terminal(
        &mut self,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) {
        self.maybe_emit_picture_recovery_terminal(now_ms, effective_rtt_ms);
    }

    fn write_keyframe_request_outcome_stats(
        &mut self,
        source_event: &'static str,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
        dispatch: KeyframeRequestDispatch,
        sparse_idr_rhythm: crate::transport::rtc::recovery::contract::SparseIdrRhythm,
    ) {
        let outcome_name = match dispatch {
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::Sent) => "sent",
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::FeedbackWarming) => {
                "feedbackWarming"
            }
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::FeedbackUnavailable) => {
                "feedbackUnavailable"
            }
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::TransportNotReady) => {
                "transportNotReady"
            }
            KeyframeRequestDispatch::Throttled => "throttled",
            KeyframeRequestDispatch::Coalesced => "coalesced",
        };
        if matches!(
            dispatch,
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::Sent)
        ) {
            self.trace_ledger
                .recovery_ledger_mut()
                .note_keyframe_request_sent(now_ms);
            self.sync_recovery_ledger_to_stats();
            if self.waiting_recovery_keyframe_since_ms.is_none() {
                self.waiting_recovery_keyframe_since_ms = Some(now_ms);
            }
            self.queue_transport_observation(
                crate::transport::rtc::stream::adapter_types::TransportObservation::Loss(
                    crate::transport::rtc::stream::adapter_types::TransportLossObservation::RecoveryKeyframeRequested,
                ),
            );
        }
        self.runtime_stats.update(|stats| {
            stats.keyframe_request_outcome_seq =
                stats.keyframe_request_outcome_seq.saturating_add(1);
            stats.latest_keyframe_request_source = Some(source_event.to_string());
            stats.latest_keyframe_request_outcome = Some(outcome_name.to_string());
            if sparse_idr_rhythm.active {
                stats.receive_sparse_idr_pli_interval_ms = Some(sparse_idr_rhythm.pli_interval_ms);
                stats.latest_receive_feedback_sparse_active = true;
            } else {
                stats.receive_sparse_idr_pli_interval_ms = None;
                stats.latest_receive_feedback_sparse_active = false;
            }
            if matches!(
                dispatch,
                KeyframeRequestDispatch::Sent(KeyframeSendOutcome::Sent)
            ) {
                stats.receive_keyframe_last_sent_at_ms = Some(now_ms);
            }
            stats.latest_observation_label = Some("keyframeRequestOutcome".to_string());
            stats.latest_observation_summary = Some(format!(
                "seq={} source={source_event} outcome={outcome_name}",
                stats.keyframe_request_outcome_seq
            ));
        });
        let _ = frame_rtp_timestamp;
        self.publish_receiver_observation(now_ms, None);
    }
}
