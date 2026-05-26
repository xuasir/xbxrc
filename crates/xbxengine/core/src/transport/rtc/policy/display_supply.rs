use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplaySupplyState {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplaySupplyCriticalSignal {
    None,
    SoftNoPendingAge,
    HardRendererStall,
    HardSupplyDrop,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SchedulingDemandSignal {
    pub(crate) no_pending_pressure_level: Option<String>,
    pub(crate) no_pending_streak: Option<u32>,
    pub(crate) present_age_ms: Option<f64>,
    pub(crate) decode_age_ms: Option<f64>,
    pub(crate) video_renderer_stalled: bool,
    pub(crate) host_display_tick_epoch: Option<u64>,
    pub(crate) host_frame_present_epoch: Option<u64>,
    pub(crate) host_cadence_phase: Option<String>,
    pub(crate) host_mailbox_enqueue_count_total: Option<u64>,
    pub(crate) host_mailbox_drop_count_total: Option<u64>,
    pub(crate) host_mailbox_overwrite_count_total: Option<u64>,
    pub(crate) pacer_submit_count_total: Option<u64>,
    pub(crate) pacer_drop_count_total: Option<u64>,
    pub(crate) renderer_submit_count_total: Option<u64>,
    pub(crate) renderer_drop_count_total: Option<u64>,
    pub(crate) smoothed_present_fps: Option<f64>,
    pub(crate) smoothed_decode_fps: Option<f64>,
    pub(crate) submit_age_ms: Option<f64>,
}

const PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS: f64 = 18.0;
const PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS: f64 = 14.0;
const PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP: f64 = 8.0;
const PRESENT_PIPELINE_PRESENT_AGE_OVER_DECODE_RATIO: f64 = 1.35;

impl SchedulingDemandSignal {
    /// decode 仍新鲜但 present 明显落后：主瓶颈在显示链，不应再抬传输 anchor 恢复。
    pub(crate) fn present_pipeline_stressed(&self, thresholds: &DisplaySupplyThresholds) -> bool {
        if self.host_is_priming_without_present() {
            return false;
        }
        let decode_fresh = self
            .decode_age_ms
            .is_some_and(|age| age <= thresholds.degraded_decode_age_ms);
        if !decode_fresh {
            return false;
        }
        let decode_fps = self.smoothed_decode_fps.unwrap_or(0.0);
        let present_fps = self.smoothed_present_fps.unwrap_or(0.0);
        let fps_gap_stressed = self.smoothed_decode_fps.is_some()
            && self.smoothed_present_fps.is_some()
            && decode_fps >= PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS
            && present_fps > 0.0
            && present_fps <= PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS
            && (decode_fps - present_fps) >= PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP;
        let present_lagging = self.smoothed_decode_fps.is_some()
            && self.smoothed_present_fps.is_some()
            && self.present_age_ms.zip(self.decode_age_ms).is_some_and(
                |(present_age, decode_age)| {
                    present_age >= thresholds.degraded_present_age_ms
                        && present_age > decode_age * PRESENT_PIPELINE_PRESENT_AGE_OVER_DECODE_RATIO
                },
            );
        fps_gap_stressed || present_lagging
    }

    pub(crate) fn host_is_priming_without_present(&self) -> bool {
        matches!(self.host_cadence_phase.as_deref(), Some("priming"))
            && self.host_display_tick_epoch.unwrap_or_default() > 0
            && self.host_frame_present_epoch.unwrap_or_default() == 0
            && self.host_mailbox_enqueue_count_total.unwrap_or_default() == 0
    }

    pub(crate) fn critical_signal(
        &self,
        thresholds: &DisplaySupplyThresholds,
    ) -> DisplaySupplyCriticalSignal {
        if self.host_is_priming_without_present() {
            return DisplaySupplyCriticalSignal::None;
        }
        let no_pending_streak = self.no_pending_streak.unwrap_or_default();
        let pressure_critical =
            matches!(self.no_pending_pressure_level.as_deref(), Some("critical"));
        let present_age_critical = self
            .present_age_ms
            .is_some_and(|age| age >= thresholds.critical_present_age_ms);
        let decode_age_critical = self
            .decode_age_ms
            .is_some_and(|age| age >= thresholds.critical_decode_age_ms);
        let host_pressure_critical = (pressure_critical
            && no_pending_streak >= thresholds.critical_no_pending_streak)
            && (present_age_critical || decode_age_critical);
        if self.video_renderer_stalled && host_pressure_critical {
            return DisplaySupplyCriticalSignal::HardRendererStall;
        }
        let present_drop_ratio = ratio(
            self.host_mailbox_drop_count_total,
            self.host_mailbox_enqueue_count_total,
        );
        let present_overwrite_ratio = ratio(
            self.host_mailbox_overwrite_count_total,
            self.host_mailbox_enqueue_count_total,
        );
        let pacer_drop_ratio = ratio(self.pacer_drop_count_total, self.pacer_submit_count_total);
        let renderer_drop_ratio = ratio(
            self.renderer_drop_count_total,
            self.renderer_submit_count_total,
        );
        let critical_supply_drop = present_drop_ratio
            .is_some_and(|value| value >= thresholds.critical_present_drop_ratio)
            || present_overwrite_ratio
                .is_some_and(|value| value >= thresholds.critical_present_overwrite_ratio)
            || pacer_drop_ratio.is_some_and(|value| value >= thresholds.critical_pacer_drop_ratio)
            || renderer_drop_ratio
                .is_some_and(|value| value >= thresholds.critical_renderer_drop_ratio);
        if critical_supply_drop {
            return DisplaySupplyCriticalSignal::HardSupplyDrop;
        }
        if host_pressure_critical {
            return DisplaySupplyCriticalSignal::SoftNoPendingAge;
        }
        DisplaySupplyCriticalSignal::None
    }

    pub(crate) fn classify_display_supply_state(
        &self,
        thresholds: &DisplaySupplyThresholds,
    ) -> DisplaySupplyState {
        if self.host_is_priming_without_present() {
            return DisplaySupplyState::Healthy;
        }
        let no_pending_streak = self.no_pending_streak.unwrap_or_default();
        let pressure_high = matches!(
            self.no_pending_pressure_level.as_deref(),
            Some("high" | "critical")
        );
        let present_age_warning = self
            .present_age_ms
            .is_some_and(|age| age >= thresholds.degraded_present_age_ms);
        let decode_age_warning = self
            .decode_age_ms
            .is_some_and(|age| age >= thresholds.degraded_decode_age_ms);
        let present_drop_ratio = ratio(
            self.host_mailbox_drop_count_total,
            self.host_mailbox_enqueue_count_total,
        );
        let present_overwrite_ratio = ratio(
            self.host_mailbox_overwrite_count_total,
            self.host_mailbox_enqueue_count_total,
        );
        let pacer_drop_ratio = ratio(self.pacer_drop_count_total, self.pacer_submit_count_total);
        let renderer_drop_ratio = ratio(
            self.renderer_drop_count_total,
            self.renderer_submit_count_total,
        );
        let degraded_supply_drop = present_drop_ratio
            .is_some_and(|value| value >= thresholds.degraded_present_drop_ratio)
            || present_overwrite_ratio
                .is_some_and(|value| value >= thresholds.degraded_present_overwrite_ratio)
            || pacer_drop_ratio.is_some_and(|value| value >= thresholds.degraded_pacer_drop_ratio)
            || renderer_drop_ratio
                .is_some_and(|value| value >= thresholds.degraded_renderer_drop_ratio);

        if self.critical_signal(thresholds) != DisplaySupplyCriticalSignal::None {
            return DisplaySupplyState::Critical;
        }
        if (pressure_high && no_pending_streak >= thresholds.degraded_no_pending_streak)
            && (present_age_warning || decode_age_warning)
        {
            return DisplaySupplyState::Degraded;
        }
        if degraded_supply_drop {
            return DisplaySupplyState::Degraded;
        }
        DisplaySupplyState::Healthy
    }

    #[allow(dead_code)]
    pub(crate) fn escalation_reason_for_display_supply(
        &self,
        thresholds: &DisplaySupplyThresholds,
    ) -> Option<VideoEscalationReason> {
        match self.classify_display_supply_state(thresholds) {
            DisplaySupplyState::Healthy => None,
            DisplaySupplyState::Degraded => Some(VideoEscalationReason::AdapterThinStream),
            DisplaySupplyState::Critical => Some(VideoEscalationReason::DisplaySupplyCritical),
        }
    }
}

fn ratio(numerator: Option<u64>, denominator: Option<u64>) -> Option<f64> {
    let num = numerator?;
    let den = denominator?;
    if den == 0 {
        return None;
    }
    Some((num as f64 / den as f64).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::{DisplaySupplyCriticalSignal, DisplaySupplyState, SchedulingDemandSignal};
    use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;

    fn cloud_thresholds() -> DisplaySupplyThresholds {
        DisplaySupplyThresholds {
            degraded_no_pending_streak: 48,
            critical_no_pending_streak: 96,
            degraded_present_age_ms: 180.0,
            degraded_decode_age_ms: 140.0,
            critical_present_age_ms: 600.0,
            critical_decode_age_ms: 320.0,
            degraded_present_drop_ratio: 0.03,
            critical_present_drop_ratio: 0.08,
            degraded_present_overwrite_ratio: 0.05,
            critical_present_overwrite_ratio: 0.12,
            degraded_pacer_drop_ratio: 0.02,
            critical_pacer_drop_ratio: 0.06,
            degraded_renderer_drop_ratio: 0.015,
            critical_renderer_drop_ratio: 0.05,
        }
    }

    fn home_thresholds() -> DisplaySupplyThresholds {
        DisplaySupplyThresholds {
            degraded_no_pending_streak: 80,
            critical_no_pending_streak: 150,
            degraded_present_age_ms: 240.0,
            degraded_decode_age_ms: 180.0,
            critical_present_age_ms: 720.0,
            critical_decode_age_ms: 420.0,
            degraded_present_drop_ratio: 0.04,
            critical_present_drop_ratio: 0.10,
            degraded_present_overwrite_ratio: 0.06,
            critical_present_overwrite_ratio: 0.14,
            degraded_pacer_drop_ratio: 0.03,
            critical_pacer_drop_ratio: 0.08,
            degraded_renderer_drop_ratio: 0.02,
            critical_renderer_drop_ratio: 0.06,
        }
    }

    #[test]
    fn present_pipeline_stressed_when_decode_fps_outruns_present() {
        let demand = SchedulingDemandSignal {
            present_age_ms: Some(40.0),
            decode_age_ms: Some(20.0),
            smoothed_present_fps: Some(10.0),
            smoothed_decode_fps: Some(31.0),
            ..SchedulingDemandSignal::default()
        };
        assert!(demand.present_pipeline_stressed(&cloud_thresholds()));
    }

    #[test]
    fn high_no_pending_without_age_pressure_is_not_forced() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("high".to_string()),
            no_pending_streak: Some(66),
            present_age_ms: Some(24.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.classify_display_supply_state(&cloud_thresholds()),
            DisplaySupplyState::Healthy
        );
    }

    #[test]
    fn stale_present_and_renderer_stalled_is_critical() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(120),
            present_age_ms: Some(1200.0),
            decode_age_ms: Some(420.0),
            video_renderer_stalled: true,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.classify_display_supply_state(&cloud_thresholds()),
            DisplaySupplyState::Critical
        );
    }

    #[test]
    fn same_signal_classifies_differently_between_cloud_and_home_thresholds() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(100),
            present_age_ms: Some(630.0),
            decode_age_ms: Some(340.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };

        assert_eq!(
            demand.classify_display_supply_state(&cloud_thresholds()),
            DisplaySupplyState::Critical
        );
        assert_eq!(
            demand.classify_display_supply_state(&home_thresholds()),
            DisplaySupplyState::Degraded
        );
    }

    #[test]
    fn heavy_present_overwrite_is_classified_as_critical_supply_even_if_age_is_fresh() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(4),
            present_age_ms: Some(18.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1000),
            host_mailbox_drop_count_total: Some(6),
            host_mailbox_overwrite_count_total: Some(190),
            pacer_submit_count_total: Some(1000),
            pacer_drop_count_total: Some(3),
            renderer_submit_count_total: Some(1000),
            renderer_drop_count_total: Some(1),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.classify_display_supply_state(&cloud_thresholds()),
            DisplaySupplyState::Critical
        );
        assert_eq!(
            demand.critical_signal(&cloud_thresholds()),
            DisplaySupplyCriticalSignal::HardSupplyDrop
        );
    }

    #[test]
    fn no_pending_and_aged_present_is_soft_critical_signal() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(120),
            present_age_ms: Some(980.0),
            decode_age_ms: Some(360.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.critical_signal(&cloud_thresholds()),
            DisplaySupplyCriticalSignal::SoftNoPendingAge
        );
    }

    #[test]
    fn renderer_stall_is_hard_critical_signal() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(96),
            present_age_ms: Some(620.0),
            decode_age_ms: Some(360.0),
            video_renderer_stalled: true,
            host_display_tick_epoch: None,
            host_frame_present_epoch: None,
            host_cadence_phase: None,
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.critical_signal(&cloud_thresholds()),
            DisplaySupplyCriticalSignal::HardRendererStall
        );
    }

    #[test]
    fn renderer_stall_without_host_pressure_stays_shadow_signal() {
        let demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(2),
            present_age_ms: Some(42.0),
            decode_age_ms: Some(32.0),
            video_renderer_stalled: true,
            host_display_tick_epoch: Some(12),
            host_frame_present_epoch: Some(12),
            host_cadence_phase: Some("steady".to_string()),
            host_mailbox_enqueue_count_total: Some(1200),
            host_mailbox_drop_count_total: Some(1),
            host_mailbox_overwrite_count_total: Some(2),
            pacer_submit_count_total: Some(1200),
            pacer_drop_count_total: Some(0),
            renderer_submit_count_total: Some(1200),
            renderer_drop_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        };
        assert_eq!(
            demand.critical_signal(&cloud_thresholds()),
            DisplaySupplyCriticalSignal::None
        );
        assert_eq!(
            demand.classify_display_supply_state(&cloud_thresholds()),
            DisplaySupplyState::Healthy
        );
    }
}
