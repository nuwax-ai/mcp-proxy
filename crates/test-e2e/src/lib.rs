//! test-e2e：document-parser / voice-cli 已部署服务的端到端集成测试。
//!
//! 公共层（本 lib）编译一次，供 `tests/*.rs` 各测试二进制共享；测试入口按
//! 服务分文件。目标服务地址三级配置：环境变量 > 仓库根 `.env.local` > 默认
//! 本机端口；目标不可达时测试显式 SKIP（非失败），保证无服务机器
//! `cargo test --workspace` 全绿。详见 README.md。

pub mod assets;
pub mod common;
pub mod ws;
