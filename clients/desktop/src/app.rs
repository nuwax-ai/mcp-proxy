//! 应用根视图：左侧 Sidebar 导航 + 右侧内容区（按当前页切换）。

use crate::pages;
use crate::services::manager::ServiceManager;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
    },
};
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement, Render, Styled,
    Window, div, px,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Status,
    Transcribe,
    Document,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Status => "服务状态",
            Page::Transcribe => "语音转写",
            Page::Document => "文档解析",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::Status => IconName::MonitorCog,
            Page::Transcribe => IconName::AudioLines,
            Page::Document => IconName::FileText,
        }
    }
}

pub struct Workspace {
    page: Page,
    manager: Entity<ServiceManager>,
    transcribe: Entity<pages::transcribe::TranscribePage>,
    document: Entity<pages::document::DocumentPage>,
}

impl Workspace {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let manager = cx.new(ServiceManager::new);
        // 状态页实时性：manager 一变（进度/状态机推进）Workspace 重渲染
        cx.observe(&manager, |_, _, cx| cx.notify()).detach();
        let transcribe = cx.new(|cx| pages::transcribe::TranscribePage::new(manager.clone(), cx));
        let document = cx.new(|cx| pages::document::DocumentPage::new(manager.clone(), cx));
        manager.update(cx, |m, cx| m.autostart_installed(cx));
        Self {
            page: Page::Status,
            manager,
            transcribe,
            document,
        }
    }

    fn nav_item(&self, page: Page, cx: &mut Context<Self>) -> SidebarMenuItem {
        SidebarMenuItem::new(page.label())
            .icon(page.icon())
            .active(self.page == page)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.page = page;
                cx.notify();
            }))
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: gpui_kit::AnyElement = match self.page {
            Page::Status => pages::status::render(&self.manager, cx),
            Page::Transcribe => self.transcribe.clone().into_any_element(),
            Page::Document => self.document.clone().into_any_element(),
        };

        div()
            .size_full()
            .bg(cx.theme().background)
            .child(
                Sidebar::new("app-sidebar")
                    .collapsible(SidebarCollapsible::Icon)
                    .w(px(220.))
                    .header(
                        SidebarHeader::new().child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .size_7()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().sidebar_primary)
                                        .text_color(cx.theme().sidebar_primary_foreground)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Icon::new(IconName::Bot)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("AI 工具箱"),
                                ),
                        ),
                    )
                    .child(
                        SidebarGroup::new("服务").child(SidebarMenu::new().children([
                            self.nav_item(Page::Status, cx),
                            self.nav_item(Page::Transcribe, cx),
                            self.nav_item(Page::Document, cx),
                        ])),
                    ),
            )
            .child(div().flex_1().min_w_0().size_full().child(content))
    }
}

/// appdata 根目录：mac ~/Library/Application Support/ai-toolbox 等（dirs 平台语义）。
pub fn app_data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("ai-toolbox")
}
