//! 服务状态页：两张服务卡片（状态机展示 + 安装/启停/重启/日志操作）。

use gpui_kit::component::{ActiveTheme, Sizable, button::Button};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement, Styled, div, px,
};

use crate::app::Workspace;
use crate::services::manager::{Kind, ServiceManager, State};

pub fn render(manager: &Entity<ServiceManager>, cx: &mut Context<Workspace>) -> AnyElement {
    let voice = card(Kind::VoiceCli, manager, cx);
    let doc = card(Kind::DocumentParser, manager, cx);
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
                .child("服务状态"),
        )
        .child(
            div()
                .text_sm()
                .child("两个服务按需安装，装好即随本客户端自动运行（仅本机 127.0.0.1）"),
        )
        .child(div().flex().gap_4().child(voice).child(doc))
        .into_any_element()
}

fn card(kind: Kind, manager: &Entity<ServiceManager>, cx: &mut Context<Workspace>) -> AnyElement {
    let state = manager.read(cx).state(kind).clone();

    let actions = match &state {
        State::NotInstalled => vec![button(
            ids(kind, "install"),
            "安装",
            manager,
            cx,
            move |m, cx| m.install(kind, cx),
        )],
        State::Stopped => vec![
            button(ids(kind, "start"), "启动", manager, cx, move |m, cx| {
                m.start(kind, cx)
            }),
            log_button(kind, manager, cx),
        ],
        State::Starting { .. } => vec![button(
            ids(kind, "stop"),
            "停止",
            manager,
            cx,
            move |m, cx| m.stop(kind, cx),
        )],
        State::Running { .. } => vec![
            button(ids(kind, "stop"), "停止", manager, cx, move |m, cx| {
                m.stop(kind, cx)
            }),
            button(ids(kind, "restart"), "重启", manager, cx, move |m, cx| {
                m.restart(kind, cx)
            }),
            log_button(kind, manager, cx),
        ],
        State::Failed { .. } => vec![
            button(ids(kind, "restart"), "重启", manager, cx, move |m, cx| {
                m.restart(kind, cx)
            }),
            log_button(kind, manager, cx),
        ],
        State::Provisioning { .. } => Vec::new(),
    };

    let note = match kind {
        Kind::VoiceCli => {
            if matches!(state, State::NotInstalled) {
                "首次安装约 0.5GB（small 档模型）"
            } else {
                ""
            }
        }
        Kind::DocumentParser => {
            if matches!(state, State::NotInstalled) {
                "首次安装约 1.4GB（Python 环境 + 解析模型）"
            } else {
                ""
            }
        }
    };

    div()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .p_5()
        .w(px(380.))
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(kind.title().to_string()),
        )
        .child(div().text_sm().child(state.label()))
        .when(!note.is_empty(), |this| {
            this.child(div().text_xs().child(note.to_string()))
        })
        .child(div().flex().gap_2().children(actions))
        .into_any_element()
}

fn button(
    id: &'static str,
    label: &'static str,
    manager: &Entity<ServiceManager>,
    cx: &mut Context<Workspace>,
    action: impl Fn(&mut ServiceManager, &mut Context<ServiceManager>) + 'static,
) -> Button {
    let manager = manager.clone();
    Button::new(id)
        .label(label)
        .small()
        .on_click(cx.listener(move |_, _, _, cx| {
            manager.update(cx, |m, cx| action(m, cx));
        }))
}

fn log_button(kind: Kind, manager: &Entity<ServiceManager>, cx: &mut Context<Workspace>) -> Button {
    let manager = manager.clone();
    Button::new(ids(kind, "logs"))
        .label("打开日志")
        .small()
        .on_click(cx.listener(move |_, _, _, cx| {
            manager.read(cx).open_log_dir(kind);
        }))
}

/// 状态页动作按钮的稳定 id（同一棵元素树内必须唯一）。
fn ids(kind: Kind, action: &'static str) -> &'static str {
    match (kind, action) {
        (Kind::VoiceCli, "install") => "vc-install",
        (Kind::VoiceCli, "start") => "vc-start",
        (Kind::VoiceCli, "stop") => "vc-stop",
        (Kind::VoiceCli, "restart") => "vc-restart",
        (Kind::VoiceCli, "logs") => "vc-logs",
        (Kind::DocumentParser, "install") => "dp-install",
        (Kind::DocumentParser, "start") => "dp-start",
        (Kind::DocumentParser, "stop") => "dp-stop",
        (Kind::DocumentParser, "restart") => "dp-restart",
        (Kind::DocumentParser, "logs") => "dp-logs",
        _ => "unknown",
    }
}
