use super::*;

impl mcp_common::BackendBridge for ProxyHandler {
    fn mcp_id(&self) -> &str {
        self.mcp_id()
    }

    fn get_server_info_json(&self) -> serde_json::Value {
        self.get_server_info_json()
    }

    fn is_backend_available(&self) -> bool {
        self.is_backend_available()
    }

    fn is_mcp_server_ready(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(self.is_mcp_server_ready())
    }

    fn is_terminated_async(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(self.is_terminated_async())
    }

    fn call_peer_method(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + '_>,
    > {
        let method = method.to_string();
        Box::pin(async move { ProxyHandler::call_peer_method(self, &method, params).await })
    }
}

#[cfg(test)]
mod meta_merge_tests {
    use super::merge_context_meta_into_params;
    use rmcp::model::{
        CallToolRequestParams, NumberOrString, ProgressToken, RequestMetaObject, RequestParamsMeta,
    };

    #[test]
    fn merge_copies_progress_token_from_context() {
        let mut params = CallToolRequestParams::new("echo");
        assert!(params.meta.is_none());

        let mut context_meta = RequestMetaObject::new();
        context_meta.set_progress_token(ProgressToken(NumberOrString::Number(42)));

        merge_context_meta_into_params(&mut params, &context_meta);

        assert_eq!(
            params.progress_token(),
            Some(ProgressToken(NumberOrString::Number(42)))
        );
    }

    #[test]
    fn merge_keeps_existing_params_meta_keys() {
        let mut params = CallToolRequestParams::new("echo");
        params.set_progress_token(ProgressToken(NumberOrString::Number(1)));

        let mut context_meta = RequestMetaObject::new();
        context_meta.set_progress_token(ProgressToken(NumberOrString::Number(99)));
        context_meta.insert("traceId".to_string(), serde_json::json!("abc"));

        merge_context_meta_into_params(&mut params, &context_meta);

        assert_eq!(
            params.progress_token(),
            Some(ProgressToken(NumberOrString::Number(1))),
            "params win on conflict"
        );
        assert_eq!(
            params.meta.as_ref().and_then(|m| m.get("traceId")),
            Some(&serde_json::json!("abc"))
        );
    }
}
