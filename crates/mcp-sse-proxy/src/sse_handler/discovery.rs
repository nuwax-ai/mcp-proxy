use super::*;

#[derive(Clone, Debug)]
struct DiscoveryState {
    info: Arc<ServerInfo>,
    tools: Option<Arc<ListToolsResult>>,
}

#[derive(Clone, Debug)]
pub(super) struct DiscoveryCache {
    state: Arc<ArcSwap<DiscoveryState>>,
}

impl DiscoveryCache {
    pub fn new(info: ServerInfo, tools: Option<ListToolsResult>) -> Self {
        Self {
            state: Arc::new(ArcSwap::from_pointee(DiscoveryState {
                info: Arc::new(info),
                tools: tools.map(Arc::new),
            })),
        }
    }

    pub fn info(&self) -> ServerInfo {
        (*self.state.load().info).clone()
    }

    pub fn capabilities(&self) -> rmcp::model::ServerCapabilities {
        self.state.load().info.capabilities.clone()
    }

    pub fn tools(&self) -> Option<ListToolsResult> {
        self.state
            .load()
            .tools
            .as_ref()
            .map(|tools| (**tools).clone())
    }

    pub fn update_tools(&self, tools: ListToolsResult) {
        let tools = Arc::new(tools);
        self.state.rcu(|current| DiscoveryState {
            info: current.info.clone(),
            tools: Some(tools.clone()),
        });
    }

    pub fn update(&self, info: ServerInfo, tools: Option<ListToolsResult>) {
        let info = Arc::new(info);
        let tools = tools.map(Arc::new);
        self.state.rcu(|current| DiscoveryState {
            info: info.clone(),
            tools: tools.clone().or_else(|| current.tools.clone()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_observe_atomic_discovery_updates() {
        let cache = DiscoveryCache::new(ServerInfo::default(), Some(ListToolsResult::default()));
        let clone = cache.clone();
        let mut replacement = ServerInfo::default();
        replacement.server_info.name = "replacement".to_string();

        cache.update(replacement, None);

        assert_eq!(clone.info().server_info.name, "replacement");
        assert!(clone.tools().is_some());
    }
}
