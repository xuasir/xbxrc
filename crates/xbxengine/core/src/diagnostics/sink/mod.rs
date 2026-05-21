//! 媒体 runtime stats 统一写入（sink）；采集面只承载事实，投影在 `diagnostics::stats`。

pub(crate) mod runtime_stats_sink;

pub(crate) use runtime_stats_sink::RuntimeStatsSink;
