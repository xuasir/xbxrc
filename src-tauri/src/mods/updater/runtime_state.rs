use std::sync::{Arc, Mutex};
use tauri::ResourceId;

#[derive(Default)]
struct Inner {
    generation: u64,
    pending: Option<ResourceId>,
    downloading: bool,
}

#[derive(Default)]
pub struct UpdaterRuntimeState {
    inner: Mutex<Inner>,
}

pub struct UpdateDownloadLease {
    state: Arc<UpdaterRuntimeState>,
    pub rid: ResourceId,
    generation: u64,
}

impl UpdaterRuntimeState {
    pub fn invalidate(&self) -> (u64, Option<ResourceId>) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.generation = inner.generation.wrapping_add(1);
        (inner.generation, inner.pending.take())
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
            == generation
    }

    pub fn publish(&self, generation: u64, rid: ResourceId) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if inner.generation != generation {
            return false;
        }
        inner.pending = Some(rid);
        true
    }

    pub fn begin_download(self: &Arc<Self>) -> Result<UpdateDownloadLease, String> {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if inner.downloading {
            return Err("更新下载正在处理中".into());
        }
        let rid = inner.pending.ok_or("请先检查更新，再下载更新包")?;
        inner.downloading = true;
        Ok(UpdateDownloadLease {
            state: self.clone(),
            rid,
            generation: inner.generation,
        })
    }
}

impl UpdateDownloadLease {
    pub fn install_if_current(
        &self,
        install: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let inner = self
            .state
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if inner.generation != self.generation || inner.pending != Some(self.rid) {
            return Err("更新渠道或检查结果已改变，请重新检查更新".into());
        }
        install()
    }

    pub fn complete(&self) {
        let mut inner = self
            .state
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if inner.generation == self.generation && inner.pending == Some(self.rid) {
            inner.pending = None;
        }
    }
}

impl Drop for UpdateDownloadLease {
    fn drop(&mut self) {
        // 失败和取消均保留当前 pending 资源，下一次下载可以重试。
        self.state
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .downloading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_or_cancelled_download_retains_candidate_for_retry() {
        let state = Arc::new(UpdaterRuntimeState::default());
        let (generation, _) = state.invalidate();
        assert!(state.publish(generation, 1));
        let first = state.begin_download().unwrap();
        assert!(state.begin_download().is_err());
        drop(first);
        let retry = state.begin_download().unwrap();
        assert_eq!(retry.rid, 1);
        retry.complete();
        drop(retry);
        assert!(state.begin_download().is_err());
    }

    #[test]
    fn stale_checks_and_changed_channel_downloads_cannot_install() {
        let state = Arc::new(UpdaterRuntimeState::default());
        let (old, _) = state.invalidate();
        let (new, _) = state.invalidate();
        assert!(!state.publish(old, 1));
        assert!(state.publish(new, 2));
        let lease = state.begin_download().unwrap();
        let (next, discarded) = state.invalidate();
        assert_eq!(discarded, Some(2));
        assert!(state.publish(next, 3));
        let installed = std::cell::Cell::new(false);
        assert!(lease
            .install_if_current(|| {
                installed.set(true);
                Ok(())
            })
            .is_err());
        assert!(!installed.get());
        drop(lease);
        assert_eq!(state.begin_download().unwrap().rid, 3);
    }
}
