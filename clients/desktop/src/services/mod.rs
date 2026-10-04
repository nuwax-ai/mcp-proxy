//! 服务托管：provision（装文件）+ supervisor（子进程生命周期）+ 对 UI 的状态机。
//!
//! 状态机（每服务一份）：
//! NotInstalled → Provisioning(进度) → Stopped → Starting → Running(端口/版本)
//!                    ↘ Failed(错误) ←── 崩溃退避重启耗尽 ←──┘

pub mod manager;
pub mod paths;
pub mod provision;
pub mod supervisor;
