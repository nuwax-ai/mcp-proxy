//! 单实例锁：appdata 上的 OS 文件写锁（fd-lock）。
//! 进程正常退出或被强杀时由 OS 自动释放——不存在陈旧锁文件把后续启动堵死的问题。

use std::{fs::File, path::Path};

use anyhow::Result;
use fd_lock::RwLock;

/// 持有进程 lifetime 的锁 guard——drop（进程退出）即释放。
pub struct InstanceGuard {
    // guard 借用已 leak 的锁，两者都活到进程结束
    _guard: fd_lock::RwLockWriteGuard<'static, File>,
}

pub fn acquire(lock_dir: &Path) -> Result<InstanceGuard> {
    let file = File::create(lock_dir.join("single-instance.lock"))?;
    let lock: &'static mut RwLock<File> = Box::leak(Box::new(RwLock::new(file)));
    match lock.try_write() {
        Ok(guard) => Ok(InstanceGuard { _guard: guard }),
        Err(_) => Err(anyhow::anyhow!(
            "another ai-toolbox instance holds the lock"
        )),
    }
}
