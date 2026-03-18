use crate::mods::data::types::DataXcloudCatalogPayload;
use std::collections::HashMap;
use tokio::sync::oneshot;

struct XcloudCatalogRefreshFlight {
    waiters: Vec<oneshot::Sender<Result<DataXcloudCatalogPayload, String>>>,
}

pub enum XcloudCatalogRefreshJoin {
    Leader(oneshot::Receiver<Result<DataXcloudCatalogPayload, String>>),
    Follower(oneshot::Receiver<Result<DataXcloudCatalogPayload, String>>),
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

    pub fn begin_xcloud_refresh(&self, cache_key: &str) -> XcloudCatalogRefreshJoin {
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
        XcloudCatalogRefreshJoin::Leader(rx)
    }

    pub fn finish_xcloud_refresh(
        &self,
        cache_key: &str,
        result: Result<DataXcloudCatalogPayload, String>,
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
