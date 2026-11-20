use super::supervision::{RuntimeObservationWindow, STABLE_WINDOW_MS};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// 平台提供累计事实，公共层构造具有连续覆盖的 3s/15s 窗口。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeMediaCounters {
    pub received: Option<u64>,
    pub decoded: Option<u64>,
    pub presented: Option<u64>,
    pub packets_received: Option<u64>,
    pub packets_lost: Option<i64>,
}

#[derive(Debug, Clone)]
struct Interval {
    start: u64,
    end: u64,
    received: Option<f64>,
    decoded: Option<f64>,
    presented: Option<f64>,
    loss: Option<f64>,
    rtt: Option<f64>,
    jitter: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RuntimeWindowAccumulator {
    previous: Option<(u64, RuntimeMediaCounters)>,
    intervals: VecDeque<Interval>,
}

impl RuntimeWindowAccumulator {
    pub(super) fn observe(
        &mut self,
        at: u64,
        counters: &RuntimeMediaCounters,
        rtt: Option<f64>,
        jitter: Option<f64>,
    ) {
        if let Some((last_at, last)) = &self.previous {
            if at <= *last_at {
                return;
            }
            let elapsed = at - last_at;
            let regressed = [
                (counters.received, last.received),
                (counters.decoded, last.decoded),
                (counters.presented, last.presented),
                (counters.packets_received, last.packets_received),
            ]
            .iter()
            .any(|(n, p)| n.zip(*p).is_some_and(|(n, p)| n < p));
            if elapsed > 1_500 || regressed {
                self.intervals.clear();
            } else {
                let rate = |n: Option<u64>, p: Option<u64>| {
                    n.zip(p)
                        .map(|(n, p)| (n - p) as f64 * 1_000.0 / elapsed as f64)
                };
                let lost = counters
                    .packets_lost
                    .zip(last.packets_lost)
                    .map(|(n, p)| (n - p).max(0) as u64);
                let received = counters
                    .packets_received
                    .zip(last.packets_received)
                    .map(|(n, p)| n - p);
                let loss = lost.zip(received).map(|(l, r)| {
                    if l + r > 0 {
                        l as f64 / (l + r) as f64
                    } else {
                        0.0
                    }
                });
                self.intervals.push_back(Interval {
                    start: *last_at,
                    end: at,
                    received: rate(counters.received, last.received),
                    decoded: rate(counters.decoded, last.decoded),
                    presented: rate(counters.presented, last.presented),
                    loss,
                    rtt,
                    jitter,
                });
            }
        }
        self.previous = Some((at, counters.clone()));
        while self
            .intervals
            .front()
            .is_some_and(|item| item.end <= at.saturating_sub(STABLE_WINDOW_MS))
        {
            self.intervals.pop_front();
        }
        while self.intervals.len() > 128 {
            self.intervals.pop_front();
        }
    }

    pub(super) fn window(&self, at: u64, duration: u64) -> RuntimeObservationWindow {
        let start = at.saturating_sub(duration);
        let metric = |read: fn(&Interval) -> Option<f64>| {
            let mut coverage = 0;
            let mut weighted = 0.0;
            for interval in &self.intervals {
                let span = interval.end.saturating_sub(interval.start.max(start));
                if let Some(value) = read(interval).filter(|n| n.is_finite()) {
                    coverage += span;
                    weighted += value * span as f64;
                }
            }
            (coverage, (coverage > 0).then(|| weighted / coverage as f64))
        };
        let (received_coverage_ms, inbound_fps_avg) = metric(|i| i.received);
        let (decode_coverage_ms, decode_fps_avg) = metric(|i| i.decoded);
        let (video_coverage_ms, video_present_fps_avg) = metric(|i| i.presented);
        let (loss_coverage_ms, loss_avg) = metric(|i| i.loss);
        let (rtt_coverage_ms, rtt_ms_avg) = metric(|i| i.rtt);
        let (jitter_coverage_ms, jitter_ms_avg) = metric(|i| i.jitter);
        RuntimeObservationWindow {
            received_coverage_ms,
            inbound_fps_avg,
            decode_coverage_ms,
            decode_fps_avg,
            video_coverage_ms,
            video_present_fps_avg,
            loss_coverage_ms,
            loss_avg,
            rtt_coverage_ms,
            rtt_ms_avg,
            jitter_coverage_ms,
            jitter_ms_avg,
            ..Default::default()
        }
    }
}
