//! 解析子进程的跨平台执行脚手架（mineru / markitdown 共用）。
//!
//! 平台策略（Win53 实测教训）：
//! - **Unix**：process-wrap 进程组（`ProcessGroup::leader` + `KillOnDrop`）——
//!   kill 直接子进程即组杀孙进程
//! - **Windows**：原生 tokio Child（process-wrap 的 spawn 路径本身与 mineru
//!   3.4.5 不兼容——JobObject 与仅 CreationFlags 包装器都让本地 api 子进程
//!   退出码 120）+ [`TreeKillGuard`]（Drop 时 taskkill /T /F 兜底树杀）
//! - **Windows stdio**：文件重定向——tokio 管道是 OVERLAPPED 句柄，mineru.exe
//!   把继承的句柄传给其本地 api 子进程（uvicorn）后跨进程 I/O 失败 → 退出码
//!   120；同步文件句柄等价于已验证成功的 Git Bash `>` 重定向
//!
//! 注意：本模块 cfg(windows) 分支在 Mac 开发机上编译不到——改动须逐行自查
//! 可见性/借用/类型（beta.7-9 三次 CI 失败的教训）。

#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::process::Stdio;
use tokio::process::Command;

/// 平台统一的子进程句柄：
/// - Unix：process-wrap 的 ChildWrapper（进程组语义，kill 即组杀）
/// - Windows：原生 tokio Child + 树杀守卫（见模块注释）
#[cfg(unix)]
pub(crate) type ManagedChild = Box<dyn process_wrap::tokio::ChildWrapper>;
#[cfg(windows)]
pub(crate) struct ManagedChild {
    child: tokio::process::Child,
    tree_guard: TreeKillGuard,
}

#[cfg(windows)]
impl ManagedChild {
    pub(crate) fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub(crate) async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait().await?;
        // 进程已自然退出，整棵树随之终结，无需 Drop 兜底再杀
        self.tree_guard.disarm();
        Ok(status)
    }

    async fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill().await
    }
}

/// Windows 树杀守卫：包装 pid，Drop 未 disarm 时同步 `taskkill /PID /T /F`。
///
/// 覆盖"解析 future 被整体 drop"（worker 超时）路径：tokio Child 的
/// kill_on_drop 只杀直接子进程，孙进程（mineru 的 uvicorn api 子进程）会成
/// 僵尸泄漏（Win53 实测）。taskkill /F 毫秒级完成，在 Drop（tokio worker
/// 线程）里同步执行可接受——比僵尸 GPU/CPU 进程泄漏划算。
#[cfg(windows)]
struct TreeKillGuard {
    pid: Option<u32>,
}

#[cfg(windows)]
impl TreeKillGuard {
    fn disarm(&mut self) {
        self.pid = None;
    }
}

#[cfg(windows)]
impl Drop for TreeKillGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// 按模块头所述的平台策略 spawn 子进程
#[cfg(unix)]
pub(crate) fn spawn_managed(cmd: Command) -> std::io::Result<ManagedChild> {
    let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    wrapped.spawn()
}

#[cfg(windows)]
pub(crate) fn spawn_managed(mut cmd: Command) -> std::io::Result<ManagedChild> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    cmd.kill_on_drop(true);
    let child = cmd.spawn()?;
    let tree_guard = TreeKillGuard { pid: child.id() };
    Ok(ManagedChild { child, tree_guard })
}

/// Windows stdio 文件重定向（OVERLAPPED 管道雷，见模块头注释）。父目录不存在
/// 时创建；返回写入的两个日志路径（结束后读取兜底）
#[cfg(windows)]
pub(crate) fn redirect_stdio_to_files(
    cmd: &mut Command,
    out_path: &Path,
    err_path: &Path,
) -> std::io::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out_log = std::fs::File::create(out_path)?;
    let err_log = std::fs::File::create(err_path)?;
    cmd.stdout(Stdio::from(out_log))
        .stderr(Stdio::from(err_log));
    Ok(())
}

/// 子进程输出行泵：Unix 双流监控任务 → mpsc 行通道；Windows 文件重定向下无
/// 实时流，`recv` 永不产出（占位实现保持调用方 select 结构跨平台同构）
pub(crate) struct OutputPump {
    #[cfg(unix)]
    rx: tokio::sync::mpsc::Receiver<(String, String)>,
    #[cfg(unix)]
    tasks: (
        Option<tokio::task::JoinHandle<()>>,
        Option<tokio::task::JoinHandle<()>>,
    ),
    #[cfg(windows)]
    _placeholder: (),
}

impl OutputPump {
    /// 取下一行输出（流名, 行内容）；两路监控任务都结束时返回 None
    #[cfg(unix)]
    pub(crate) async fn recv(&mut self) -> Option<(String, String)> {
        self.rx.recv().await
    }

    #[cfg(windows)]
    pub(crate) async fn recv(&mut self) -> Option<(String, String)> {
        std::future::pending::<Option<(String, String)>>().await
    }

    /// 终止监控任务（进程收尾时调用；Windows 占位实现为 no-op）
    #[cfg(unix)]
    pub(crate) fn abort(&self) {
        if let Some(t) = &self.tasks.0 {
            t.abort();
        }
        if let Some(t) = &self.tasks.1 {
            t.abort();
        }
    }

    #[cfg(windows)]
    pub(crate) fn abort(&self) {}
}

/// 给已 spawn 的子进程挂输出行监控（Unix 管道双流；Windows 返回占位泵）
#[cfg(unix)]
pub(crate) fn attach_output_pump(child: &mut ManagedChild) -> OutputPump {
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    let tx_err = tx.clone();

    let tasks = (
        spawn_line_monitor(child.stdout().take(), "stdout", tx),
        spawn_line_monitor(child.stderr().take(), "stderr", tx_err),
    );
    OutputPump { rx, tasks }
}

#[cfg(windows)]
pub(crate) fn attach_output_pump(_child: &mut ManagedChild) -> OutputPump {
    OutputPump { _placeholder: () }
}

/// 单流行监控（流缺失时不起任务——piped 下理论必 Some，但生产代码不 unwrap）
#[cfg(unix)]
fn spawn_line_monitor<S>(
    stream: Option<S>,
    source: &'static str,
    tx: tokio::sync::mpsc::Sender<(String, String)>,
) -> Option<tokio::task::JoinHandle<()>>
where
    S: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncBufReadExt;

    stream.map(|stream| {
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send((source.to_string(), line)).await;
            }
        })
    })
}

/// 进程树击杀：Unix 在 ProcessGroup::leader 下 kill 即组杀；Windows 先
/// `taskkill /PID <pid> /T /F`（官方树杀，孙进程一并终止）再 kill 兜底直接
/// 子进程，并 disarm 守卫（树已处理，Drop 不必重杀）。
///
/// Windows 侧用 tokio Command（不开管道——OVERLAPPED 雷；stdio 置 null）
#[cfg(unix)]
pub(crate) async fn kill_tree(child: &mut ManagedChild) {
    let _ = Box::into_pin(child.kill()).await;
}

#[cfg(windows)]
pub(crate) async fn kill_tree(child: &mut ManagedChild) {
    if let Some(pid) = child.id() {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
    child.tree_guard.disarm();
}

/// 进程树击杀回归：ProcessGroup 包装下 kill 直接子进程时，孙进程必须一并终止
/// （此前 Child::kill 只杀直接子进程——Win53 实测 mineru 孙进程僵尸泄漏）。
/// Unix 用 /proc 探活；非 Unix 平台跳过（Windows 行为靠实机验证）。
#[cfg(all(test, unix))]
mod tree_kill_tests {
    use super::kill_tree;
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::process::Command;

    #[tokio::test]
    async fn process_group_kill_terminates_grandchildren() {
        let marker = std::env::temp_dir().join("dp-treekill-test.pid");
        let marker_str = marker.display().to_string();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!("sleep 300 & echo $! > {marker_str}; wait")])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        let mut child = wrapped.spawn().expect("spawn sh");

        // 等 sh 写出孙进程 pid 并让 sleep 真正起跑
        let mut grandchild_pid = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(content) = std::fs::read_to_string(&marker)
                && let Ok(pid) = content.trim().parse::<u32>()
            {
                grandchild_pid = Some(pid);
                break;
            }
        }
        let grandchild_pid = grandchild_pid.expect("sh 应写出孙进程 pid");
        let _ = std::fs::remove_file(&marker);
        let alive = |pid: u32| unsafe { libc::kill(pid as i32, 0) == 0 };
        assert!(alive(grandchild_pid), "孙进程应已在运行（前置条件）");

        kill_tree(&mut child).await;

        // 组击杀后孙进程应消失（短暂宽限轮询）
        let mut gone = false;
        for _ in 0..30 {
            if !alive(grandchild_pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            gone,
            "kill 进程组后孙进程 {grandchild_pid} 仍存活（树击杀失效）"
        );
    }
}
