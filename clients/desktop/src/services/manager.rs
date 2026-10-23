//! ServiceManager：每服务一个状态机，驱动 provision（安装）与 supervisor（托管）。
//!
//! 桥接模式（gpui-kit stream-markdown 范例同款）：
//! 重活（下载/进程/HTTP 轮询）在 background executor 上跑，经 smol::channel 把
//! Msg 递给 cx.spawn 的消费任务回填 Entity 状态并 notify。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Context};
use smol::channel::{Sender, unbounded};

use super::paths;
use super::provision::{self, Progress};
use super::supervisor;

pub const DEFAULT_STT_TIER: &str = "small";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    VoiceCli,
    DocumentParser,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::VoiceCli => "voice-cli（语音转写/合成）",
            Kind::DocumentParser => "document-parser（文档解析）",
        }
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            Kind::VoiceCli => paths::VOICE_CLI,
            Kind::DocumentParser => paths::DOCUMENT_PARSER,
        }
    }

    fn install_dir(self) -> PathBuf {
        paths::service_install_dir(self.dir_name())
    }

    /// 已安装判定：setup 落过 config.yml 即算（幂等重装走状态页按钮）。
    pub fn is_installed(self) -> bool {
        self.install_dir().join("config.yml").exists()
    }

    /// 就绪探测端点：vc 用 /health（data 有 version）；dp 用 /ready（比 /health 严格）。
    fn health_path(self) -> &'static str {
        match self {
            Kind::VoiceCli => "/health",
            Kind::DocumentParser => "/ready",
        }
    }

    /// 未就绪容忍上限：dp 首启可能要现场建 venv（uv 下载+装 mineru，数分钟）。
    fn starting_timeout(self) -> Duration {
        match self {
            Kind::VoiceCli => Duration::from_secs(120),
            Kind::DocumentParser => Duration::from_secs(15 * 60),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    NotInstalled,
    Provisioning {
        detail: String,
        done: u64,
        total: Option<u64>,
    },
    Stopped,
    Starting {
        note: String,
    },
    Running {
        port: u16,
        version: Option<String>,
    },
    Failed {
        error: String,
    },
}

impl State {
    pub fn label(&self) -> String {
        match self {
            State::NotInstalled => "未安装".into(),
            State::Provisioning {
                detail,
                done,
                total,
            } => match total {
                Some(t) if *t > 0 => format!("安装中：{detail}（{done}/{t}）"),
                _ => format!("安装中：{detail}"),
            },
            State::Stopped => "已安装，未运行".into(),
            State::Starting { note } => format!("启动中：{note}"),
            State::Running { port, version } => match version {
                Some(v) => format!("运行中 · 127.0.0.1:{port} · {v}"),
                None => format!("运行中 · 127.0.0.1:{port}"),
            },
            State::Failed { error } => format!("失败：{error}"),
        }
    }
}

/// 后台 → UI 的消息。
enum Msg {
    State(Kind, State),
}

pub struct ServiceManager {
    voice_cli: State,
    document_parser: State,
    tx: Sender<Msg>,
    /// 每服务的监控任务取消旗标（stop 时置 true；监控循环 2s 粒度感知）。
    cancels: Vec<(Kind, Arc<AtomicBool>)>,
}

impl ServiceManager {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (tx, rx) = unbounded::<Msg>();
        let this = Self {
            voice_cli: State::NotInstalled,
            document_parser: State::NotInstalled,
            tx,
            cancels: Vec::new(),
        };

        // 消费任务：Msg → 状态字段 + notify（Workspace observe 本 Entity 联动重渲染）
        cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                _ = this.update(cx, |this, cx| {
                    this.apply(msg);
                    cx.notify();
                });
            }
        })
        .detach();

        this
    }

    /// 客户端启动时：已安装的服务自动拉起（设计 §1.4"打开即用"）。
    pub fn autostart_installed(&mut self, cx: &mut Context<Self>) {
        for kind in [Kind::VoiceCli, Kind::DocumentParser] {
            if kind.is_installed() {
                self.start(kind, cx);
            } else {
                self.set(kind, State::NotInstalled, cx);
            }
        }
    }

    pub fn state(&self, kind: Kind) -> &State {
        match kind {
            Kind::VoiceCli => &self.voice_cli,
            Kind::DocumentParser => &self.document_parser,
        }
    }

    fn set(&mut self, kind: Kind, state: State, cx: &mut Context<Self>) {
        match kind {
            Kind::VoiceCli => self.voice_cli = state,
            Kind::DocumentParser => self.document_parser = state,
        }
        cx.notify();
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::State(kind, state) => match kind {
                Kind::VoiceCli => self.voice_cli = state,
                Kind::DocumentParser => self.document_parser = state,
            },
        }
    }

    fn cancel_flag(&mut self, kind: Kind) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels.retain(|(k, _)| *k != kind);
        self.cancels.push((kind, flag.clone()));
        flag
    }

    fn raise_cancel(&mut self, kind: Kind) {
        for (k, flag) in &self.cancels {
            if *k == kind {
                flag.store(true, Ordering::SeqCst);
            }
        }
        self.cancels.retain(|(k, _)| *k != kind);
    }

    // ---- 动作（UI 按钮入口） ----

    /// 安装（按需）：provision 完成后自动进入托管。
    pub fn install(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let cancel = self.cancel_flag(kind);
        self.set(
            kind,
            State::Provisioning {
                detail: "准备安装".into(),
                done: 0,
                total: None,
            },
            cx,
        );
        let tx = self.tx.clone();
        cx.spawn(async move |_this, cx| {
            let tx_job = tx.clone();
            let result = cx
                .background_spawn(async move {
                    let tx_progress = tx_job;
                    let progress = move |p: Progress| {
                        // try_send：同步上下文直投，满了就丢帧（进度显示不追求完整）
                        _ = tx_progress.try_send(Msg::State(
                            kind,
                            State::Provisioning {
                                detail: p.detail,
                                done: p.done,
                                total: p.total,
                            },
                        ));
                    };
                    match kind {
                        Kind::VoiceCli => {
                            provision::provision_voice_cli(DEFAULT_STT_TIER, &progress)
                        }
                        Kind::DocumentParser => provision::provision_document_parser(&progress),
                    }
                })
                .await;
            match result {
                Ok(_install_dir) => {
                    let _ = tx
                        .send(Msg::State(
                            kind,
                            State::Starting {
                                note: "安装完成，首次启动".into(),
                            },
                        ))
                        .await;
                    supervise_forever(kind, tx, cancel).await;
                }
                Err(e) => {
                    _ = tx
                        .send(Msg::State(
                            kind,
                            State::Failed {
                                error: format!("{e:#}"),
                            },
                        ))
                        .await;
                }
            }
        })
        .detach();
    }

    /// 启动（已安装未运行）。
    pub fn start(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if !kind.is_installed() {
            return;
        }
        let cancel = self.cancel_flag(kind);
        self.set(
            kind,
            State::Starting {
                note: "拉起服务进程".into(),
            },
            cx,
        );
        let tx = self.tx.clone();
        cx.spawn(async move |_this, _cx| {
            supervise_forever(kind, tx, cancel).await;
        })
        .detach();
    }

    /// 停止（取消监控 + 终止进程组），幂等。
    pub fn stop(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.raise_cancel(kind);
        self.set(kind, State::Stopped, cx);
        let dir = kind.dir_name().to_string();
        cx.spawn(async move |_this, cx| {
            cx.background_spawn(async move {
                smol::unblock(move || {
                    _ = supervisor::stop_service(&dir);
                })
                .await
            })
            .await;
        })
        .detach();
    }

    /// 重启 = 停止后重新托管。
    pub fn restart(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.raise_cancel(kind);
        self.set(
            kind,
            State::Starting {
                note: "重启中".into(),
            },
            cx,
        );
        let cancel = self.cancel_flag(kind);
        let dir = kind.dir_name().to_string();
        let tx = self.tx.clone();
        cx.spawn(async move |_this, cx| {
            cx.background_spawn(async move {
                smol::unblock(move || {
                    _ = supervisor::stop_service(&dir);
                })
                .await
            })
            .await;
            supervise_forever(kind, tx, cancel).await;
        })
        .detach();
    }

    /// 在系统文件管理器里打开日志目录（目录存在才打开）。
    pub fn open_log_dir(&self, kind: Kind) {
        let dir = kind.install_dir().join("logs");
        if !dir.is_dir() {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            _ = std::process::Command::new("open").arg(&dir).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        }
    }
}

/// 托管主循环（每个服务一份，常驻直到 cancel）：
/// 随机端口 → 生成运行时配置 → spawn → 轮询健康；
/// 已就绪后持续失联 → 整组终止并重拉（退避），3 次后转 Failed 待人工。
async fn supervise_forever(kind: Kind, tx: Sender<Msg>, cancel: Arc<AtomicBool>) {
    let mut restarts: u32 = 0;
    'supervise: loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }

        let install_dir = kind.install_dir();
        let spawn_res = smol::unblock(move || -> anyhow::Result<u16> {
            let port = supervisor::pick_free_port()?;
            let config = match kind {
                Kind::VoiceCli => {
                    supervisor::VoiceCliRuntimeConfig::write(&install_dir, port, DEFAULT_STT_TIER)?
                }
                Kind::DocumentParser => supervisor::write_dp_runtime_config(&install_dir, port)?,
            };
            let bin = install_dir.join(deploy_installer::binary_name(kind.dir_name()));
            let argv = match kind {
                Kind::VoiceCli => supervisor::voice_cli_argv(&bin, &config),
                Kind::DocumentParser => supervisor::document_parser_argv(&bin, &config),
            };
            supervisor::spawn_service(kind.dir_name(), &argv, &install_dir)?;
            Ok(port)
        })
        .await;

        let port = match spawn_res {
            Ok(p) => p,
            Err(e) => {
                _ = tx
                    .send(Msg::State(
                        kind,
                        State::Failed {
                            error: format!("启动失败：{e:#}"),
                        },
                    ))
                    .await;
                return;
            }
        };

        let deadline = Instant::now() + kind.starting_timeout();
        let mut announced = false;
        let mut misses: u32 = 0;
        loop {
            smol::Timer::after(Duration::from_secs(2)).await;
            if cancel.load(Ordering::SeqCst) {
                let dir = kind.dir_name();
                _ = smol::unblock(move || supervisor::stop_service(dir)).await;
                return;
            }

            let path = kind.health_path();
            let ok = smol::unblock(move || supervisor::health_ok(port, path)).await;
            if ok {
                misses = 0;
                if !announced {
                    announced = true;
                    let version = match kind {
                        // dp 的 /health data 只是 "health" 字面量，无版本可显示
                        Kind::VoiceCli => {
                            smol::unblock(move || {
                                supervisor::fetch_health_data(port).ok().and_then(|d| {
                                    d.get("version")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string)
                                })
                            })
                            .await
                        }
                        Kind::DocumentParser => None,
                    };
                    _ = tx
                        .send(Msg::State(kind, State::Running { port, version }))
                        .await;
                }
            } else if announced {
                misses += 1;
                if misses >= 2 {
                    // ~4s 持续失联 → 整组终止重拉（dp 的 mineru 孙进程一并收掉）
                    restarts += 1;
                    if restarts > 3 {
                        _ = tx
                            .send(Msg::State(
                                kind,
                                State::Failed {
                                    error: "连续重启 3 次仍未恢复，等待人工处理".into(),
                                },
                            ))
                            .await;
                        let dir = kind.dir_name();
                        _ = smol::unblock(move || supervisor::stop_service(dir)).await;
                        return;
                    }
                    _ = tx
                        .send(Msg::State(
                            kind,
                            State::Starting {
                                note: format!("进程失联，第 {restarts} 次自动重启"),
                            },
                        ))
                        .await;
                    let dir = kind.dir_name();
                    _ = smol::unblock(move || supervisor::stop_service(dir)).await;
                    continue 'supervise;
                }
            } else if Instant::now() > deadline {
                // 一直未就绪：dp 首启建 venv 合法地久，超过容忍上限才判失败
                _ = tx
                    .send(Msg::State(
                        kind,
                        State::Failed {
                            error: format!(
                                "启动超时（{} 分钟内未就绪）",
                                kind.starting_timeout().as_secs() / 60
                            ),
                        },
                    ))
                    .await;
                let dir = kind.dir_name();
                _ = smol::unblock(move || supervisor::stop_service(dir)).await;
                return;
            }
        }
    }
}
