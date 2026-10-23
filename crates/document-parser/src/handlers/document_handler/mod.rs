//! 文档处理接口（上传、同步解析、结构化生成、解析器信息）
//!
//! 按职责拆分为多个子模块，本模块通过 re-export 保持对外统一的 API 面：
//! 所有 `handlers::document_handler::xxx` 路径与拆分前完全一致。

pub(crate) mod detection;
pub mod info;
pub mod parse_sync;
pub mod structured;
pub mod upload;

pub use info::*;
pub use parse_sync::*;
pub use structured::*;
pub use upload::*;
