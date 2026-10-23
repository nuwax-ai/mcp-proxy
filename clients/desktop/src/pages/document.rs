//! 文档解析页：选文档 → parse-sync → Markdown 渲染（TextView）+ 导出 .md。

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, Styled, Window, div, px,
};

use crate::api::document::{self, SyncParseResponse};
use crate::services::manager::{Kind, ServiceManager, State};

enum Job {
    Idle,
    Busy { note: String },
}

pub struct DocumentPage {
    manager: Entity<ServiceManager>,
    file: Option<PathBuf>,
    job: Job,
    result: Option<Result<SyncParseResponse, String>>,
    markdown: Option<Entity<TextViewState>>,
}

impl DocumentPage {
    pub fn new(manager: Entity<ServiceManager>, cx: &mut Context<Self>) -> Self {
        cx.observe(&manager, |_, _, cx| cx.notify()).detach();
        Self {
            manager,
            file: None,
            job: Job::Idle,
            result: None,
            markdown: None,
        }
    }

    fn dp_port(&self, cx: &Context<Self>) -> Option<u16> {
        match self.manager.read(cx).state(Kind::DocumentParser) {
            State::Running { port, .. } => Some(*port),
            _ => None,
        }
    }

    fn pick_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择文档（PDF/Word/Excel/PPT/Markdown…）".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                _ = this.update(cx, |this, cx| {
                    this.file = paths.into_iter().next();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(port) = self.dp_port(cx) else {
            return;
        };
        let Some(file) = self.file.clone() else {
            return;
        };
        self.job = Job::Busy {
            note: "解析中（大文件可达数分钟）…".into(),
        };
        self.result = None;
        self.markdown = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    smol::unblock(move || document::parse_sync(port, &file)).await
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.job = Job::Idle;
                match res {
                    Ok(r) => {
                        // parse-sync 不传 upload_* 时图片引用是服务端本地路径（不可达）——
                        // P0 定位为快速文本预览；带图完整体验在 P1 异步任务流
                        let src = r.markdown_content.clone();
                        this.markdown = Some(cx.new(|cx| TextViewState::markdown(&src, cx)));
                        this.result = Some(Ok(r));
                    }
                    Err(e) => this.result = Some(Err(format!("{e:#}"))),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_markdown(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ok(r)) = &self.result else {
            return;
        };
        let text = r.markdown_content.clone();
        let suggested = self
            .file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| format!("{}.md", s.to_string_lossy()))
            .unwrap_or_else(|| "parsed.md".into());
        let dir = self
            .file
            .as_ref()
            .and_then(|f| f.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let rx = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                _ = std::fs::write(&path, &text);
                _ = this.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();
    }
}

impl Render for DocumentPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port = self.dp_port(cx);
        let busy = matches!(self.job, Job::Busy { .. });

        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child("文档解析"),
            )
            .when(port.is_none(), |this| {
                this.child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .p_4()
                        .text_sm()
                        .child(
                            "document-parser 未运行——请先在左侧「服务状态」页安装并启动 \
                             （首次安装含 Python 环境与解析模型，约 1.4GB）",
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("pick-doc")
                            .label(
                                self.file
                                    .as_ref()
                                    .map(|f| {
                                        f.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "?".into())
                                    })
                                    .unwrap_or_else(|| "选择文档…".into()),
                            )
                            .small()
                            .disabled(busy || port.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_file(window, cx);
                            })),
                    )
                    .child(
                        Button::new("start-parse")
                            .label("开始解析")
                            .primary()
                            .disabled(busy || port.is_none() || self.file.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start(window, cx);
                            })),
                    ),
            )
            .when(busy, |this| match &self.job {
                Job::Busy { note } => this.child(div().text_sm().child(note.clone())),
                Job::Idle => this,
            })
            .map(|this| match &self.result {
                None => this,
                Some(Err(msg)) => this.child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().danger)
                        .p_4()
                        .text_sm()
                        .child(format!("解析失败：{msg}")),
                ),
                Some(Ok(r)) => this
                    .child(div().flex().items_center().gap_2().text_xs().child(format!(
                        "{} · {} · {} 引擎 · {:.1}s · {} 字",
                        r.filename.as_deref().unwrap_or("?"),
                        r.format.as_deref().unwrap_or("?"),
                        r.engine.as_deref().unwrap_or("?"),
                        r.processing_time_ms.unwrap_or(0) as f64 / 1000.0,
                        r.word_count.unwrap_or(0),
                    )))
                    .child(
                        Button::new("save-md")
                            .label("导出 .md")
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_markdown(window, cx);
                            })),
                    )
                    .when_some(self.markdown.clone(), |this, state| {
                        this.child(
                            div()
                                .id("markdown-result")
                                .flex_1()
                                .min_h(px(160.))
                                .rounded_lg()
                                .border_1()
                                .p_4()
                                .overflow_y_scrollbar()
                                .child(TextView::new(&state).selectable(true)),
                        )
                    }),
            })
    }
}
