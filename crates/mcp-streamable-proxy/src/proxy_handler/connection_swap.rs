use super::*;

impl ProxyHandler {
    /// Update the backend from a high-level client connection.
    pub fn swap_backend_from_connection(
        &self,
        conn: Option<crate::client::StreamClientConnection>,
    ) {
        match conn {
            Some(c) => {
                let running = c.into_running_service();
                self.swap_backend(Some(running));
            }
            None => {
                self.swap_backend(None);
            }
        }
    }
}
