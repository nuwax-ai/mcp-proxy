//! 文档任务级解析取消注册表。
//!
//! 打通 `POST /api/v1/tasks/{id}/cancel` → 解析子进程 kill：
//! [`DocumentService`](crate::services::DocumentService) 解析开始时以文档
//! task_id 为 key 注册令牌（并把令牌经 [`DocumentParser::parse_with_cancel`]
//! 下传给引擎，execute 层 select 轮询），[`TaskService::cancel_task`] 改完
//! 状态后调 [`request_parse_cancel`] 触发令牌，子进程被 kill、解析 future
//! 返回 Err（Cancelled 终态不被 [`TaskService::set_task_error`] 覆盖）。
//!
//! 注册表用 `std::sync::Mutex`（注册/注销/查询都是非 async 快操作；令牌
//! 本身的 `cancel()` 是 async，在持锁外调用）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::parsers::mineru_parser::CancellationToken;

fn registry() -> &'static Mutex<HashMap<String, CancellationToken>> {
    static REG: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册指定文档任务的解析取消令牌（解析开始时调用；同 key 重复注册覆盖）
pub fn register(task_id: &str, token: CancellationToken) {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(task_id.to_string(), token);
}

/// 注册并返回 RAII 注销 guard：解析 future 无论正常结束、返回 Err、超时还是
/// 被 drop（worker 超时 drop 整个解析 future），guard 的 Drop 都会注销注册项——
/// 手工 register/unregister 对在 drop 路径永远走不到 unregister，注册表会泄漏
pub fn register_guarded(task_id: &str, token: CancellationToken) -> ParseCancelGuard {
    register(task_id, token);
    ParseCancelGuard {
        task_id: task_id.to_string(),
    }
}

/// 注销 guard（Drop 时注销注册表项；显式 unregister 与之等价、幂等）
pub struct ParseCancelGuard {
    task_id: String,
}

impl Drop for ParseCancelGuard {
    fn drop(&mut self) {
        unregister(&self.task_id);
    }
}

/// 注销（解析结束/失败/超时路径统一调用；幂等）
pub fn unregister(task_id: &str) {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(task_id);
}

/// 请求取消：触发令牌（不注销——由解析方收尾时 unregister）
pub async fn request_parse_cancel(task_id: &str) -> bool {
    let token = registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(task_id)
        .cloned();
    match token {
        Some(t) => {
            t.cancel().await;
            true
        }
        None => false,
    }
}

/// 当前活跃的解析任务数（诊断/测试用）
pub fn active_count() -> usize {
    registry().lock().unwrap_or_else(|p| p.into_inner()).len()
}

/// 测试辅助：清空注册表
#[cfg(test)]
pub fn clear_for_test() {
    registry().lock().unwrap_or_else(|p| p.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// register_guarded 的 Drop 注销：guard 离开作用域（含 future 被 drop）后
    /// 注册表不再持有该任务——worker 超时 drop 解析 future 的泄漏路径回归
    #[tokio::test]
    async fn guarded_register_unregisters_on_drop() {
        clear_for_test();
        let token = CancellationToken::new();
        {
            let _guard = register_guarded("task-drop", token.clone());
            assert_eq!(active_count(), 1);
            assert!(request_parse_cancel("task-drop").await);
        }
        assert_eq!(
            active_count(),
            0,
            "guard drop 后注册表应清空（drop 路径不泄漏）"
        );
        assert!(!request_parse_cancel("task-drop").await);
    }

    #[tokio::test]
    async fn registry_roundtrip_register_request_unregister() {
        clear_for_test();
        let token = CancellationToken::new();
        register("task-1", token.clone());
        assert_eq!(active_count(), 1);

        // 请求取消：令牌触发、返回 true
        assert!(request_parse_cancel("task-1").await);
        assert!(token.is_cancelled().await, "令牌应被置位");

        // 未注册的任务：返回 false 不 panic
        assert!(!request_parse_cancel("nope").await);

        // 注销后：不可再取消（由解析方收尾 unregister）
        unregister("task-1");
        assert_eq!(active_count(), 0);
        assert!(!request_parse_cancel("task-1").await);
    }
}
