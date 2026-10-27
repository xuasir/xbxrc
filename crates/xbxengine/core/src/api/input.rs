use crate::api::runtime::XbxEngineRuntimeError;
use ohmygamepad_protocol::{LogicalPadSnapshotDto, LogicalPadStateDto};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XbxEngineInputStatus {
    pub device_count: usize,
    pub pad_count: usize,
    pub stream_input_active: bool,
}

pub trait XbxEngineInputBackend: Send {
    fn attach_session(
        &mut self,
        session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn press_controller_button(
        &mut self,
        button: &str,
        duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

#[derive(Default)]
pub struct NoopXbxEngineInputBackend;

impl XbxEngineInputBackend for NoopXbxEngineInputBackend {
    fn attach_session(
        &mut self,
        _session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn press_controller_button(
        &mut self,
        _button: &str,
        _duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

#[derive(Default)]
struct ApplicationInputState {
    session: Option<String>,
    producer_sequence: u64,
    sample_sequence: u64,
    state: LogicalPadStateDto,
    pending: VecDeque<LogicalPadSnapshotDto>,
}

impl ApplicationInputState {
    fn submit(&mut self, session_id: &str, sequence: u64, state: LogicalPadStateDto) -> bool {
        if self.session.as_deref() != Some(session_id) || sequence <= self.producer_sequence {
            return false;
        }
        self.producer_sequence = sequence;
        self.enqueue(state);
        true
    }

    fn enqueue(&mut self, state: LogicalPadStateDto) {
        if self.state == state {
            return;
        }
        self.state = state.clone();
        // 有界积压：过载时先释放，再发送最新状态，避免重放过时按键。
        if self.pending.len() >= 32 {
            self.pending.clear();
            self.push_frame(LogicalPadStateDto::default());
        }
        self.push_frame(state);
    }

    fn push_frame(&mut self, state: LogicalPadStateDto) {
        self.sample_sequence = self.sample_sequence.wrapping_add(1);
        self.pending.push_back(LogicalPadSnapshotDto {
            state,
            sample_seq: self.sample_sequence,
            ..Default::default()
        });
    }

    fn acknowledge(&mut self, sequence: u64) {
        if self
            .pending
            .front()
            .is_some_and(|frame| frame.sample_seq == sequence)
        {
            self.pending.pop_front();
        }
    }
}

fn application_input() -> &'static Mutex<ApplicationInputState> {
    static INPUT: OnceLock<Mutex<ApplicationInputState>> = OnceLock::new();
    INPUT.get_or_init(Default::default)
}

/// 接收公共控制器已经完成路由、映射与释放的状态；会话与序号拒绝迟到写入。
pub fn submit_application_gamepad_state(
    session_id: &str,
    sequence: u64,
    state: LogicalPadStateDto,
) -> bool {
    let Ok(mut input) = application_input().lock() else {
        return false;
    };
    input.submit(session_id, sequence, state)
}

pub(crate) fn application_gamepad_frame() -> Option<LogicalPadSnapshotDto> {
    let input = application_input().lock().ok()?;
    input.session.as_ref()?;
    Some(
        input
            .pending
            .front()
            .cloned()
            .unwrap_or_else(|| LogicalPadSnapshotDto {
                state: input.state.clone(),
                sample_seq: input.sample_sequence,
                ..Default::default()
            }),
    )
}

/// 仅在传输成功后推进；失败时下一 tick 重发同一帧。
pub(crate) fn acknowledge_application_gamepad_frame(sequence: u64) {
    if let Ok(mut input) = application_input().lock() {
        input.acknowledge(sequence);
    }
}

/// Rust 媒体消费应用输入端口，设备后端由桌面装配层独立选择。
#[derive(Default)]
pub struct ApplicationXbxEngineInputBackend {
    session: Option<String>,
}

impl ApplicationXbxEngineInputBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl XbxEngineInputBackend for ApplicationXbxEngineInputBackend {
    fn attach_session(
        &mut self,
        session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        let mut input = application_input()
            .lock()
            .map_err(|e| XbxEngineRuntimeError::new(e.to_string()))?;
        input.session = Some(session_id.to_owned());
        input.producer_sequence = 0;
        input.sample_sequence = input.sample_sequence.wrapping_add(1);
        input.state = LogicalPadStateDto::default();
        input.pending.clear();
        self.session = Some(session_id.to_owned());
        Ok(XbxEngineInputStatus {
            device_count: 0,
            pad_count: 1,
            stream_input_active: true,
        })
    }

    fn press_controller_button(
        &mut self,
        _button: &str,
        _duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Err(XbxEngineRuntimeError::new("useApplicationInputController"))
    }

    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        let input = application_input()
            .lock()
            .map_err(|e| XbxEngineRuntimeError::new(e.to_string()))?;
        let active = self.session.is_some() && input.session == self.session;
        Ok(XbxEngineInputStatus {
            device_count: 0,
            pad_count: usize::from(active),
            stream_input_active: active,
        })
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let mut input = application_input()
            .lock()
            .map_err(|e| XbxEngineRuntimeError::new(e.to_string()))?;
        if input.session == self.session {
            input.session = None;
            input.state = LogicalPadStateDto::default();
            input.pending.clear();
            input.sample_sequence = input.sample_sequence.wrapping_add(1);
        }
        self.session = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_session_and_sequence_cannot_replace_current_input() {
        let mut input = ApplicationInputState {
            session: Some("current".into()),
            ..Default::default()
        };
        let mut pressed = LogicalPadStateDto::default();
        pressed.buttons.south = 1.0;
        assert!(input.submit("current", 2, pressed.clone()));
        assert!(!input.submit("old", 3, LogicalPadStateDto::default()));
        assert!(!input.submit("current", 1, LogicalPadStateDto::default()));
        assert!(!input.submit("current", 2, LogicalPadStateDto::default()));
        assert_eq!(input.state, pressed);
        assert!(input.submit("current", 3, LogicalPadStateDto::default()));
    }

    #[test]
    fn release_is_retained_until_acknowledged_before_new_press() {
        let mut input = ApplicationInputState::default();
        let mut pressed = LogicalPadStateDto::default();
        pressed.buttons.south = 1.0;
        input.enqueue(pressed.clone());
        input.acknowledge(input.pending.front().unwrap().sample_seq);
        input.enqueue(LogicalPadStateDto::default());
        input.enqueue(pressed.clone());
        let release = input.pending.front().unwrap().clone();
        assert_eq!(release.state, LogicalPadStateDto::default());
        input.acknowledge(release.sample_seq.wrapping_sub(1));
        assert_eq!(
            input.pending.front().unwrap().sample_seq,
            release.sample_seq
        );
        input.acknowledge(release.sample_seq);
        assert_eq!(input.pending.front().unwrap().state, pressed);
    }

    #[test]
    fn overloaded_queue_is_bounded_and_releases_before_latest() {
        let mut input = ApplicationInputState::default();
        for index in 0..33 {
            let mut state = LogicalPadStateDto::default();
            state.left_stick.x = (index + 1) as f32 / 100.0;
            input.enqueue(state);
        }
        assert_eq!(input.pending.len(), 2);
        assert_eq!(
            input.pending.front().unwrap().state,
            LogicalPadStateDto::default()
        );
        assert_eq!(input.pending.back().unwrap().state.left_stick.x, 0.33);
    }
}
