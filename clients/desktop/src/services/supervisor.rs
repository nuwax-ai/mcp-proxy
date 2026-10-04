//! 子进程托管：随机端口、运行时配置生成、spawn/停止、孤儿回收。
//!
//! 与服务注册（systemd/launchd/计划任务）无关——客户端进程就是服务的生命周期所有者：
//! spawn 出的子进程进独立进程组，客户端退出（含崩溃后重启的回收）时整组终止。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::paths;

/// 挑一个随机空闲端口（bind :0 让 OS 分配后释放；窗口期内被抢占的概率极低，
/// spawn 失败会走重启路径自愈）。
pub fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("bind ephemeral port")?;
    let port = listener.local_addr().context("read ephemeral port")?.port();
    drop(listener);
    Ok(port)
}

/// voice-cli 运行时配置：最小集 + 显式关 auto_download（代码默认 true，会在缺模型时
/// 从 HF 直拉——被墙环境必挂；模型由 provision 负责放好）。models_dir 用绝对路径
/// （服务侧按 CWD 解析相对路径，托管进程 CWD=安装目录但绝对路径消除歧义）。
#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceCliRuntimeConfig {
    pub server: ServerSection,
    pub whisper: WhisperSection,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerSection {
    pub host: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WhisperSection {
    pub default_model: String,
    pub models_dir: PathBuf,
    pub auto_download: bool,
}

impl VoiceCliRuntimeConfig {
    pub fn write(install_dir: &Path, port: u16, model_tier: &str) -> Result<PathBuf> {
        let config = Self {
            server: ServerSection {
                host: "127.0.0.1".into(),
                port,
            },
            whisper: WhisperSection {
                default_model: model_tier.into(),
                models_dir: install_dir.join("models"),
                auto_download: false,
            },
        };
        // .yml 后缀按 YAML 解析——必须用 serde_yaml 序列化（首版误用 toml 导致服务起进程即退）
        let path = install_dir.join("config.runtime.yml");
        std::fs::write(&path, serde_yaml::to_string(&config)?)
            .with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }
}

/// voice-cli argv：`server run --config <p>`（--config 必须在 server run 之后——
/// 全局 -c 对 server 子命令不生效，见 voice-cli main.rs 的分派逻辑）。
pub fn voice_cli_argv(bin: &Path, config: &Path) -> Vec<String> {
    vec![
        bin.to_string_lossy().into_owned(),
        "server".into(),
        "run".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
    ]
}

/// document-parser argv：`--config <p> server`（--config 在 server 之前，与 vc 相反）。
pub fn document_parser_argv(bin: &Path, config: &Path) -> Vec<String> {
    vec![
        bin.to_string_lossy().into_owned(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "server".into(),
    ]
}

/// spawn 服务子进程（独立进程组 + 输出丢弃——服务自己按 tracing-appender 轮转日志）。
/// 返回子进程 pid（同时写入 pidfile）。
pub fn spawn_service(service: &str, argv: &[String], install_dir: &Path) -> Result<u32> {
    let program = &argv[0];
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
        .current_dir(install_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：组信号可以整组终止（dp 的 mineru 是孙进程，也要一并收掉）
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {} ({})", service, program))?;
    let pid = child.id();
    // 托管循环按 /health 轮询判断存活，不依赖 Child 退出状态——detach（drop handle），
    // pidfile 是停止/回收的凭据。
    drop(child);

    let pid_path = paths::pid_file(service);
    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&pid_path, pid.to_string())
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(pid)
}

/// 终止服务（幂等）：pidfile 不存在/进程已死都视为已停止。
/// Unix：kill 前用 `ps` 验证 pid 确属本服务（防 pid 复用误杀）；先 TERM 后 KILL。
pub fn stop_service(service: &str) -> Result<()> {
    let pid_path = paths::pid_file(service);
    let Some(pid_str) = std::fs::read_to_string(&pid_path).ok() else {
        return Ok(());
    };
    let Ok(pid) = pid_str.trim().parse::<i32>() else {
        let _ = std::fs::remove_file(&pid_path);
        return Ok(());
    };

    #[cfg(unix)]
    {
        if process_matches_service(pid, service) {
            let pgid = -pid; // 负 pid = 进程组
            signal(pgid, libc::SIGTERM);
            for _ in 0..20 {
                if signal(pid, 0) != 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if signal(pid, 0) == 0 {
                signal(pgid, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }

    std::fs::remove_file(&pid_path).ok();
    Ok(())
}

/// 客户端启动时回收陈旧 pid（上次崩溃/强杀残留的子进程）。
pub fn reap_orphans(service: &str) {
    let _ = stop_service(service);
}

#[cfg(unix)]
fn signal(pid_or_pgid: i32, sig: i32) -> i32 {
    // FFI 边界调用 libc::kill（signal 0 为存在性探测）
    unsafe { libc::kill(pid_or_pgid, sig) }
}

#[cfg(unix)]
fn process_matches_service(pid: i32, service: &str) -> bool {
    // /proc 只在 Linux 有；macOS 走 ps。ps 输出整条命令行，匹配服务名即可。
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|o| {
            let cmd = String::from_utf8_lossy(&o.stdout);
            cmd.split_whitespace().any(|w| w.contains(service))
        })
        .unwrap_or(false)
}

/// 健康检查：HttpResult 信封 code=="0000" 判就绪（voice-cli 用 /health，dp 用 /ready）。
#[derive(Deserialize, Debug)]
struct HttpEnvelope {
    code: String,
    #[allow(dead_code)]
    message: String,
}

pub fn health_ok(port: u16, path: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}{path}");
    let Ok(resp) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .and_then(|c| c.get(&url).send())
    else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(text) = resp.text() else {
        return false;
    };
    matches!(
        serde_json::from_str::<HttpEnvelope>(&text),
        Ok(env) if env.code == "0000"
    )
}

/// 服务端 /health 的 data 段（状态页展示 version / models_loaded 用）。
pub fn fetch_health_data(port: u16) -> Result<serde_json::Value> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("build http client")?
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .context("GET /health")?;
    let text = resp.text().context("read /health body")?;
    let v: serde_json::Value = serde_json::from_str(&text).context("parse /health json")?;
    match v.get("code").and_then(|c| c.as_str()) {
        Some("0000") => Ok(v.get("data").cloned().unwrap_or(serde_json::Value::Null)),
        Some(code) => bail!(
            "/health code={code}: {}",
            v["message"].as_str().unwrap_or("")
        ),
        None => bail!("/health 响应缺少 code 字段"),
    }
}

/// dp 运行时配置：以 setup 落好的 config.yml 为底，仅改 server.port（随机）。
/// 模板的占位桶/占位密钥形态恰好满足服务的非空校验，无 OSS 凭证也能启动。
pub fn write_dp_runtime_config(
    install_dir: &std::path::Path,
    port: u16,
) -> anyhow::Result<PathBuf> {
    let base = install_dir.join("config.yml");
    let text = std::fs::read_to_string(&base)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", base.display()))?;
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| anyhow::anyhow!("parse config.yml: {e}"))?;
    value["server"]["port"] = serde_yaml::Value::Number(port.into());
    let out = install_dir.join("config.runtime.yml");
    std::fs::write(&out, serde_yaml::to_string(&value)?)
        .map_err(|e| anyhow::anyhow!("write {}: {e}", out.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防回归：runtime 配置是 .yml 文件，必须是 YAML 语法——首版误用 toml 序列化，
    /// 服务按 YAML 解析失败、起进程即退（90s 健康等待全超时才暴露）。
    #[test]
    fn runtime_config_is_yaml_not_toml() {
        let dir = std::env::temp_dir().join(format!("ai-toolbox-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let path = VoiceCliRuntimeConfig::write(&dir, 59999, "small").expect("write config");
        let text = std::fs::read_to_string(&path).expect("read config");
        assert!(
            text.starts_with("server:"),
            "config.runtime.yml 不是 YAML：{text}"
        );
        assert!(!text.contains("[server]"), "疑似 TOML 语法：{text}");
        let parsed: VoiceCliRuntimeConfig = serde_yaml::from_str(&text).expect("roundtrip parse");
        assert_eq!(parsed.server.port, 59999);
        assert_eq!(parsed.whisper.default_model, "small");
        assert!(!parsed.whisper.auto_download);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn free_port_is_bindable() {
        let port = pick_free_port().expect("pick");
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("rebind");
        assert_eq!(listener.local_addr().expect("addr").port(), port);
    }
}
