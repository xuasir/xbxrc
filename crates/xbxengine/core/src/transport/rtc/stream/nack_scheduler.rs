//! WebRTC NACK 调度：包级时效、可恢复性与 `PacketRecoveryDisposition`。
//! RFC：包价值评估归属本层；禁止在此直接决定 reconnect / failed-terminal（见 `session::policy`）。

use std::collections::{BTreeMap, BTreeSet};

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameValue;

use super::video_source::timeline::GapState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketRecoveryDisposition {
    Attempted,
    SkippedTooLate,
    SkippedLowValue,
    SkippedChainBroken,
}

impl PacketRecoveryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
            Self::SkippedTooLate => "skippedTooLate",
            Self::SkippedLowValue => "skippedLowValue",
            Self::SkippedChainBroken => "skippedChainBroken",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NackSchedulerConfig {
    pub max_age_ms: u64,
    pub frame_deadline_ms: u64,
    pub burst_count: u16,
    pub retry_interval_ms: u64,
    pub max_retry_count: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NackBatch {
    pub sequences: Vec<u16>,
    pub retry_count: u8,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNack {
    pub sequence: u16,
    pub recovery_time_ms: f64,
    pub retry_count: u8,
    pub was_late: bool,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpiredNackBatch {
    pub sequences: Vec<u16>,
    pub reason: String,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkippedNackBatch {
    pub sequences: Vec<u16>,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NackPollResult {
    pub retry_batch: Option<NackBatch>,
    pub expired_batches: Vec<ExpiredNackBatch>,
}

#[derive(Clone, Debug)]
struct PendingNack {
    gap_state: GapState,
    first_seen_at_ms: f64,
    last_sent_at_ms: f64,
    deadline_at_ms: f64,
    retry_count: u8,
    max_retry_count: u8,
    max_age_ms: u64,
    retry_interval_ms: u64,
    source: &'static str,
    frame_rtp_timestamp: Option<u32>,
    frame_is_keyframe: Option<bool>,
    frame_importance: &'static str,
    priority: u8,
    estimated_recovery_arrival_ms: Option<f64>,
    frame_playout_deadline_at_ms: Option<f64>,
    frame_unrecoverable_reason: Option<&'static str>,
    budget_context: FrameBudgetContext,
}

#[derive(Clone, Copy, Debug)]
struct SkippedAdmissionRecord {
    reason: &'static str,
    emitted_at_ms: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct NackObservePolicy {
    pub source: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub max_age_ms: Option<u64>,
    pub retry_interval_ms: Option<u64>,
    pub burst_count: Option<u16>,
    pub max_tracked_sequences: Option<u16>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub priority: u8,
    pub budget_context: FrameBudgetContext,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

pub struct NackScheduler {
    config: NackSchedulerConfig,
    pending: BTreeMap<u16, PendingNack>,
    skipped_low_value: BTreeMap<u16, SkippedAdmissionRecord>,
    /// 最近的RTT（毫秒），用于动态预算调整
    latest_rtt_ms: Option<f64>,
    /// 最近的丢包率（0.0-1.0），用于动态预算调整
    latest_loss_rate: Option<f64>,
}

fn should_bypass_low_value_skip(policy: NackObservePolicy) -> bool {
    policy.frame_is_keyframe.unwrap_or(false)
        || matches!(policy.frame_importance, "anchor" | "supply")
}

type PendingMeta = (
    &'static str,
    Option<u32>,
    Option<bool>,
    &'static str,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<&'static str>,
    FrameBudgetContext,
);

impl NackScheduler {
    pub fn new(config: NackSchedulerConfig) -> Self {
        Self {
            config,
            pending: BTreeMap::new(),
            skipped_low_value: BTreeMap::new(),
            latest_rtt_ms: None,
            latest_loss_rate: None,
        }
    }

    /// 更新网络状态（用于动态预算调整）
    pub fn update_network_stats(&mut self, rtt_ms: Option<f64>, loss_rate: Option<f64>) {
        self.latest_rtt_ms = rtt_ms;
        self.latest_loss_rate = loss_rate;
    }

    pub fn observe_missing_sequences_with_policy(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        policy: NackObservePolicy,
    ) -> (Option<NackBatch>, Option<SkippedNackBatch>) {
        if sequences.is_empty() {
            return (None, None);
        }

        let burst_count = usize::from(policy.burst_count.unwrap_or(self.config.burst_count).max(1));
        let max_tracked_sequences = usize::from(
            policy
                .max_tracked_sequences
                .unwrap_or((burst_count.saturating_mul(2)).min(u16::MAX as usize) as u16)
                .max(1),
        );
        let retry_interval_ms = policy
            .retry_interval_ms
            .unwrap_or(self.config.retry_interval_ms);
        let max_age_ms = policy.max_age_ms.unwrap_or(self.config.max_age_ms);
        let deadline_at_ms = policy
            .deadline_at_ms
            .unwrap_or(now_ms + self.config.frame_deadline_ms as f64);
        let estimated_recovery_arrival_ms = policy.estimated_recovery_arrival_ms;
        let max_retry_count = frame_importance_retry_budget(policy, self.config.max_retry_count);

        if now_ms >= deadline_at_ms {
            let skipped = SkippedNackBatch {
                sequences: sequences
                    .iter()
                    .copied()
                    .take(max_tracked_sequences)
                    .collect(),
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy
                    .frame_playout_deadline_at_ms
                    .or(Some(deadline_at_ms)),
                nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                frame_unrecoverable_reason: Some("deadlineExceededBeforeAdmission"),
                budget_context: policy.budget_context,
            };
            return (None, Some(skipped));
        }

        if matches!(
            policy.nack_disposition,
            PacketRecoveryDisposition::Attempted
        ) && estimated_recovery_arrival_ms.is_some_and(|arrival| arrival > deadline_at_ms)
        {
            let skipped = SkippedNackBatch {
                sequences: sequences
                    .iter()
                    .copied()
                    .take(max_tracked_sequences)
                    .collect(),
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy
                    .frame_playout_deadline_at_ms
                    .or(Some(deadline_at_ms)),
                nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                frame_unrecoverable_reason: Some("estimatedArrivalPastDeadline"),
                budget_context: policy.budget_context,
            };
            return (None, Some(skipped));
        }

        if matches!(
            policy.nack_disposition,
            PacketRecoveryDisposition::SkippedTooLate
                | PacketRecoveryDisposition::SkippedChainBroken
        ) {
            let skipped = SkippedNackBatch {
                sequences: sequences
                    .iter()
                    .copied()
                    .take(max_tracked_sequences)
                    .collect(),
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy.frame_playout_deadline_at_ms,
                nack_disposition: policy.nack_disposition,
                frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
                budget_context: policy.budget_context,
            };
            return (None, Some(skipped));
        }

        if matches!(
            policy.nack_disposition,
            PacketRecoveryDisposition::SkippedLowValue
        ) {
            if should_bypass_low_value_skip(policy) {
                return self.observe_missing_sequences_with_policy(
                    sequences,
                    now_ms,
                    NackObservePolicy {
                        nack_disposition: PacketRecoveryDisposition::Attempted,
                        frame_unrecoverable_reason: None,
                        ..policy
                    },
                );
            }
            const LOW_VALUE_SKIP_SUPPRESS_MS: f64 = 250.0;
            let unrecoverable_reason = policy
                .frame_unrecoverable_reason
                .unwrap_or("cloudHighRttLowValueAdmission");
            self.skipped_low_value.retain(|_, record| {
                (now_ms - record.emitted_at_ms).max(0.0) < LOW_VALUE_SKIP_SUPPRESS_MS
            });
            let filtered_sequences: Vec<u16> = sequences
                .iter()
                .copied()
                .take(max_tracked_sequences)
                .filter(|sequence| {
                    !matches!(
                        self.skipped_low_value.get(sequence),
                        Some(record)
                            if record.reason == unrecoverable_reason
                                && (now_ms - record.emitted_at_ms).max(0.0)
                                    < LOW_VALUE_SKIP_SUPPRESS_MS
                    )
                })
                .collect();
            if filtered_sequences.is_empty() {
                return (None, None);
            }
            for sequence in &filtered_sequences {
                self.skipped_low_value.insert(
                    *sequence,
                    SkippedAdmissionRecord {
                        reason: unrecoverable_reason,
                        emitted_at_ms: now_ms,
                    },
                );
            }
            let skipped = SkippedNackBatch {
                sequences: filtered_sequences,
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy.frame_playout_deadline_at_ms,
                nack_disposition: policy.nack_disposition,
                frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
                budget_context: policy.budget_context,
            };
            return (None, Some(skipped));
        }

        let mut inserted = Vec::new();
        for (index, sequence) in sequences.iter().take(max_tracked_sequences).enumerate() {
            if let Some(pending) = self.pending.get_mut(sequence) {
                // 已有 pending 时执行“有界合并”：
                // 1) deadline/max_age/retry_interval 只朝更严格方向收敛；
                // 2) retry budget/priority 朝更积极方向提升，避免路径先后顺序影响恢复强度。
                pending.deadline_at_ms = pending.deadline_at_ms.min(deadline_at_ms);
                pending.max_age_ms = pending.max_age_ms.min(max_age_ms);
                // retry_interval 收紧后，把 last_sent_at_ms 对齐到 now_ms，
                // 避免旧时间戳导致下次 poll 时用新的更短间隔提前触发额外重试。
                if retry_interval_ms < pending.retry_interval_ms {
                    pending.last_sent_at_ms = now_ms;
                }
                pending.retry_interval_ms = pending.retry_interval_ms.min(retry_interval_ms);
                pending.max_retry_count = pending.max_retry_count.max(max_retry_count);
                pending.priority = pending.priority.max(policy.priority);
                pending.frame_is_keyframe = Some(
                    pending.frame_is_keyframe.unwrap_or(false)
                        || policy.frame_is_keyframe.unwrap_or(false),
                );
                if pending.source != "rtpGap" && policy.source == "rtpGap" {
                    pending.source = "rtpGap";
                }
                pending.frame_importance = if pending.frame_is_keyframe.unwrap_or(false) {
                    "anchor"
                } else if matches!(pending.frame_importance, "supply")
                    || matches!(policy.frame_importance, "supply")
                {
                    "supply"
                } else {
                    "disposable"
                };
                pending.frame_rtp_timestamp =
                    pending.frame_rtp_timestamp.or(policy.frame_rtp_timestamp);
                pending.estimated_recovery_arrival_ms = match (
                    pending.estimated_recovery_arrival_ms,
                    estimated_recovery_arrival_ms,
                ) {
                    (Some(existing), Some(next)) => Some(existing.min(next)),
                    (None, Some(next)) => Some(next),
                    (Some(existing), None) => Some(existing),
                    (None, None) => None,
                };
                pending.frame_playout_deadline_at_ms = match (
                    pending.frame_playout_deadline_at_ms,
                    policy.frame_playout_deadline_at_ms.or(Some(deadline_at_ms)),
                ) {
                    (Some(existing), Some(next)) => Some(existing.min(next)),
                    (None, Some(next)) => Some(next),
                    (Some(existing), None) => Some(existing),
                    (None, None) => None,
                };
                continue;
            }
            let last_sent_at_ms = if inserted.len() < burst_count && index < burst_count {
                now_ms
            } else {
                now_ms - retry_interval_ms as f64
            };
            self.pending.insert(
                *sequence,
                PendingNack {
                    gap_state: GapState::NackCandidate,
                    first_seen_at_ms: now_ms,
                    last_sent_at_ms,
                    deadline_at_ms,
                    retry_count: 0,
                    max_retry_count,
                    max_age_ms,
                    retry_interval_ms,
                    source: policy.source,
                    frame_rtp_timestamp: policy.frame_rtp_timestamp,
                    frame_is_keyframe: policy.frame_is_keyframe,
                    frame_importance: policy.frame_importance,
                    priority: policy.priority,
                    estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms: policy
                        .frame_playout_deadline_at_ms
                        .or(Some(deadline_at_ms)),
                    frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
                    budget_context: policy.budget_context,
                },
            );
            inserted.push(*sequence);
        }

        let batch = if inserted.is_empty() {
            None
        } else {
            Some(NackBatch {
                sequences: inserted.into_iter().take(burst_count).collect(),
                retry_count: 0,
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy
                    .frame_playout_deadline_at_ms
                    .or(Some(deadline_at_ms)),
                nack_disposition: PacketRecoveryDisposition::Attempted,
                frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
                budget_context: policy.budget_context,
            })
        };
        (batch, None)
    }

    pub fn poll(&mut self, now_ms: f64) -> NackPollResult {
        let mut expired_deadline_sequences = Vec::new();
        let mut expired_max_age_sequences = Vec::new();
        let mut expired_retry_budget_sequences = Vec::new();
        let mut expired_deadline_meta = None;
        let mut expired_max_age_meta = None;
        let mut expired_retry_budget_meta = None;
        self.pending.retain(|sequence, pending| {
            let age_ms = (now_ms - pending.first_seen_at_ms).max(0.0);
            if now_ms >= pending.deadline_at_ms {
                expired_deadline_sequences.push(*sequence);
                expired_deadline_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                    pending.budget_context,
                ));
                return false;
            }
            if age_ms >= pending.max_age_ms as f64 {
                expired_max_age_sequences.push(*sequence);
                expired_max_age_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                    pending.budget_context,
                ));
                return false;
            }
            true
        });

        // deadline/maxAge 先于预算耗尽处理，避免覆盖更高优先级过期原因。
        // 检查预算耗尽：直接使用主预算
        let exhausted_sequences: Vec<u16> = self
            .pending
            .iter()
            .filter_map(|(sequence, pending)| {
                (pending.retry_count >= pending.max_retry_count).then_some(*sequence)
            })
            .collect();
        for sequence in exhausted_sequences {
            if let Some(pending) = self.pending.remove(&sequence) {
                expired_retry_budget_sequences.push(sequence);
                expired_retry_budget_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                    pending.budget_context,
                ));
            }
        }

        // 筛选重试候选：直接使用主预算（pending.max_retry_count）
        let mut retry_candidates = Vec::new();
        for (sequence, pending) in &mut self.pending {
            let since_last_sent_ms = (now_ms - pending.last_sent_at_ms).max(0.0);

            // 预算检查：直接使用主预算，不做二次判死
            if pending.retry_count >= pending.max_retry_count {
                continue;
            }

            if since_last_sent_ms < pending.retry_interval_ms as f64 {
                continue;
            }
            retry_candidates.push((
                *sequence,
                pending.priority,
                pending.first_seen_at_ms,
                pending.source,
                pending.frame_rtp_timestamp,
                pending.frame_is_keyframe,
                pending.frame_importance,
                pending.deadline_at_ms,
                pending.estimated_recovery_arrival_ms,
                pending.frame_playout_deadline_at_ms,
                pending.frame_unrecoverable_reason,
                pending.budget_context,
            ));
        }
        retry_candidates.sort_by(|left, right| {
            right.1.cmp(&left.1).then_with(|| {
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let mut retry_sequences = Vec::new();
        let mut next_retry_count = 0u8;
        let mut retry_meta = None;
        let burst_count = usize::from(self.config.burst_count.max(1));
        for (
            sequence,
            _,
            _,
            source,
            frame_rtp_timestamp,
            frame_is_keyframe,
            frame_importance,
            deadline_at_ms,
            estimated_recovery_arrival_ms,
            frame_playout_deadline_at_ms,
            frame_unrecoverable_reason,
            budget_context,
        ) in retry_candidates.into_iter().take(burst_count)
        {
            if let Some(pending) = self.pending.get_mut(&sequence) {
                pending.gap_state = GapState::RepairInFlight;
                pending.retry_count = pending.retry_count.saturating_add(1);
                pending.last_sent_at_ms = now_ms;
                next_retry_count = pending.retry_count;
                retry_sequences.push(sequence);
                retry_meta.get_or_insert((
                    source,
                    frame_rtp_timestamp,
                    frame_is_keyframe,
                    frame_importance,
                    Some(deadline_at_ms),
                    estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms,
                    frame_unrecoverable_reason,
                    budget_context,
                ));
            }
        }

        NackPollResult {
            retry_batch: if retry_sequences.is_empty() {
                None
            } else {
                Some({
                    let (
                        source,
                        frame_rtp_timestamp,
                        frame_is_keyframe,
                        frame_importance,
                        deadline_at_ms,
                        estimated_recovery_arrival_ms,
                        frame_playout_deadline_at_ms,
                        frame_unrecoverable_reason,
                        budget_context,
                    ) = retry_meta.unwrap_or((
                        "rtpWindow",
                        None,
                        None,
                        "unknown",
                        None,
                        None,
                        None,
                        None,
                        FrameBudgetContext::steady_for_value(frame_value_for_importance("unknown")),
                    ));
                    NackBatch {
                        sequences: retry_sequences,
                        retry_count: next_retry_count,
                        source,
                        frame_rtp_timestamp,
                        frame_is_keyframe,
                        frame_importance,
                        deadline_at_ms,
                        estimated_recovery_arrival_ms,
                        frame_playout_deadline_at_ms: frame_playout_deadline_at_ms
                            .or(deadline_at_ms),
                        nack_disposition: PacketRecoveryDisposition::Attempted,
                        frame_unrecoverable_reason,
                        budget_context,
                    }
                })
            },
            expired_batches: vec![
                ExpiredNackBatch {
                    sequences: expired_deadline_sequences,
                    reason: "deadline".to_string(),
                    source: expired_deadline_meta
                        .map(|meta| meta.0)
                        .unwrap_or("rtpWindow"),
                    frame_rtp_timestamp: expired_deadline_meta.and_then(|meta| meta.1),
                    frame_is_keyframe: expired_deadline_meta.and_then(|meta| meta.2),
                    frame_importance: expired_deadline_meta
                        .map(|meta| meta.3)
                        .unwrap_or("unknown"),
                    deadline_at_ms: expired_deadline_meta.and_then(|meta| meta.4),
                    estimated_recovery_arrival_ms: expired_deadline_meta.and_then(|meta| meta.5),
                    frame_playout_deadline_at_ms: expired_deadline_meta
                        .and_then(|meta| meta.6)
                        .or(expired_deadline_meta.and_then(|meta| meta.4)),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: expired_deadline_meta
                        .and_then(|meta| meta.7)
                        .or(Some("deadlineExceeded")),
                    budget_context: expired_deadline_meta.map(|meta| meta.8).unwrap_or_else(|| {
                        FrameBudgetContext::steady_for_value(frame_value_for_importance("unknown"))
                    }),
                },
                ExpiredNackBatch {
                    sequences: expired_max_age_sequences,
                    reason: "maxAge".to_string(),
                    source: expired_max_age_meta
                        .map(|meta| meta.0)
                        .unwrap_or("rtpWindow"),
                    frame_rtp_timestamp: expired_max_age_meta.and_then(|meta| meta.1),
                    frame_is_keyframe: expired_max_age_meta.and_then(|meta| meta.2),
                    frame_importance: expired_max_age_meta.map(|meta| meta.3).unwrap_or("unknown"),
                    deadline_at_ms: expired_max_age_meta.and_then(|meta| meta.4),
                    estimated_recovery_arrival_ms: expired_max_age_meta.and_then(|meta| meta.5),
                    frame_playout_deadline_at_ms: expired_max_age_meta
                        .and_then(|meta| meta.6)
                        .or(expired_max_age_meta.and_then(|meta| meta.4)),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: expired_max_age_meta
                        .and_then(|meta| meta.7)
                        .or(Some("maxAgeExceeded")),
                    budget_context: expired_max_age_meta.map(|meta| meta.8).unwrap_or_else(|| {
                        FrameBudgetContext::steady_for_value(frame_value_for_importance("unknown"))
                    }),
                },
                ExpiredNackBatch {
                    sequences: expired_retry_budget_sequences,
                    reason: "retryBudget".to_string(),
                    source: expired_retry_budget_meta
                        .map(|meta| meta.0)
                        .unwrap_or("rtpWindow"),
                    frame_rtp_timestamp: expired_retry_budget_meta.and_then(|meta| meta.1),
                    frame_is_keyframe: expired_retry_budget_meta.and_then(|meta| meta.2),
                    frame_importance: expired_retry_budget_meta
                        .map(|meta| meta.3)
                        .unwrap_or("unknown"),
                    deadline_at_ms: expired_retry_budget_meta.and_then(|meta| meta.4),
                    estimated_recovery_arrival_ms: expired_retry_budget_meta
                        .and_then(|meta| meta.5),
                    frame_playout_deadline_at_ms: expired_retry_budget_meta
                        .and_then(|meta| meta.6)
                        .or(expired_retry_budget_meta.and_then(|meta| meta.4)),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: expired_retry_budget_meta
                        .and_then(|meta| meta.7)
                        .or(Some("retryBudgetExhausted")),
                    budget_context: expired_retry_budget_meta.map(|meta| meta.8).unwrap_or_else(
                        || {
                            FrameBudgetContext::steady_for_value(frame_value_for_importance(
                                "unknown",
                            ))
                        },
                    ),
                },
            ]
            .into_iter()
            .filter(|batch| !batch.sequences.is_empty())
            .collect(),
        }
    }

    pub fn resolve_sequence(&mut self, sequence: u16, now_ms: f64) -> Option<ResolvedNack> {
        self.pending.remove(&sequence).map(|mut pending| {
            pending.gap_state = GapState::Resolved;
            // 如果resolve_sequence被调用，说明包已经成功到达并被sample builder接受
            // 即使晚到，也应该标记为成功恢复，而不是SkippedTooLate
            let was_late = now_ms >= pending.deadline_at_ms;
            ResolvedNack {
                sequence,
                recovery_time_ms: (now_ms - pending.first_seen_at_ms).max(0.0),
                retry_count: pending.retry_count,
                was_late,
                source: pending.source,
                frame_rtp_timestamp: pending.frame_rtp_timestamp,
                frame_is_keyframe: pending.frame_is_keyframe,
                frame_importance: pending.frame_importance,
                deadline_at_ms: Some(pending.deadline_at_ms),
                estimated_recovery_arrival_ms: pending.estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: pending
                    .frame_playout_deadline_at_ms
                    .or(Some(pending.deadline_at_ms)),
                // 包已经到达并被接受，标记为成功恢复（可能晚到但仍有效）
                nack_disposition: PacketRecoveryDisposition::Attempted,
                // 包已经成功恢复，不设置unrecoverable_reason
                frame_unrecoverable_reason: None,
                budget_context: pending.budget_context,
            }
        })
    }

    pub fn prune_rtp_window_pending_not_missing(&mut self, still_missing: &[u16]) -> Vec<u16> {
        let missing: BTreeSet<u16> = still_missing.iter().copied().collect();
        let stale_sequences: Vec<u16> = self
            .pending
            .iter()
            .filter_map(|(sequence, pending)| {
                (pending.source == "rtpWindow" && !missing.contains(sequence)).then_some(*sequence)
            })
            .collect();
        for sequence in &stale_sequences {
            self.pending.remove(sequence);
        }
        stale_sequences
    }

    pub fn prune_pending_in_range(&mut self, start: u16, end_exclusive: u16) -> Vec<u16> {
        let stale_sequences: Vec<u16> = self
            .pending
            .keys()
            .copied()
            .filter(|sequence| sequence_in_wrapping_range(*sequence, start, end_exclusive))
            .collect();
        for sequence in &stale_sequences {
            self.pending.remove(sequence);
        }
        stale_sequences
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn flush_non_keyframe_pending(&mut self, reason: &'static str) -> Option<ExpiredNackBatch> {
        let non_keyframe_sequences: Vec<u16> = self
            .pending
            .iter()
            .filter_map(|(sequence, pending)| {
                (!matches!(pending.frame_is_keyframe, Some(true))).then_some(*sequence)
            })
            .collect();
        let mut flushed_sequences = Vec::new();
        let mut flushed_meta: Option<PendingMeta> = None;
        for sequence in non_keyframe_sequences {
            if let Some(pending) = self.pending.remove(&sequence) {
                flushed_sequences.push(sequence);
                flushed_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                    pending.budget_context,
                ));
            }
        }
        if flushed_sequences.is_empty() {
            return None;
        }
        Some(ExpiredNackBatch {
            sequences: flushed_sequences,
            reason: "chainBroken".to_string(),
            source: flushed_meta.map(|meta| meta.0).unwrap_or("rtpWindow"),
            frame_rtp_timestamp: flushed_meta.and_then(|meta| meta.1),
            frame_is_keyframe: flushed_meta.and_then(|meta| meta.2),
            frame_importance: flushed_meta.map(|meta| meta.3).unwrap_or("unknown"),
            deadline_at_ms: flushed_meta.and_then(|meta| meta.4),
            estimated_recovery_arrival_ms: flushed_meta.and_then(|meta| meta.5),
            frame_playout_deadline_at_ms: flushed_meta
                .and_then(|meta| meta.6)
                .or(flushed_meta.and_then(|meta| meta.4)),
            nack_disposition: PacketRecoveryDisposition::SkippedChainBroken,
            frame_unrecoverable_reason: Some(reason),
            budget_context: flushed_meta.map(|meta| meta.8).unwrap_or_else(|| {
                FrameBudgetContext::steady_for_value(frame_value_for_importance("unknown"))
            }),
        })
    }
}

fn sequence_in_wrapping_range(sequence: u16, start: u16, end_exclusive: u16) -> bool {
    if start == end_exclusive {
        return false;
    }
    if start < end_exclusive {
        (start..end_exclusive).contains(&sequence)
    } else {
        sequence >= start || sequence < end_exclusive
    }
}

fn frame_importance_retry_budget(policy: NackObservePolicy, default_max_retry_count: u8) -> u8 {
    let value = frame_value_for_importance(policy.frame_importance);
    let effective_context =
        if policy.budget_context.recovery_value_tier() == policy.frame_importance {
            policy.budget_context
        } else {
            FrameBudgetContext::steady_for_value(value)
        };
    effective_context.retry_budget(value, default_max_retry_count)
}

fn frame_value_for_importance(frame_importance: &'static str) -> FrameValue {
    match frame_importance {
        "anchor" => FrameValue::new(true, false, 128 * 1024),
        "supply" => FrameValue::new(false, true, 48 * 1024),
        _ => FrameValue::new(false, false, 12 * 1024),
    }
}

#[cfg(test)]
#[path = "nack_scheduler.test.rs"]
mod tests;
