//! 启动时清理 apalis 遗留的 stale worker 注册行。
//!
//! 背景：apalis 在 `Workers` 表注册 worker（`id` UNIQUE）。进程被非干净退出时
//! （Windows scheduled task 杀进程、deploy-installer 升级覆盖安装、崩溃、断电），
//! `Workers` 表残留上一实例的注册行且未注销；下次启动新 worker 用同名 `id` 注册
//! 触发 `WORKER_ALREADY_EXISTS`，worker 起不来 → 任务队列不处理（任务卡 Pending）。
//!
//! voice-cli 单实例独占 DB，启动时（注册 worker 之前）任何遗留 `Workers` 行都属
//! 已死实例，可安全清除。`Jobs.lock_by` 外键引用 `Workers.id`，故删 `Workers` 前必须
//! 释放**所有**任务的锁——不止 Running：已 Done/Failed 的任务也会保留历史 `lock_by`，
//! 同样会挡住 DELETE（外键冲突，实测 code 787）。同时把中断的 Running 任务重新入队
//! （at-least-once，apalis `max_attempts` 兜底防无限重试）。

use sqlx::SqlitePool;

/// 清除 stale worker 注册并释放其占用的任务锁。在 `SqliteStorage::setup` 之后、
/// worker 注册之前调用（每个 apalis DB 各调一次：STT `tasks.db` / TTS `tts_tasks.db`）。
pub async fn purge_stale_apalis_workers(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // 中断的 Running 任务重新入队为 Pending，使新 worker 能重新拉取处理
    // （干净启动时无 Running 行，为 no-op）。
    let requeued = sqlx::query("UPDATE Jobs SET status = 'Pending' WHERE status = 'Running'")
        .execute(pool)
        .await?
        .rows_affected();

    // 释放所有任务的 worker 锁（启动时均为上一死实例的 stale 锁，含 Done/Failed 的
    // 历史锁）——否则 DELETE FROM Workers 触发 Jobs.lock_by 外键冲突。
    let released = sqlx::query("UPDATE Jobs SET lock_by = NULL WHERE lock_by IS NOT NULL")
        .execute(pool)
        .await?
        .rows_affected();

    // 清除所有 worker 注册行；当前进程的 worker 尚未注册，不受影响。
    let cleared = sqlx::query("DELETE FROM Workers")
        .execute(pool)
        .await?
        .rows_affected();

    if cleared > 0 || released > 0 || requeued > 0 {
        tracing::info!(
            cleared_workers = cleared,
            released_job_locks = released,
            requeued_running_jobs = requeued,
            "purged stale apalis worker registrations from previous instance"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    // 内存库 + 单连接（避免内存库多连接各持一份）；开 FK 忠实复现外键路径。
    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("pragma");
        // 复刻 apalis Workers/Jobs 关键结构（Jobs.lock_by 外键引用 Workers.id）
        sqlx::query(
            "CREATE TABLE Workers (id TEXT NOT NULL UNIQUE, worker_type TEXT NOT NULL, \
             storage_name TEXT NOT NULL, layers TEXT, last_seen INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("workers");
        sqlx::query(
            "CREATE TABLE Jobs (id TEXT NOT NULL UNIQUE, job_type TEXT NOT NULL, \
             status TEXT NOT NULL DEFAULT 'Pending', lock_by TEXT, \
             FOREIGN KEY(lock_by) REFERENCES Workers(id))",
        )
        .execute(&pool)
        .await
        .expect("jobs");
        pool
    }

    #[tokio::test]
    async fn purges_stale_workers_and_releases_locks() {
        let pool = setup_pool().await;
        // 遗留：一个已死 worker + 被它锁住的 Running 任务 + 一个无关的 Done 任务
        sqlx::query(
            "INSERT INTO Workers (id, worker_type, storage_name) \
             VALUES ('w1','transcription-pipeline','sqlite')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO Jobs (id, job_type, status, lock_by) \
             VALUES ('j1','transcription','Running','w1')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO Jobs (id, job_type, status) VALUES ('j2','transcription','Done')")
            .execute(&pool)
            .await
            .unwrap();
        // j3：已 Done 但保留了历史 lock_by（apalis 不清）——曾挡住 DELETE 触发外键冲突。
        sqlx::query(
            "INSERT INTO Jobs (id, job_type, status, lock_by) \
             VALUES ('j3','transcription','Done','w1')",
        )
        .execute(&pool)
        .await
        .unwrap();

        purge_stale_apalis_workers(&pool).await.unwrap();

        let workers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM Workers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            workers, 0,
            "stale worker 行应被清除（含被 Done 任务历史锁引用）"
        );

        let (status, lock): (String, Option<String>) =
            sqlx::query_as("SELECT status, lock_by FROM Jobs WHERE id = 'j1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "Pending", "中断的 Running 任务应重新入队");
        assert_eq!(lock, None, "任务锁应被释放（否则 DELETE 违反外键）");

        // j3 Done 历史锁应被释放，但状态保持 Done（不重新入队）
        let (j3_status, j3_lock): (String, Option<String>) =
            sqlx::query_as("SELECT status, lock_by FROM Jobs WHERE id = 'j3'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(j3_status, "Done", "Done 任务不应被重新入队");
        assert_eq!(j3_lock, None, "Done 任务的历史锁应被释放（满足外键）");

        let done_untouched: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM Jobs WHERE id = 'j2' AND status = 'Done' AND lock_by IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(done_untouched, 1, "无锁的 Done 任务不受影响");
    }

    #[tokio::test]
    async fn noop_on_clean_db() {
        let pool = setup_pool().await;
        // 干净 DB（无 worker、无 Running）应不报错、无改动
        purge_stale_apalis_workers(&pool).await.unwrap();
        let workers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM Workers")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(workers, 0);
    }
}
