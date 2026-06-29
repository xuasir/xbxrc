use std::collections::VecDeque;

use super::{MAX_BUFFERED_AUDIO_FRAMES, OPUS_OUTPUT_CHANNELS, OPUS_SAMPLE_RATE_HZ};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct AudioPlaybackOutputMetrics {
    pub(super) playout_latency_ms: f64,
}

#[derive(Default)]
pub(super) struct AudioPlaybackSharedState {
    pub(super) frames: VecDeque<[f32; OPUS_OUTPUT_CHANNELS]>,
    pub(super) source_cursor_frames: f64,
}

impl AudioPlaybackSharedState {
    pub(super) fn enqueue_interleaved_stereo(&mut self, samples: &[f32]) {
        for chunk in samples.chunks(OPUS_OUTPUT_CHANNELS) {
            let left = chunk.first().copied().unwrap_or(0.0);
            let right = chunk.get(1).copied().unwrap_or(left);
            self.frames.push_back([left, right]);
        }
        self.trim_overflow();
    }

    pub(super) fn fill_output_f32(
        &mut self,
        output: &mut [f32],
        output_sample_rate_hz: u32,
        output_channels: usize,
        volume: f32,
    ) -> AudioPlaybackOutputMetrics {
        let metrics = self.current_output_metrics();
        output.fill(0.0);
        if output_channels == 0 {
            return metrics;
        }

        // 先用最近邻重采样保证链路可播，后续再按需要替换更高质量实现。
        let output_frame_count = output.len() / output_channels;
        let source_step = OPUS_SAMPLE_RATE_HZ as f64 / output_sample_rate_hz.max(1) as f64;
        let gain = volume.max(0.0);

        for frame_index in 0..output_frame_count {
            let source_index = self.source_cursor_frames.floor() as usize;
            let [left, right] = self.frames.get(source_index).copied().unwrap_or([0.0, 0.0]);
            write_output_frame(
                &mut output[frame_index * output_channels..(frame_index + 1) * output_channels],
                left * gain,
                right * gain,
            );
            self.source_cursor_frames += source_step;
        }

        self.discard_consumed_frames();
        metrics
    }

    pub(super) fn current_output_metrics(&self) -> AudioPlaybackOutputMetrics {
        AudioPlaybackOutputMetrics {
            playout_latency_ms: self.playout_buffer_frames() * 1_000.0 / OPUS_SAMPLE_RATE_HZ as f64,
        }
    }

    pub(super) fn playout_buffer_frames(&self) -> f64 {
        (self.frames.len() as f64 - self.source_cursor_frames).max(0.0)
    }

    fn trim_overflow(&mut self) {
        let overflow_frames = self.frames.len().saturating_sub(MAX_BUFFERED_AUDIO_FRAMES);
        for _ in 0..overflow_frames {
            self.frames.pop_front();
        }
        if overflow_frames > 0 {
            self.source_cursor_frames =
                (self.source_cursor_frames - overflow_frames as f64).max(0.0);
        }
        if self.frames.is_empty() {
            self.source_cursor_frames = 0.0;
        } else {
            self.source_cursor_frames = self
                .source_cursor_frames
                .min((self.frames.len().saturating_sub(1)) as f64);
        }
    }

    fn discard_consumed_frames(&mut self) {
        let consumed_frames = self.source_cursor_frames.floor() as usize;
        if consumed_frames == 0 {
            return;
        }

        if consumed_frames >= self.frames.len() {
            self.frames.clear();
            self.source_cursor_frames = 0.0;
            return;
        }

        for _ in 0..consumed_frames {
            self.frames.pop_front();
        }
        self.source_cursor_frames -= consumed_frames as f64;
    }
}

fn write_output_frame(frame: &mut [f32], left: f32, right: f32) {
    match frame.len() {
        0 => {}
        1 => {
            frame[0] = (left + right) * 0.5;
        }
        _ => {
            frame[0] = left;
            frame[1] = right;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AudioPlaybackSharedState;
    use crate::transport::rtc::stream::audio::{
        MAX_BUFFERED_AUDIO_FRAMES, MAX_BUFFERED_AUDIO_LATENCY_MS, OPUS_SAMPLE_RATE_HZ,
    };

    #[test]
    fn playback_buffer_trims_overflow_and_keeps_cursor_bounded() {
        let mut state = AudioPlaybackSharedState::default();
        let mut samples = Vec::new();
        for _ in 0..(OPUS_SAMPLE_RATE_HZ as usize + 16) {
            samples.extend_from_slice(&[0.25, -0.25]);
        }

        state.enqueue_interleaved_stereo(&samples);

        assert_eq!(state.frames.len(), MAX_BUFFERED_AUDIO_FRAMES);
        assert!(state.source_cursor_frames <= (state.frames.len().saturating_sub(1)) as f64);
    }

    #[test]
    fn playback_buffer_caps_latency_for_low_latency_streaming() {
        let mut state = AudioPlaybackSharedState::default();
        let mut samples = Vec::new();
        for _ in 0..OPUS_SAMPLE_RATE_HZ as usize {
            samples.extend_from_slice(&[0.25, -0.25]);
        }

        state.enqueue_interleaved_stereo(&samples);

        assert!(
            state.current_output_metrics().playout_latency_ms <= {
                MAX_BUFFERED_AUDIO_LATENCY_MS as f64
            }
        );
    }

    #[test]
    fn playback_buffer_resamples_and_consumes_source_frames() {
        let mut state = AudioPlaybackSharedState::default();
        state.enqueue_interleaved_stereo(&[
            0.1, 0.2, //
            0.3, 0.4, //
            0.5, 0.6, //
            0.7, 0.8,
        ]);
        let mut output = vec![0.0; 4];

        state.fill_output_f32(&mut output, 24_000, 2, 1.0);

        assert_eq!(output, vec![0.1, 0.2, 0.5, 0.6]);
        assert!(state.frames.is_empty());
        assert_eq!(state.source_cursor_frames, 0.0);
    }

    #[test]
    fn playback_buffer_metrics_reflect_remaining_frames() {
        let mut state = AudioPlaybackSharedState::default();
        state.enqueue_interleaved_stereo(&[
            0.1, 0.2, //
            0.3, 0.4, //
            0.5, 0.6, //
            0.7, 0.8,
        ]);
        state.source_cursor_frames = 1.5;

        let metrics = state.current_output_metrics();

        assert_eq!(state.playout_buffer_frames(), 2.5);
        assert_eq!(
            metrics.playout_latency_ms,
            2.5 * 1_000.0 / OPUS_SAMPLE_RATE_HZ as f64
        );
    }
}
