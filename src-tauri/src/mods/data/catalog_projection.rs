use super::types::{
    DataCloudCatalogPatch, DataXcloudCatalogPayload, DataXcloudCatalogUpdatedEvent,
    DataXcloudTitleSummary,
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

pub(crate) fn content_revision(titles: &[DataXcloudTitleSummary]) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    titles.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

/// 单次刷新沿已有快照发布有界商品变更；终态与内容版本各自表达。
pub(crate) struct CatalogPublication {
    revision: String,
    titles: HashMap<String, DataXcloudTitleSummary>,
    order: Vec<String>,
}
impl CatalogPublication {
    pub(crate) fn new(titles: Vec<DataXcloudTitleSummary>) -> Self {
        Self {
            revision: content_revision(&titles),
            order: titles
                .iter()
                .map(|title| title.product_id.clone())
                .collect(),
            titles: titles
                .into_iter()
                .map(|title| (title.product_id.clone(), title))
                .collect(),
        }
    }

    pub(crate) fn publish(
        &mut self,
        payload: DataXcloudCatalogPayload,
        scope_key: &str,
        reason: &str,
    ) -> Vec<DataXcloudCatalogUpdatedEvent> {
        let revision = content_revision(&payload.titles);
        let order = payload
            .titles
            .iter()
            .map(|title| title.product_id.clone())
            .collect::<Vec<_>>();
        let ids = order.iter().collect::<HashSet<_>>();
        let removed = self
            .order
            .iter()
            .filter(|id| !ids.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        let changed = payload
            .titles
            .iter()
            .filter(|title| self.titles.get(&title.product_id) != Some(title))
            .cloned()
            .collect::<Vec<_>>();
        let count = changed.len().div_ceil(75).max(1);
        let mut events = Vec::with_capacity(count);
        for index in 0..count {
            let last = index + 1 == count;
            let next = if last {
                revision.clone()
            } else {
                format!("{revision}:{index}")
            };
            events.push(DataXcloudCatalogUpdatedEvent {
                scope_key: scope_key.into(),
                content_revision: next.clone(),
                fresh_until: payload.fresh_until,
                titles: Vec::new(),
                cache_state: payload.cache_state.clone(),
                updated_at: payload.updated_at,
                refreshing: payload.refreshing || !last,
                sources: payload.sources.clone(),
                access: payload.access.clone(),
                error: payload.error.clone(),
                reason: reason.into(),
                patch: Some(DataCloudCatalogPatch {
                    base_revision: self.revision.clone(),
                    upserts: changed.iter().skip(index * 75).take(75).cloned().collect(),
                    removed_product_ids: if index == 0 {
                        removed.clone()
                    } else {
                        Vec::new()
                    },
                    // 顺序在首次变更批次交付，后续商品到达后填入同一索引。
                    order: (index == 0 && order != self.order).then(|| order.clone()),
                }),
            });
            self.revision = next;
        }
        self.order = order;
        self.titles = payload
            .titles
            .into_iter()
            .map(|title| (title.product_id.clone(), title))
            .collect();
        events
    }
}

#[cfg(test)]
#[path = "catalog_projection.test.rs"]
mod tests;
