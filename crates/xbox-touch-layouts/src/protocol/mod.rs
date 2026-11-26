mod actions;
mod compiler;
mod registry;
mod styles;
mod types;
mod versions;

pub use compiler::{compile_layout, compile_layout_json};
pub use registry::{protocol_information_json, protocol_revision};
pub use types::{CompiledTouchLayout, TouchCompiledControl, TouchProtocolIssue};

pub const MAX_COMPILED_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
#[path = "protocol.test.rs"]
mod tests;
