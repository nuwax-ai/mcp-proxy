use std::future::Future;

use super::*;

#[derive(Debug)]
pub(super) enum ForwardError {
    Cancelled,
    BackendUnavailable,
    TransportClosed,
    Backend(ServiceError),
}

impl ForwardError {
    pub(super) fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub(super) fn message(&self) -> String {
        match self {
            Self::Cancelled => "Request cancelled".to_string(),
            Self::BackendUnavailable => {
                "Backend connection is not available, reconnecting...".to_string()
            }
            Self::TransportClosed => "Backend connection closed, please retry".to_string(),
            Self::Backend(error) => format!("Backend error: {error}"),
        }
    }

    pub(super) fn into_error_data(self) -> ErrorData {
        ErrorData::internal_error(self.message(), None)
    }
}

impl SseHandler {
    pub(super) fn load_backend_peer(&self) -> Result<Peer<RoleClient>, ForwardError> {
        let inner = self
            .peer
            .load_full()
            .ok_or(ForwardError::BackendUnavailable)?;
        if inner.peer.is_transport_closed() {
            return Err(ForwardError::TransportClosed);
        }
        Ok(inner.peer.clone())
    }

    pub(super) async fn forward_backend<T, F, Fut>(
        &self,
        context: &RequestContext<RoleServer>,
        operation: &'static str,
        call: F,
    ) -> Result<T, ForwardError>
    where
        F: FnOnce(Peer<RoleClient>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>>,
    {
        if context.ct.is_cancelled() {
            return Err(ForwardError::Cancelled);
        }
        let peer = self.load_backend_peer()?;

        tokio::select! {
            result = call(peer) => {
                result.map_err(|error| {
                    error!(operation, %error, "Backend request failed");
                    ForwardError::Backend(error)
                })
            }
            _ = context.ct.cancelled() => {
                info!(operation, mcp_id = %self.mcp_id, "Backend request cancelled");
                Err(ForwardError::Cancelled)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_handler_returns_typed_unavailable_error() {
        let handler = SseHandler::new_disconnected(
            "test".to_string(),
            ToolFilter::default(),
            ServerInfo::default(),
        );

        assert!(matches!(
            handler.load_backend_peer(),
            Err(ForwardError::BackendUnavailable)
        ));
    }

    #[test]
    fn forward_errors_keep_compatible_messages() {
        assert_eq!(
            ForwardError::BackendUnavailable.message(),
            "Backend connection is not available, reconnecting..."
        );
        assert_eq!(
            ForwardError::TransportClosed.message(),
            "Backend connection closed, please retry"
        );
        assert_eq!(ForwardError::Cancelled.message(), "Request cancelled");
    }
}
