//! 语音转写页：选文件 → 选引擎（按钮组）→ 转写 → 结果复制/保存。

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, Render, SharedString, Styled, Window, div, px,
};

use crate::api::voice::{self, ENGINES, TranscriptionResponse};
use crate::services::manager::{Kind, ServiceManager, State};

enum Job {
    Idle,
    Busy { note: String },
}

pub struct TranscribePage {
    manager: Entity<ServiceManager>,
    file: Option<PathBuf>,
    engine: &'static str,
    job: Job,
    result: Option<Result<TranscriptionResponse, String>>,
}

impl TranscribePage {
    pub fn new(manager: Entity<ServiceManager>, cx: &mut Context<Self>) -> Self {
        // 服务状态（Running/Stopped）变化时刷新本页按钮可用性
        cx.observe(&manager, |_, _, cx| cx.notify()).detach();
        Self {
            manager,
            file: None,
            engine: "whisper",
            job: Job::Idle,
            result: None,
        }
    }

    fn voice_port(&self, cx: &Context<Self>) -> Option<u16> {
        match self.manager.read(cx).state(Kind::VoiceCli) {
            State::Running { port, .. } => Some(*port),
            _ => None,
        }
    }

    fn pick_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择音频/视频文件".into()),
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
        let Some(port) = self.voice_port(cx) else {
            return;
        };
        let Some(file) = self.file.clone() else {
            return;
        };
        let engine = self.engine;
        self.job = Job::Busy {
            note: "上传并转写中…".into(),
        };
        self.result = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    smol::unblock(move || voice::transcribe(port, &file, engine)).await
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.job = Job::Idle;
                this.result = Some(res.map_err(|e| format!("{e:#}")));
                cx.notify();
            });
        })
        .detach();
    }

    fn save_result(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ok(r)) = &self.result else {
            return;
        };
        let text = r.text.clone();
        let suggested = self
            .file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| format!("{}.txt", s.to_string_lossy()))
            .unwrap_or_else(|| "transcript.txt".into());
        let dir = self
            .file
            .as_ref()
            .and_then(|f| f.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let rx = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let written = std::fs::write(&path, &text).is_ok();
                _ = this.update(cx, |_this, cx| {
                    if written {
                        cx.notify();
                    }
                    let _ = &path; // 保存失败时静默（P0；后续可加 toast）
                });
            }
        })
        .detach();
    }
}

impl Render for TranscribePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port = self.voice_port(cx);
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
                    .child("语音转写"),
            )
            .when(port.is_none(), |this| {
                this.child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .p_4()
                        .text_sm()
                        .child("voice-cli 未运行——请先在左侧「服务状态」页安装并启动"),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("pick-file")
                            .label(
                                self.file
                                    .as_ref()
                                    .map(|f| {
                                        f.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "?".into())
                                    })
                                    .unwrap_or_else(|| "选择文件…".into()),
                            )
                            .small()
                            .disabled(busy || port.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_file(window, cx);
                            })),
                    )
                    .child(div().flex().gap_2().children(ENGINES.iter().map(|e| {
                        Button::new(SharedString::from(format!("engine-{e}")))
                            .label(*e)
                            .small()
                            .selected(self.engine == *e)
                            .disabled(busy || port.is_none())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.engine = e;
                                cx.notify();
                            }))
                    }))),
            )
            .child(
                Button::new("start-transcribe")
                    .label("开始转写")
                    .primary()
                    .disabled(busy || port.is_none() || self.file.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start(window, cx);
                    })),
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
                        .child(format!("转写失败：{msg}")),
                ),
                Some(Ok(r)) => this
                    .child(div().flex().items_center().gap_2().text_xs().child(format!(
                                "耗时 {:.1}s · 时长 {} · 语言 {}",
                                r.processing_time.unwrap_or(0.0),
                                r.duration
                                    .map(|d| format!("{d:.1}s"))
                                    .unwrap_or_else(|| "?".into()),
                                r.language.clone().unwrap_or_else(|| "?".into()),
                            )))
                    .child(
                        div().flex().gap_2().children([
                            Button::new("copy-result")
                                .label("复制全文")
                                .small()
                                .on_click({
                                    let text = r.text.clone();
                                    cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ));
                                    })
                                }),
                            Button::new("save-result")
                                .label("保存 .txt")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_result(window, cx);
                                })),
                        ]),
                    )
                    .child(result_view(r.text.clone())),
            })
    }
}

fn result_view(text: String) -> impl IntoElement {
    div()
        .id("transcript-result")
        .flex_1()
        .min_h(px(120.))
        .rounded_lg()
        .border_1()
        .p_4()
        .text_sm()
        .overflow_y_scrollbar()
        .child(text)
}
