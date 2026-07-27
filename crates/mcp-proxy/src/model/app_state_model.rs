use std::{ops::Deref, sync::Arc};

use crate::AppConfig;

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}
#[allow(unused)]
#[derive(Debug)]
pub struct AppStateInner {
    pub addr: String,
    pub config: AppConfig,
}

impl Deref for AppState {
    type Target = AppStateInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                addr: format!("0.0.0.0:{}", config.server.port),
                config,
            }),
        }
    }
}
