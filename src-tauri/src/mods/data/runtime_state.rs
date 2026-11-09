use crate::mods::data::types::DataXcloudCatalogPayload;
use crate::mods::data::DataError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

struct XcloudCatalogRefreshFlight {
    waiters: Vec<oneshot::Sender<Result<DataXcloudCatalogPayload, DataError>>>,
}

pub enum XcloudCatalogRefreshJoin {
    Leader(XcloudCatalogRefreshLease),
    Follower(oneshot::Receiver<Result<DataXcloudCatalogPayload, DataError>>),
}

pub struct XcloudCatalogRefreshLease {
    state: Arc<DataRuntimeState>,
    cache_key: String,
    finished: bool,
}

impl XcloudCatalogRefreshLease {
    pub fn finish(mut self, result: Result<DataXcloudCatalogPayload, DataError>) {
        self.finished = true;
        self.state.finish_xcloud_refresh(&self.cache_key, result);
    }
}

impl Drop for XcloudCatalogRefreshLease {
    fn drop(&mut self) {
        if !self.finished {
            self.state
                .finish_xcloud_refresh(&self.cache_key, Err("目录刷新已中断，请重试".into()));
        }
    }
}

pub struct DataRuntimeState {
    xcloud_refresh_flights: std::sync::Mutex<HashMap<String, XcloudCatalogRefreshFlight>>,
}

impl DataRuntimeState {
    pub fn new() -> Self {
        Self {
            xcloud_refresh_flights: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn begin_xcloud_refresh(self: &Arc<Self>, cache_key: &str) -> XcloudCatalogRefreshJoin {
        let (tx, rx) = oneshot::channel();
        let mut flights = self
            .xcloud_refresh_flights
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(flight) = flights.get_mut(cache_key) {
            flight.waiters.push(tx);
            return XcloudCatalogRefreshJoin::Follower(rx);
        }

        flights.insert(
            cache_key.to_string(),
            XcloudCatalogRefreshFlight { waiters: vec![tx] },
        );
        XcloudCatalogRefreshJoin::Leader(XcloudCatalogRefreshLease {
            state: self.clone(),
            cache_key: cache_key.to_string(),
            finished: false,
        })
    }

    pub fn finish_xcloud_refresh(
        &self,
        cache_key: &str,
        result: Result<DataXcloudCatalogPayload, DataError>,
    ) {
        let waiters = self
            .xcloud_refresh_flights
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(cache_key)
            .map(|flight| flight.waiters)
            .unwrap_or_default();

        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    pub fn is_xcloud_refreshing(&self, cache_key: &str) -> bool {
        self.xcloud_refresh_flights
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(cache_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_catalog_leader_releases_waiters_and_allows_retry() {
        let state = Arc::new(DataRuntimeState::new());
        let XcloudCatalogRefreshJoin::Leader(lease) = state.begin_xcloud_refresh("scope") else {
            panic!("leader");
        };
        let XcloudCatalogRefreshJoin::Follower(receiver) = state.begin_xcloud_refresh("scope")
        else {
            panic!("follower");
        };
        assert!(state.is_xcloud_refreshing("scope"));
        drop(lease);
        assert!(receiver.await.unwrap().is_err());
        assert!(!state.is_xcloud_refreshing("scope"));
        assert!(matches!(
            state.begin_xcloud_refresh("scope"),
            XcloudCatalogRefreshJoin::Leader(_)
        ));
    }

    #[tokio::test]
    async fn catalog_refresh_waiters_keep_the_authentication_failure_category() {
        let state = Arc::new(DataRuntimeState::new());
        let XcloudCatalogRefreshJoin::Leader(lease) = state.begin_xcloud_refresh("scope") else {
            panic!("leader");
        };
        let XcloudCatalogRefreshJoin::Follower(receiver) = state.begin_xcloud_refresh("scope")
        else {
            panic!("follower");
        };
        let error = DataError::from(xbox_webapi::WebApiError::auth("登录续期失效"));
        let expected = error.safe_summary();
        lease.finish(Err(error));
        assert_eq!(
            receiver.await.unwrap().unwrap_err().safe_summary(),
            expected
        );
        assert!(!state.is_xcloud_refreshing("scope"));
    }
}
