use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::policy::TraceImportance;

pub(super) const QUEUE_MAX_ROWS: usize = 8_192;
pub(super) const QUEUE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_ROW_BYTES: usize = 64 * 1024;
pub(super) const CONTROL_CAPACITY: usize = 16;
const SEQUENCE_OVERHEAD: usize = 32;

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct QueueUsage {
    pub rows: usize,
    pub bytes: usize,
    pub peak_rows: usize,
    pub peak_bytes: usize,
}

#[derive(Default)]
pub(super) struct QueueBudget {
    usage: Mutex<QueueUsage>,
}

impl QueueBudget {
    fn limits(importance: TraceImportance) -> (usize, usize) {
        match importance {
            TraceImportance::Essential => (QUEUE_MAX_ROWS, QUEUE_MAX_BYTES),
            TraceImportance::Key => (QUEUE_MAX_ROWS - 256, QUEUE_MAX_BYTES - 256 * 1024),
            TraceImportance::Debug | TraceImportance::Raw => (6_144, 6 * 1024 * 1024),
        }
    }

    pub fn saturated(&self, importance: TraceImportance) -> bool {
        let (rows, bytes) = Self::limits(importance);
        self.usage
            .lock()
            .map(|usage| usage.rows >= rows || usage.bytes >= bytes)
            .unwrap_or(true)
    }

    /// 行数和字节一次保留，覆盖队列与writer正在处理的报文。
    pub fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        importance: TraceImportance,
    ) -> Option<RowReservation> {
        let (row_limit, byte_limit) = Self::limits(importance);
        let mut usage = self.usage.lock().ok()?;
        if bytes > MAX_ROW_BYTES
            || usage.rows >= row_limit
            || bytes > byte_limit.saturating_sub(usage.bytes)
        {
            return None;
        }
        usage.rows += 1;
        usage.bytes += bytes;
        usage.peak_rows = usage.peak_rows.max(usage.rows);
        usage.peak_bytes = usage.peak_bytes.max(usage.bytes);
        Some(RowReservation {
            budget: self.clone(),
            bytes,
        })
    }

    pub fn snapshot(&self) -> QueueUsage {
        self.usage.lock().map(|usage| *usage).unwrap_or_default()
    }
}

pub(super) struct RowReservation {
    budget: Arc<QueueBudget>,
    bytes: usize,
}

impl Drop for RowReservation {
    fn drop(&mut self) {
        if let Ok(mut usage) = self.budget.usage.lock() {
            usage.rows -= 1;
            usage.bytes -= self.bytes;
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DropReason {
    QueuePressure,
    Oversized,
    Configuration,
    Io,
}

#[derive(Default)]
pub(super) struct DropCounters {
    snapshot: Mutex<DropSnapshot>,
}

#[derive(Default)]
pub(super) struct DropSnapshot {
    pub essential: u64,
    pub key: u64,
    pub debug: u64,
    pub raw: u64,
    pub oversized: u64,
    pub configuration: u64,
    pub io: u64,
}

impl DropSnapshot {
    pub fn total(&self) -> u64 {
        self.essential + self.key + self.debug + self.raw
    }
}

impl DropCounters {
    pub fn note(&self, importance: TraceImportance, reason: DropReason) {
        let Ok(mut snapshot) = self.snapshot.lock() else {
            return;
        };
        // 等级与原因在同一事务累计，writer提取时保留完整丢弃证据。
        let counter = match importance {
            TraceImportance::Essential => &mut snapshot.essential,
            TraceImportance::Key => &mut snapshot.key,
            TraceImportance::Debug => &mut snapshot.debug,
            TraceImportance::Raw => &mut snapshot.raw,
        };
        *counter += 1;
        let reason_counter = match reason {
            DropReason::QueuePressure => return,
            DropReason::Oversized => &mut snapshot.oversized,
            DropReason::Configuration => &mut snapshot.configuration,
            DropReason::Io => &mut snapshot.io,
        };
        *reason_counter += 1;
    }

    pub fn take(&self) -> DropSnapshot {
        self.snapshot
            .lock()
            .map(|mut snapshot| std::mem::take(&mut *snapshot))
            .unwrap_or_default()
    }

    pub fn restore(&self, dropped: DropSnapshot) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.essential += dropped.essential;
            snapshot.key += dropped.key;
            snapshot.debug += dropped.debug;
            snapshot.raw += dropped.raw;
            snapshot.oversized += dropped.oversized;
            snapshot.configuration += dropped.configuration;
            snapshot.io += dropped.io;
        }
    }
}

struct LimitedRow(Vec<u8>);

impl Write for LimitedRow {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (MAX_ROW_BYTES - SEQUENCE_OVERHEAD - 1).saturating_sub(self.0.len()) {
            return Err(io::Error::other("runtime trace row exceeds byte budget"));
        }
        let required = self.0.len() + bytes.len();
        if required > self.0.capacity() {
            let capacity = required.max(self.0.capacity() * 2).min(MAX_ROW_BYTES);
            self.0.reserve_exact(capacity - self.0.len());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode_row(row: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut output = LimitedRow(Vec::with_capacity(512));
    serde_json::to_writer(&mut output, row).map_err(io::Error::other)?;
    output.0.push(b'\n');
    Ok(output.0)
}

#[cfg(test)]
#[path = "runtime_state.test.rs"]
mod tests;
