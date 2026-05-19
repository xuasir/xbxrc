pub mod channel;
pub mod endpoints;
pub mod events;
pub mod rpc;
pub mod service;

pub use channel::UpdateChannel;
pub use service::UpdaterService;

use std::sync::Arc;

pub type UpdaterServiceRef = Arc<UpdaterService>;
