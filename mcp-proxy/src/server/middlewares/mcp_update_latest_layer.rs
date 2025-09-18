use std::task::{Context, Poll};

use axum::extract::Request;
use log::debug;
use tower::{Layer, Service};

use crate::{
    get_proxy_manager,
    model::{AppState, McpRouterPath},
};

#[derive(Clone)]
pub struct MySseRouterLayer {
    state: AppState,
}
#[allow(dead_code)]
#[derive(Clone)]
pub struct MySseRouterService<S> {
    inner: S,
    state: AppState,
}

impl MySseRouterLayer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for MySseRouterLayer {
    type Service = MySseRouterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MySseRouterService {
            inner,
            state: self.state.clone(),
        }
    }
}

impl<S, B> Service<Request<B>> for MySseRouterService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let path = req.uri().path().to_string();
        //检查请求路径,是否 /mcp 开头
        let check_mcp_path = McpRouterPath::check_mcp_path(&path);
        if check_mcp_path {
            //请求路径,可能是: /mcp/{mcp_id}/sse,或者 /mcp/{mcp_id}/message
            let mcp_router_path = McpRouterPath::from_url(&path);
            if let Some(mcp_router_path) = mcp_router_path {
                let mcp_id = mcp_router_path.mcp_id.clone();
                debug!("更新最后访问时间,请求访问MCP ID: {mcp_id}");
                // 更新最后访问时间
                get_proxy_manager().update_last_accessed(&mcp_id);
            }
        }

        self.inner.call(req)
    }
}
