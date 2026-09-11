//! Shared helpers for `document-parser` / `voice-cli` CLI modules.

use crate::{
    bundled_binary_path, default_document_parser_install_dir, default_voice_cli_install_dir,
    group_for_user, make_executable, resolve_service_user, restart_in_dir, status_in_dir,
    uninstall_in_dir,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::{ServiceAction, ServiceDirArgs};
use crate::cli::assets::maybe_copy_companion_libs;

pub const CONFIG_FILENAME: &str = "config.yml";

/// 剥离 YAML 值的行内注释：引号值取到闭合引号为止（保留引号让调用方统一剥壳），
/// 裸值在首个 `#` 处截断（`#` 前至少一个空白才是注释，`a#b` 这种伪注释不截）。
/// 出厂模板的 port/bucket/default_model 行都带 `# ← 说明` 尾注——朴素解析
/// 不剥注释会读出脏值或直接解析失败
pub(crate) fn strip_yaml_inline_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'"') || bytes.first() == Some(&b'\'') {
        let quote = bytes[0];
        if let Some(end) = bytes[1..].iter().position(|&b| b == quote).map(|i| i + 1) {
            return &value[..end + 1];
        }
        // 无闭合引号：畸形行，退回裸值规则
    }
    match value.find(" #") {
        Some(idx) => value[..idx].trim_end(),
        None => value,
    }
}

/// Read first non-comment `port:` value from a YAML-ish config file.
pub fn read_server_port(config_path: &Path) -> Option<u16> {
    let content = fs::read_to_string(config_path).ok()?;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("port:") {
            return strip_yaml_inline_comment(rest.trim())
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .parse()
                .ok();
        }
    }
    None
}

pub fn canonicalize_install_dir(path: &Path) -> PathBuf {
    // dunce：Windows 的 std canonicalize 返回 \\?\ 扩展长度路径——进了任务计划
    // XML 的 WorkingDirectory/Command 后 schtasks 无法正确处理（实测 53 服务
    // 行为异常）。dunce 在 Windows 返回普通绝对路径，Unix 行为等同 std。
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn resolve_user_group(user: Option<String>) -> Result<(String, String)> {
    let user = resolve_service_user(user).context("resolve service user")?;
    let group = group_for_user(&user).context("resolve service group")?;
    Ok((user, group))
}

/// Resolve `.` or missing install dir to the service default under `$HOME`.
pub fn resolve_service_install_dir(service: &str, install_dir: &Path) -> PathBuf {
    if install_dir.as_os_str().is_empty() || install_dir == Path::new(".") {
        match service {
            "document-parser" => default_document_parser_install_dir(),
            "voice-cli" => default_voice_cli_install_dir(),
            _ => install_dir.to_path_buf(),
        }
    } else {
        install_dir.to_path_buf()
    }
}

/// voice-cli 启动秒级就绪；document-parser 首启含 MinerU 环境检查，放宽。
const VOICE_CLI_HEALTH_WAIT_SECS: u64 = 45;
const DOCUMENT_PARSER_HEALTH_WAIT_SECS: u64 = 120;
/// upgrade 自动重启后的健康等待（尽力而为，非致命——重启成功即契约达成）
const UPGRADE_HEALTH_WAIT_SECS: u64 = 45;

/// Poll `http://127.0.0.1:{port}{path}` until curl succeeds or timeout.
pub fn wait_for_health(port: u16, path: &str, timeout_secs: u64) -> bool {
    let url = format!("http://127.0.0.1:{port}{path}");
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if Command::new("curl")
            .args([
                "-fsS",
                "-o",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
                &url,
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        thread::sleep(Duration::from_secs(2));
    }
    false
}

/// Print a short success banner after `install` completes.
///
/// 健康等待超时 = 安装失败（Fail Fast）：服务注册成功但起不来时，不能打印 ✅
/// 让用户误以为部署完成（131 无 GPU 机器装 CUDA voice-cli 崩溃循环的实测教训）。
/// 排障指引按当前服务后端给出。
pub fn print_install_success(service: &str, install_dir: &Path, port: u16) -> Result<()> {
    let timeout_secs = match service {
        "document-parser" => DOCUMENT_PARSER_HEALTH_WAIT_SECS,
        _ => VOICE_CLI_HEALTH_WAIT_SECS,
    };
    print!("\n   Checking /health (up to {timeout_secs}s)");
    let healthy = wait_for_health(port, "/health", timeout_secs);
    if !healthy {
        println!(" … TIMEOUT");
        println!("\n⚠️  {service} 已安装但服务未就绪（{timeout_secs}s 内 /health 未响应）");
        println!("   排查：");
        if cfg!(target_os = "windows") {
            println!("     deploy-installer {service} service status   # 任务状态与 last result");
            println!("     任务计划程序 taskschd.msc → com.nuwax.{service} → 历史");
        } else if cfg!(target_os = "macos") {
            println!(
                "     tail -30 ~/logs/launchd.*.log 或 {}/logs/ 下最新日志",
                install_dir.display()
            );
            println!("     deploy-installer {service} service status");
        } else {
            println!("     journalctl -u {service} -n 30 --no-pager   # 崩溃原因");
            println!("     deploy-installer {service} service status");
        }
        bail!("{service} installed but health check timed out on port {port}");
    }
    println!(" … OK");

    println!("\n✅ {service} → http://127.0.0.1:{port}");
    println!("   API docs: http://127.0.0.1:{port}/api/docs");
    println!("   Dir: {}", install_dir.display());
    println!("   Ops: deploy-installer {service} service status");
    Ok(())
}

/// Ensure required Whisper model files exist after an OSS extract.
/// Copy `vendor/<platform>/<service>` into `install_dir/<service>` when the bundle exists.
///
/// If the bundle is missing, keeps an existing binary in place; otherwise errors.
/// For `voice-cli`, also copies macOS `@rpath` companion dylibs from the same vendor dir.
pub fn ensure_bundled_binary(service: &str, install_dir: &Path, quiet: bool) -> Result<PathBuf> {
    let bundled = bundled_binary_path(service);
    let dst = install_dir.join(crate::binary_name(service));
    if bundled.exists() {
        stop_service_for_binary_replace(service, quiet);
        copy_file_atomic(&bundled, &dst)?;
        make_executable(&dst)?;
        if !quiet {
            println!("  copied binary from {}", bundled.display());
        }
        maybe_copy_companion_libs(service, &bundled, install_dir, quiet)?;
    } else if !dst.exists() {
        bail!(
            "bundled binary not found at {} and no existing binary in {}",
            bundled.display(),
            install_dir.display()
        );
    } else {
        // Keep existing binary; still require companions (from vendor or already installed).
        maybe_copy_companion_libs(service, &bundled, install_dir, quiet)?;
    }
    Ok(dst)
}

/// Replace `install_dir/<service>` from the npm/vendor bundle and print a restart hint.
pub fn upgrade_bundled_binary(service: &str, install_dir: &Path) -> Result<()> {
    let bundled = bundled_binary_path(service);
    let dst = install_dir.join(crate::binary_name(service));
    if !bundled.exists() {
        bail!("bundled binary not found at {}", bundled.display());
    }
    // 替换前探测"服务在跑?"——替换后自动重启原在跑的服务，升级零感知：
    // Windows 停任务换文件（否则服务留在停止态），Linux/macOS rename 顶替
    // （旧进程继续跑旧版本直到重启）——两种场景都需要重启才生效新版本
    let was_running = crate::checks::unit_is_active(service, crate::platform::current_backend());
    stop_service_for_binary_replace(service, false);
    copy_file_atomic(&bundled, &dst)?;
    make_executable(&dst)?;
    maybe_copy_companion_libs(service, &bundled, install_dir, false)?;
    println!("✅ upgraded {} → {}", bundled.display(), dst.display());
    if was_running {
        match restart_in_dir(service, Some(install_dir.to_path_buf())) {
            Ok(()) => {
                // 重启成功即契约达成；健康等待尽力而为（首启含模型加载时 45s 可能不够）
                let port = read_server_port(&install_dir.join(CONFIG_FILENAME));
                if let Some(port) = port {
                    print!("   restarting");
                    if wait_for_health(port, "/health", UPGRADE_HEALTH_WAIT_SECS) {
                        println!(" … healthy");
                    } else {
                        println!(" … health still warming up (service was restarted)");
                    }
                } else {
                    println!("   restarted (config.yml unreadable — skipping health wait)");
                }
            }
            Err(e) => println!(
                "   ⚠️ restart failed: {e} — run: deploy-installer {service} service restart"
            ),
        }
    } else {
        println!("   Run: deploy-installer {service} service restart");
    }
    Ok(())
}

/// 原子复制文件：先写入同目录临时文件，再 rename 顶替目标。
///
/// 直接 `fs::copy` 覆盖**正在运行**的二进制会被 Linux 以 ETXTBSY 拒绝
/// （macOS 无此限制，因此仅在 Linux 部署中暴露）；dlopen 加载中的 .so 同理。
/// rename(2) 替换运行中的可执行文件是合法的：旧 inode 继续服务已运行的
/// 进程，新文件即刻对后续启动生效——install 重跑 / upgrade 无需先停服务。
///
/// Windows 特例：目标 exe 正在运行时连 rename 顶替也被拒（ERROR_ACCESS_DENIED），
/// 但把**运行中的旧文件改名挪走**是合法的——降级链：顶替失败 → 旧文件
/// rename 为 `.old-<pid>`（进程退出前可能残留，下次替换时清理）→ 新文件就位。
pub(crate) fn copy_file_atomic(src: &Path, dst: &Path) -> Result<()> {
    let tmp = dst.with_extension(format!("new-{}", std::process::id()));
    fs::copy(src, &tmp).with_context(|| format!("copy {} → {}", src.display(), tmp.display()))?;
    if let Err(e) = fs::rename(&tmp, dst) {
        #[cfg(windows)]
        {
            let moved_away = dst.with_extension(format!("old-{}", std::process::id()));
            if fs::rename(dst, &moved_away).is_ok() && fs::rename(&tmp, dst).is_ok() {
                let _ = fs::remove_file(&moved_away);
                return Ok(());
            }
        }
        let _ = fs::remove_file(&tmp);
        return Err(anyhow::Error::new(e).context(format!(
            "replace {} ← {}",
            dst.display(),
            src.display()
        )));
    }
    Ok(())
}

/// Windows：替换服务二进制前的预停——结束任务计划实例并等待文件解锁。
///
/// 非 Windows 上是空操作（unix 侧 rename 顶替运行中文件本就合法）。
#[cfg(windows)]
fn stop_service_for_binary_replace(service: &str, quiet: bool) {
    use crate::platform::{ServiceBackend, current_backend};
    if current_backend() != ServiceBackend::TaskScheduler {
        return;
    }
    let name = format!("com.nuwax.{service}");
    if !crate::task_scheduler::task_exists(&name) {
        return;
    }
    if !quiet {
        println!("  stopping running task {name} before binary replace…");
    }
    let _ = crate::task_scheduler::end(&name);
    // 进程退出与句柄释放有延迟：轮询任务状态（非 Running 即视为停稳，≤15s）
    for _ in 0..30 {
        if crate::task_scheduler::task_state(&name) != crate::task_scheduler::TaskState::Running {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    // 再沉降 1s：任务态转 Ready 与进程退出/端口释放之间仍有窗口，
    // 紧随其后的端口预检会误报冲突（Windows 重装实测的竞态缓解）
    thread::sleep(Duration::from_secs(1));
}

#[cfg(not(windows))]
fn stop_service_for_binary_replace(_service: &str, _quiet: bool) {}

/// 返回 true=服务已启动（调用方做健康等待）；false=SSH-only 降级未启动（有意状态，
/// 调用方跳过健康等待、退出码保持 0）。
///
/// 处理 macOS launchd 服务安装结果：SSH-only（无桌面会话）时 bootstrap/enable
/// 会以 exit 134 失败，但 **plist 已写入** `~/Library/LaunchAgents/`——这不是安装
/// 失败，降级为成功退出并给出激活指引，避免用户对着 "Failed to execute command
/// with no output" 无所适从。
///
/// `manual_cmd` 为该服务的手动运行命令（各服务子命令形状不同：document-parser
/// 是 `--config <path> server`，voice-cli 是 `server run --config <path>`）。
#[cfg(target_os = "macos")]
pub fn handle_launchd_install_result(
    result: Result<()>,
    service_name: &str,
    manual_cmd: &str,
) -> Result<bool> {
    if result.is_ok() || crate::macos_gui_session_present() {
        return result.map(|_| true);
    }
    println!(
        "\n⚠️  {service_name} 服务未启动：当前 SSH 会话无桌面登录（launchd gui domain 不可用）。"
    );
    println!("   plist 已写入 ~/Library/LaunchAgents/，桌面登录后服务自动启动。");
    println!("   也可手动运行：{manual_cmd}");
    Ok(false)
}

#[cfg(not(target_os = "macos"))]
pub fn handle_launchd_install_result(
    result: Result<()>,
    _service_name: &str,
    _manual_cmd: &str,
) -> Result<bool> {
    result.map(|_| true)
}

/// Shared uninstall / status / restart; `on_install` handles service-specific Install.
pub fn dispatch_service_action(
    service_name: &str,
    action: ServiceAction,
    on_install: impl FnOnce(&ServiceDirArgs) -> Result<()>,
) -> Result<()> {
    match action {
        ServiceAction::Install(args) => {
            let mut args = args;
            args.install_dir = resolve_service_install_dir(service_name, &args.install_dir);
            on_install(&args)
        }
        ServiceAction::Uninstall(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            uninstall_in_dir(service_name, Some(dir)).context("uninstall failed")
        }
        ServiceAction::Status(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            status_in_dir(service_name, Some(dir)).context("status failed")
        }
        ServiceAction::Restart(args) => {
            let dir = resolve_service_install_dir(service_name, &args.install_dir);
            restart_in_dir(service_name, Some(dir)).context("restart failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 出厂模板的 port 行带 `# ← 说明` 尾注（93 实机踩到）：不剥注释时
    /// `8077   # ← 监听端口` 解析失败 → verify 误报"未解析到 server.port"
    #[test]
    fn read_server_port_strips_inline_comments_and_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.yml");

        std::fs::write(
            &cfg,
            "server:\n  host: \"0.0.0.0\"\n  port: 8077                       # ← 监听端口\n",
        )
        .unwrap();
        assert_eq!(read_server_port(&cfg), Some(8077), "裸值 + 尾注应解析成功");

        std::fs::write(&cfg, "server:\n  port: \"8088\"  # quoted\n").unwrap();
        assert_eq!(
            read_server_port(&cfg),
            Some(8088),
            "引号值 + 尾注应解析成功"
        );

        std::fs::write(&cfg, "server:\n  port: 8089\n").unwrap();
        assert_eq!(read_server_port(&cfg), Some(8089), "裸值无注释应解析成功");

        assert_eq!(read_server_port(&dir.path().join("nope.yml")), None);
    }
}
