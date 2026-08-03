use arc_swap::{ArcSwap, ArcSwapOption};
pub use mcp_common::ToolFilter;
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceError,
    model::{
        CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation, ListToolsResult,
        PaginatedRequestParam, ProtocolVersion, ServerInfo,
    },
    service::{NotificationContext, Peer, RequestContext, RunningService},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};
use tracing::{debug, error, info, warn};

mod backend_session;
mod direct_core;
mod discovery;
mod dispatch;
mod forwarding;
mod lifecycle;

pub use backend_session::BackendSessionHandler;
pub use direct_core::SseHandler;
use discovery::DiscoveryCache;
pub use dispatch::SseServerHandler;
use forwarding::ForwardError;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
