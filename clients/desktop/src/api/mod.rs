//! 两服务 HTTP API 客户端（blocking reqwest，跑在 smol::unblock 后台线程）。

pub mod document;
pub mod voice;
