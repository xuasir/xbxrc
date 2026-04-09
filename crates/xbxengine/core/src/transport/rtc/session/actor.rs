use std::collections::VecDeque;

use crate::transport::rtc::facts::{SessionCommand, TransportFact};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};

use super::{clock::SessionClock, mailbox::SessionMailbox};

/// policy hook 先保留最小接口，当前默认 no-op。
pub trait SessionPolicyHook {
    fn on_snapshot(&mut self, _snapshot: &TransportSnapshot) -> Vec<SessionCommand> {
        Vec::new()
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct NoopSessionPolicy;

#[cfg(test)]
impl SessionPolicyHook for NoopSessionPolicy {}

pub struct SessionActor<C, P>
where
    C: SessionClock,
    P: SessionPolicyHook,
{
    mailbox: SessionMailbox,
    clock: C,
    policy: P,
    snapshot_version: u64,
    connection: ConnectionProjection,
    media: MediaProjection,
    recovery: RecoveryProjection,
    bwe: BweProjection,
    diagnostics: DiagnosticsProjection,
    pending_commands: VecDeque<SessionCommand>,
}

impl<C, P> SessionActor<C, P>
where
    C: SessionClock,
    P: SessionPolicyHook,
{
    pub fn new(clock: C, policy: P) -> Self {
        Self {
            mailbox: SessionMailbox::new(),
            clock,
            policy,
            snapshot_version: 0,
            connection: ConnectionProjection::default(),
            media: MediaProjection::default(),
            recovery: RecoveryProjection::default(),
            bwe: BweProjection::default(),
            diagnostics: DiagnosticsProjection::default(),
            pending_commands: VecDeque::new(),
        }
    }

    pub fn enqueue_fact(&mut self, fact: TransportFact) {
        self.mailbox.push_fact(fact);
    }

    /// 执行最小串行推进，后续可替换为 async actor runtime。
    pub fn drain_once(&mut self, max_steps: usize) -> usize {
        let mut processed = 0usize;
        while processed < max_steps {
            let Some(fact) = self.mailbox.pop_fact() else {
                break;
            };
            self.apply_fact(&fact);
            processed = processed.saturating_add(1);
        }
        processed
    }

    pub fn pop_next_command(&mut self) -> Option<SessionCommand> {
        self.pending_commands.pop_front()
    }

    pub fn snapshot(&self) -> TransportSnapshot {
        TransportSnapshot::new(
            self.snapshot_version,
            self.clock.now_ms(),
            self.connection.clone(),
            self.media.clone(),
            self.recovery.clone(),
            self.bwe.clone(),
            self.diagnostics.clone(),
        )
    }

    fn apply_fact(&mut self, fact: &TransportFact) {
        self.connection.apply_fact(fact);
        self.media.apply_fact(fact);
        self.recovery.apply_fact(fact);
        self.bwe.apply_fact(fact);
        self.diagnostics.apply_fact(fact);

        self.snapshot_version = self.snapshot_version.saturating_add(1);
        let snapshot = self.snapshot();
        for command in self.policy.on_snapshot(&snapshot) {
            self.pending_commands.push_back(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NoopSessionPolicy, SessionActor};
    use crate::transport::rtc::facts::{MediaFact, TransportFact};
    use crate::transport::rtc::session::clock::SystemSessionClock;

    #[test]
    fn session_actor_applies_media_fact_to_snapshot() {
        let mut actor = SessionActor::new(SystemSessionClock, NoopSessionPolicy);
        actor.enqueue_fact(TransportFact::Media(MediaFact::FrameArrived {
            rtp_timestamp: 123,
            width: 1920,
            height: 1080,
            is_keyframe: true,
            observed_at_ms: 1.0,
        }));
        assert_eq!(actor.drain_once(8), 1);
        let snapshot = actor.snapshot();
        assert_eq!(snapshot.media.frame_count, 1);
        assert_eq!(snapshot.media.latest_frame_resolution, Some((1920, 1080)));
    }
}
