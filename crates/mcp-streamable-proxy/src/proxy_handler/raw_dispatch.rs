use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::*;

type JsonResult = Result<Value, String>;

impl ProxyHandler {
    /// Call a backend method through the version-independent JSON bridge.
    pub async fn call_peer_method(&self, method: &str, params: Value) -> JsonResult {
        let peer = self.raw_peer()?;
        match method {
            "tools/list" => list_tools(peer, params).await,
            "tools/call" => call_tool(peer, params).await,
            "resources/list" => list_resources(peer, params).await,
            "resources/read" => read_resource(peer, params).await,
            "resources/templates/list" => list_resource_templates(peer, params).await,
            "resources/subscribe" => subscribe(peer, params).await,
            "resources/unsubscribe" => unsubscribe(peer, params).await,
            "prompts/list" => list_prompts(peer, params).await,
            "prompts/get" => get_prompt(peer, params).await,
            "completion/complete" => complete(peer, params).await,
            "logging/setLevel" => set_log_level(peer, params).await,
            "tasks/list" => forward_raw(peer, "tasks/list", params).await,
            "tasks/get" => get_task(peer, params).await,
            "tasks/result" => forward_raw(peer, "tasks/result", params).await,
            "tasks/update" => update_task(peer, params).await,
            "tasks/cancel" => cancel_task(peer, params).await,
            _ => Err(format!("unsupported method: {method}")),
        }
    }

    fn raw_peer(&self) -> Result<Peer<RoleClient>, String> {
        let inner = self
            .peer
            .load_full()
            .ok_or_else(|| "backend connection is not available (reconnecting)".to_string())?;
        if inner.peer.is_transport_closed() {
            return Err("backend transport is closed".to_string());
        }
        Ok(inner.peer.clone())
    }

    /// Forward an arbitrary JSON-RPC method to the backend verbatim (transparent bridge).
    ///
    /// Unlike [`call_peer_method`](Self::call_peer_method) (a whitelist dispatcher),
    /// this forwards ANY method string to the backend as a custom request, so the
    /// proxy stays transparent for legacy methods (`tasks/list`, `tasks/result`) and
    /// vendor extensions alike. Used by `ServerHandler::on_custom_request`.
    pub(super) async fn forward_to_backend(&self, method: &str, params: Value) -> JsonResult {
        let peer = self.raw_peer()?;
        forward_raw(peer, method, params).await
    }
}

fn parse<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, String> {
    serde_json::from_value(params).map_err(|error| format!("invalid params for {method}: {error}"))
}

fn serialize<T: Serialize>(value: T) -> JsonResult {
    serde_json::to_value(value)
        .map_err(|error| format!("failed to serialize backend result: {error}"))
}

fn page_params(
    params: Value,
    method: &str,
) -> Result<Option<rmcp::model::PaginatedRequestParams>, String> {
    if params.is_null() {
        return Ok(None);
    }
    parse(params, method)
}

async fn list_tools(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let result = peer
        .list_tools(page_params(params, "tools/list")?)
        .await
        .map_err(|error| format!("tools/list failed: {error:?}"))?;
    serialize(result)
}

async fn call_tool(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request: CallToolRequestParams = parse(params, "tools/call")?;
    match peer
        .call_tool_once(request)
        .await
        .map_err(|error| format!("tools/call failed: {error:?}"))?
    {
        CallToolResponse::Complete(result) => serialize(result),
        CallToolResponse::InputRequired(result) => serialize(result),
        CallToolResponse::Task(result) => serialize(result),
        other => Err(format!("unsupported tools/call response: {other:?}")),
    }
}

async fn list_resources(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let result = peer
        .list_resources(page_params(params, "resources/list")?)
        .await
        .map_err(|error| format!("resources/list failed: {error:?}"))?;
    serialize(result)
}

async fn read_resource(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request = parse(params, "resources/read")?;
    match peer
        .read_resource_once(request)
        .await
        .map_err(|error| format!("resources/read failed: {error:?}"))?
    {
        ReadResourceResponse::Complete(result) => serialize(result),
        ReadResourceResponse::InputRequired(result) => serialize(result),
        other => Err(format!("unsupported resources/read response: {other:?}")),
    }
}

async fn list_resource_templates(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let result = peer
        .list_resource_templates(page_params(params, "resources/templates/list")?)
        .await
        .map_err(|error| format!("resources/templates/list failed: {error:?}"))?;
    serialize(result)
}

#[allow(deprecated)]
async fn subscribe(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    peer.subscribe(parse(params, "resources/subscribe")?)
        .await
        .map_err(|error| format!("resources/subscribe failed: {error:?}"))?;
    Ok(serde_json::json!({}))
}

#[allow(deprecated)]
async fn unsubscribe(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    peer.unsubscribe(parse(params, "resources/unsubscribe")?)
        .await
        .map_err(|error| format!("resources/unsubscribe failed: {error:?}"))?;
    Ok(serde_json::json!({}))
}

async fn list_prompts(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let result = peer
        .list_prompts(page_params(params, "prompts/list")?)
        .await
        .map_err(|error| format!("prompts/list failed: {error:?}"))?;
    serialize(result)
}

async fn get_prompt(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request = parse(params, "prompts/get")?;
    match peer
        .get_prompt_once(request)
        .await
        .map_err(|error| format!("prompts/get failed: {error:?}"))?
    {
        GetPromptResponse::Complete(result) => serialize(result),
        GetPromptResponse::InputRequired(result) => serialize(result),
        other => Err(format!("unsupported prompts/get response: {other:?}")),
    }
}

async fn complete(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let result = peer
        .complete(parse(params, "completion/complete")?)
        .await
        .map_err(|error| format!("completion/complete failed: {error:?}"))?;
    serialize(result)
}

async fn set_log_level(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    #[allow(deprecated)]
    peer.set_level(parse(params, "logging/setLevel")?)
        .await
        .map_err(|error| format!("logging/setLevel failed: {error:?}"))?;
    Ok(serde_json::json!({}))
}

async fn get_task(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request = GetTaskRequest::new(parse(params, "tasks/get")?);
    let result = peer
        .send_request(ClientRequest::GetTaskRequest(request))
        .await
        .map_err(|error| format!("tasks/get failed: {error:?}"))?;
    match result {
        ServerResult::GetTaskResult(result) => serialize(result),
        other => Err(format!("unexpected tasks/get response: {other:?}")),
    }
}

async fn update_task(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request: rmcp::model::UpdateTaskParams = parse(params, "tasks/update")?;
    peer.update_task(request)
        .await
        .map_err(|error| format!("tasks/update failed: {error:?}"))?;
    Ok(serde_json::json!({}))
}

async fn cancel_task(peer: Peer<RoleClient>, params: Value) -> JsonResult {
    let request: rmcp::model::CancelTaskParams = parse(params, "tasks/cancel")?;
    peer.cancel_task(request)
        .await
        .map_err(|error| format!("tasks/cancel failed: {error:?}"))?;
    Ok(serde_json::json!({}))
}

/// Forward a method to the backend verbatim as a custom JSON-RPC request.
///
/// Used for legacy methods (`tasks/list`, `tasks/result`) whose typed
/// request/result were dropped in rmcp 3.1.0 (SEP-2663 Tasks redesign) but
/// that older backends may still implement. Keeps the proxy a transparent,
/// version-tolerant bridge instead of hard-failing on spec-removed methods.
async fn forward_raw(peer: Peer<RoleClient>, method: &str, params: Value) -> JsonResult {
    let result = peer
        .send_request(ClientRequest::CustomRequest(
            rmcp::model::CustomRequest::new(method.to_string(), Some(params)),
        ))
        .await
        .map_err(|error| format!("{method} failed: {error:?}"))?;
    match result {
        ServerResult::CustomResult(result) => Ok(result.0),
        other => Err(format!("unexpected {method} response: {other:?}")),
    }
}
