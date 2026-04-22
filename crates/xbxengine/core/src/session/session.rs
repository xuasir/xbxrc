use super::{commands::SessionCommand, events::SessionEvent, state::SessionState};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionControlFlags {
    pub request_keyframe: bool,
    pub flush_video_pipeline: bool,
    pub reconfigure_video: bool,
    pub restart_transport: bool,
}

pub struct Session {
    state: SessionState,
    control_flags: SessionControlFlags,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: SessionState::Negotiating,
            control_flags: SessionControlFlags::default(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.state.clone()
    }

    /**
     * 统一回收控制动作，避免 transport/media 各自持有恢复意图。
     */
    pub fn take_control_flags(&mut self) -> SessionControlFlags {
        std::mem::take(&mut self.control_flags)
    }

    /**
     * 事件驱动状态流转：
     * packet/frame/decode/render 相关事件都先汇聚到 session，再决定是否触发恢复动作。
     */
    pub fn handle_event(&mut self, ev: SessionEvent) {
        match ev {
            SessionEvent::PacketLossBurst => {
                self.state = SessionState::Recovering;
                self.control_flags.request_keyframe = true;
            }
            SessionEvent::FrameReady => {
                self.state = SessionState::Running;
            }
            SessionEvent::DecodeError => {
                self.state = SessionState::Recovering;
                self.control_flags.request_keyframe = true;
                self.control_flags.flush_video_pipeline = true;
            }
            SessionEvent::ResolutionChanged => {
                self.state = SessionState::Reconfiguring;
                self.control_flags.reconfigure_video = true;
                self.control_flags.flush_video_pipeline = true;
            }
            SessionEvent::KeyframeReceived => {
                if self.state == SessionState::Recovering {
                    self.state = SessionState::Primed;
                }
            }
        }
    }

    /**
     * Command 是外部显式控制入口，session 负责把命令映射为一致的状态/动作。
     */
    pub fn command(&mut self, cmd: SessionCommand) {
        match cmd {
            SessionCommand::RequestPli => {
                self.control_flags.request_keyframe = true;
                if self.state == SessionState::Stopped {
                    self.state = SessionState::Recovering;
                }
            }
            SessionCommand::FlushVideoPipeline => {
                self.control_flags.flush_video_pipeline = true;
                if matches!(self.state, SessionState::Running | SessionState::Primed) {
                    self.state = SessionState::Reconfiguring;
                }
            }
            SessionCommand::ReconfigureVideo => {
                self.control_flags.reconfigure_video = true;
                self.control_flags.flush_video_pipeline = true;
                self.state = SessionState::Reconfiguring;
            }
            SessionCommand::RestartTransport => {
                self.control_flags.restart_transport = true;
                self.state = SessionState::Negotiating;
            }
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_error_drives_recovery_and_flush_actions() {
        let mut session = Session::new();
        session.handle_event(SessionEvent::DecodeError);
        assert_eq!(session.state(), SessionState::Recovering);

        let flags = session.take_control_flags();
        assert!(flags.request_keyframe);
        assert!(flags.flush_video_pipeline);
        assert!(!flags.reconfigure_video);
        assert!(!flags.restart_transport);
    }

    #[test]
    fn reconfigure_command_moves_state_and_flags() {
        let mut session = Session::new();
        session.handle_event(SessionEvent::FrameReady);
        assert_eq!(session.state(), SessionState::Running);

        session.command(SessionCommand::ReconfigureVideo);
        assert_eq!(session.state(), SessionState::Reconfiguring);

        let flags = session.take_control_flags();
        assert!(flags.reconfigure_video);
        assert!(flags.flush_video_pipeline);
    }

    #[test]
    fn keyframe_received_primes_recovering_session() {
        let mut session = Session::new();
        session.handle_event(SessionEvent::PacketLossBurst);
        assert_eq!(session.state(), SessionState::Recovering);

        session.handle_event(SessionEvent::KeyframeReceived);
        assert_eq!(session.state(), SessionState::Primed);
    }
}
