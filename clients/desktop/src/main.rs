mod api;
mod app;
mod pages;
mod services;
mod single_instance;
mod smoke;

use anyhow::{Context, Result};
use gpui_kit::{AppContext as _, WindowBounds, WindowOptions, px, size};

fn main() -> Result<()> {
    let data_dir = app::app_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create app data dir {}", data_dir.display()))?;

    // 单实例：OS 文件锁，进程退出/崩溃自动释放；抢不到说明已有实例在跑
    let _instance = single_instance::acquire(&data_dir)
        .context("ai-toolbox 已在运行（单实例限制，请使用已打开的窗口）")?;

    // 回收上次崩溃/强杀可能残留的服务子进程（幂等：pidfile 不存在即 no-op）
    services::supervisor::reap_orphans(services::paths::VOICE_CLI);
    services::supervisor::reap_orphans(services::paths::DOCUMENT_PARSER);

    // 无头自检：完整走一遍 provision → spawn → 健康检查 → 停止
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        return smoke::run();
    }
    if let Some(i) = args.iter().position(|a| a == "--smoke-dp") {
        let pdf = args
            .get(i + 1)
            .filter(|s| !s.starts_with('-'))
            .map(std::path::PathBuf::from);
        return smoke::run_dp(pdf.as_deref());
    }

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            // 注册 gpui 生态 HTTP 客户端：Markdown 渲染的远程图片（P1 异步流的 OSS 图）经它加载
            match reqwest_client::ReqwestClient::user_agent("ai-toolbox") {
                Ok(client) => cx.set_http_client(std::sync::Arc::new(client)),
                Err(e) => {
                    eprintln!("failed to init http client: {e}");
                    std::process::exit(1);
                }
            }
            let options = WindowOptions {
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("AI 工具箱".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
                ..Default::default()
            };
            if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::Workspace::new(window, cx))
            }) {
                eprintln!("failed to open window: {e}");
                std::process::exit(1);
            }
            // 退出清理：托管的服务子进程整组终止（不给用户留后台进程）
            cx.on_app_quit(|_cx| async move {
                smol::unblock(|| {
                    services::supervisor::stop_service(services::paths::VOICE_CLI).ok();
                    services::supervisor::stop_service(services::paths::DOCUMENT_PARSER).ok();
                })
                .await;
            })
            .detach();
        });

    Ok(())
}
