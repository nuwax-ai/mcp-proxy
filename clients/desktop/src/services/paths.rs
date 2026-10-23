//! 托管服务的目录布局（全部落在客户端 appdata 下，不碰系统位置）。

use std::path::PathBuf;

use crate::app::app_data_dir;

pub const VOICE_CLI: &str = "voice-cli";
pub const DOCUMENT_PARSER: &str = "document-parser";

pub fn services_root() -> PathBuf {
    app_data_dir().join("services")
}

pub fn service_install_dir(service: &str) -> PathBuf {
    services_root().join(service)
}

/// 每服务一个 pidfile（记录进程组 leader pid），供孤儿回收与退出清理。
pub fn pid_file(service: &str) -> PathBuf {
    services_root().join(format!("{service}.pid"))
}
