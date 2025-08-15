mod auth;
mod mark_log_span;
mod mcp_router_json;
mod mcp_update_latest_layer;
mod request_id;
mod request_logger;
mod server_time;

use crate::model::AppState;
use axum::Router;
use axum::middleware::from_fn;
use mark_log_span::MyDefaultMakeSpan;
use mcp_router_json::mcp_json_config_extract;
use request_id::set_request_id;
use request_logger::log_request;
use server_time::ServerTimeLayer;
use tower::ServiceBuilder;
use tower_http::LatencyUnit;
use tower_http::compression::CompressionLayer;
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

pub use mcp_update_latest_layer::MySseRouterLayer;

// pub use auth::{extract_user, verify_token};

// pub trait TokenVerify {
//     type Error: fmt::Debug;
//     fn verify(&self, token: &str) -> Result<User, Self::Error>;
// }

const REQUEST_ID_HEADER: &str = "x-request-id";
const SERVER_TIME_HEADER: &str = "x-server-time";

pub fn set_layer(app: Router, state: AppState) -> Router {
    app.layer(
        ServiceBuilder::new()
            .layer(from_fn(mcp_json_config_extract))
            .layer(from_fn(set_request_id))
            .layer(from_fn(log_request))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(MyDefaultMakeSpan::new().include_headers(false))
                    .on_request(DefaultOnRequest::new().level(Level::INFO))
                    .on_response(
                        DefaultOnResponse::new()
                            .level(Level::INFO)
                            .latency_unit(LatencyUnit::Micros),
                    ),
            )
            .layer(CompressionLayer::new().gzip(true).br(true).deflate(true))
            .layer(ServerTimeLayer)
            .layer(MySseRouterLayer::new(state.clone())),
    )
}
