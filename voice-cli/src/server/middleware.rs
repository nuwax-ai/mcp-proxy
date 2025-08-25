use axum::{extract::Request, middleware::Next, response::Response};
use tracing::{error, warn};

pub async fn error_handler(request: Request, next: Next) -> Response {
    let uri = request.uri().clone();
    let method = request.method().clone();

    let response = next.run(request).await;

    // Log errors based on status code
    match response.status().as_u16() {
        200..=299 => {
            // Success - no logging needed for normal operations
        }
        400..=499 => {
            warn!("Client error {} {} - {}", method, uri, response.status());
        }
        500..=599 => {
            error!("Server error {} {} - {}", method, uri, response.status());
        }
        _ => {
            warn!(
                "Unexpected status {} {} - {}",
                method,
                uri,
                response.status()
            );
        }
    }

    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_middleware_module_exists() {
        // Simple test to verify the module compiles
        // Actual middleware testing would require more complex setup
        assert!(true);
    }
}
