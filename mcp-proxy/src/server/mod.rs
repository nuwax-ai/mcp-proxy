pub mod handlers;
mod mcp_dynamic_router_service;
mod middlewares;
mod protocol_detector;
mod router_layer;
mod task;
pub mod telemetry;

pub use handlers::{get_health, get_ready};

pub use middlewares::set_layer;

pub use protocol_detector::detect_mcp_protocol;
pub use router_layer::get_router;
pub use task::{mcp_start_task, schedule_check_mcp_live, start_schedule_task};
pub use telemetry::{
    create_telemetry_layer, init_tracer_provider, log_service_info, shutdown_telemetry,
};
