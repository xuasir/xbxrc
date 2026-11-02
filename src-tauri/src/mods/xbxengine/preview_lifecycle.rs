use std::sync::Arc;
use tokio::sync::watch;

pub(super) type StopResult = Option<Result<(), String>>;
pub(super) enum PreviewStop<T> {
    Started {
        runtime: Arc<T>,
        completion: watch::Sender<StopResult>,
    },
    Waiting(watch::Receiver<StopResult>),
    Idle,
}

/// 单一预览装配槽；所有停止调用共享排空结果，完成回收后才允许重启。
pub(super) struct PreviewRuntimeSlot<T> {
    current: Option<Arc<T>>,
    stopping: Option<watch::Receiver<StopResult>>,
}

impl<T> Default for PreviewRuntimeSlot<T> {
    fn default() -> Self {
        Self {
            current: None,
            stopping: None,
        }
    }
}

impl<T> PreviewRuntimeSlot<T> {
    pub fn start(&mut self, create: impl FnOnce() -> T) -> Result<Arc<T>, &'static str> {
        if self.stopping.is_some() {
            return Err("nativeRuntimeStopping");
        }
        if self.current.is_some() {
            return Err("nativeRuntimeAlreadyActive");
        }
        let runtime = Arc::new(create());
        self.current = Some(runtime.clone());
        Ok(runtime)
    }
    pub fn current(&self) -> Option<Arc<T>> {
        self.current.clone()
    }
    pub fn begin_stop(&mut self) -> PreviewStop<T> {
        if let Some(completion) = &self.stopping {
            return PreviewStop::Waiting(completion.clone());
        }
        let Some(runtime) = self.current.take() else {
            return PreviewStop::Idle;
        };
        let (completion, receiver) = watch::channel(None);
        self.stopping = Some(receiver);
        PreviewStop::Started {
            runtime,
            completion,
        }
    }
    pub fn finish_stop(&mut self) {
        self.stopping = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn creates_only_on_start_and_blocks_restart_until_drained() {
        let mut slot = PreviewRuntimeSlot::default();
        let created = AtomicUsize::new(0);
        assert!(slot.current().is_none());
        assert!(matches!(slot.begin_stop(), PreviewStop::Idle));
        let current = slot
            .start(|| {
                created.fetch_add(1, Ordering::SeqCst);
                1
            })
            .unwrap();
        assert_eq!(
            slot.start(|| {
                created.fetch_add(1, Ordering::SeqCst);
                2
            })
            .unwrap_err(),
            "nativeRuntimeAlreadyActive"
        );
        assert_eq!(created.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&current, &slot.current().unwrap()));
        let PreviewStop::Started {
            runtime: stopping, ..
        } = slot.begin_stop()
        else {
            panic!("stop owner");
        };
        assert!(slot.current().is_none());
        assert_eq!(slot.start(|| 2).unwrap_err(), "nativeRuntimeStopping");
        slot.finish_stop();
        let next = slot.start(|| 2).unwrap();
        assert!(!Arc::ptr_eq(&next, &stopping));
    }

    #[tokio::test]
    async fn concurrent_stop_waiters_share_completion_and_failure() {
        let mut slot = PreviewRuntimeSlot::default();
        slot.start(|| 1).unwrap();
        let PreviewStop::Started { completion, .. } = slot.begin_stop() else {
            panic!("stop owner");
        };
        let PreviewStop::Waiting(mut first) = slot.begin_stop() else {
            panic!("first waiter");
        };
        let PreviewStop::Waiting(mut second) = slot.begin_stop() else {
            panic!("second waiter");
        };
        assert!(first.borrow().is_none());
        completion.send_replace(Some(Err("drainFailed".into())));
        first.changed().await.unwrap();
        second.changed().await.unwrap();
        assert_eq!(*first.borrow(), Some(Err("drainFailed".into())));
        assert_eq!(*first.borrow(), *second.borrow());
        slot.finish_stop();
        assert!(slot.start(|| 2).is_ok());
    }
}
