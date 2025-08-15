use axum::{extract::{Path, State}, Json};
use serde::Serialize;
use crate::app_state::AppState;
use crate::models::{HttpResult, StructuredSection};

#[derive(Debug, Serialize)]
pub struct TocResponse {
    pub task_id: String,
    pub toc: Vec<StructuredSection>,
    pub total_sections: usize,
}

#[derive(Debug, Serialize)]
pub struct SectionResponse {
    pub section_id: String,
    pub title: String,
    pub content: String,
    pub level: u8,
    pub has_children: bool,
}

#[derive(Debug, Serialize)]
pub struct SectionsResponse {
    pub task_id: String,
    pub document_title: String,
    pub toc: Vec<StructuredSection>,
    pub total_sections: usize,
}

pub async fn get_document_toc(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<HttpResult<TocResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                let resp = TocResponse {
                    task_id: task_id.clone(),
                    toc: doc.toc.clone(),
                    total_sections: doc.total_sections,
                };
                Json(HttpResult::success(resp))
            } else {
                Json(HttpResult::<TocResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<TocResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<TocResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}

pub async fn get_section_content(
    State(state): State<AppState>,
    Path((task_id, section_id)): Path<(String, String)>,
) -> Json<HttpResult<SectionResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                // 遍历 toc 查找 section 元信息
                fn find<'a>(items: &'a [StructuredSection], id: &str) -> Option<&'a StructuredSection> {
                    for it in items {
                        if it.id == id { return Some(it); }
                        if let Some(found) = find(&it.children, id) { return Some(found); }
                    }
                    None
                }
                if let Some(toc_item) = find(&doc.toc, &section_id) {
                    let content = toc_item.content.clone();
                    let resp = SectionResponse {
                        section_id: section_id.clone(),
                        title: toc_item.title.clone(),
                        content,
                        level: toc_item.level,
                        has_children: !toc_item.children.is_empty(),
                    };
                    Json(HttpResult::success(resp))
                } else {
                    Json(HttpResult::<SectionResponse>::error(
                        "T010".to_string(),
                        format!("章节不存在: {section_id}"),
                    ))
                }
            } else {
                Json(HttpResult::<SectionResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<SectionResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<SectionResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}

pub async fn get_all_sections(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Json<HttpResult<SectionsResponse>> {
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            if let Some(doc) = task.structured_document {
                let resp = SectionsResponse {
                    task_id: task_id.clone(),
                    document_title: doc.document_title.clone(),
                    toc: doc.toc.clone(),
                    total_sections: doc.total_sections,
                };
                Json(HttpResult::success(resp))
            } else {
                Json(HttpResult::<SectionsResponse>::error(
                    "T009".to_string(),
                    "任务尚未生成结构化文档".to_string(),
                ))
            }
        }
        Ok(None) => Json(HttpResult::<SectionsResponse>::error(
            "T002".to_string(),
            format!("任务不存在: {task_id}"),
        )),
        Err(e) => Json(HttpResult::<SectionsResponse>::error(
            "T003".to_string(),
            format!("查询任务失败: {e}"),
        )),
    }
}


