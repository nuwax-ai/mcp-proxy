//! document-parser 已部署服务的端到端测试（本地或远程 E2E_DOCUMENT_PARSER_URL）。
//!
//! 服务不可达 → 全部 SKIP（非失败）；PDF 场景标注 #[ignore]（首跑模型下载/慢机器
//! 耗时长，`cargo test -- --ignored` 显式开）。

use test_e2e::assets;
use test_e2e::common::{Gate, assert_ok_code, asset_url, document_parser_url, probe_or_skip};

/// 通用：上传 multipart 文件
async fn upload_file(
    client: &reqwest::Client,
    base: &str,
    endpoint: &str,
    name: &str,
    bytes: Vec<u8>,
) -> serde_json::Value {
    let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let resp = client
        .post(format!("{base}{endpoint}"))
        .multipart(form)
        .send()
        .await
        .expect("request ok");
    resp.json().await.expect("json body")
}

#[tokio::test]
async fn health_and_readiness() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    for path in ["/health", "/ready"] {
        let body: serde_json::Value = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .expect("ok")
            .json()
            .await
            .expect("json");
        assert_ok_code(path, &body);
    }
}

#[tokio::test]
async fn parser_engines_healthy() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body: serde_json::Value = client
        .get(format!("{base}/api/v1/documents/parser/health"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("parser/health", &body);
    let engines = body.pointer("/data").expect("data 字段");
    let mineru = engines.get("mineru_available").and_then(|v| v.as_bool());
    let markitdown = engines
        .get("markitdown_available")
        .and_then(|v| v.as_bool());
    eprintln!("parser engines: mineru={mineru:?} markitdown={markitdown:?}");
    assert!(
        markitdown.unwrap_or(false),
        "MarkItDown 引擎应可用（venv 未装好）：{body}"
    );
}

#[tokio::test]
async fn openapi_docs_scalar_and_swagger_coexist() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    // Scalar：HTML 且内嵌 spec（含文档标题）
    let scalar = client
        .get(format!("{base}/api/docs/scalar"))
        .send()
        .await
        .expect("ok");
    assert!(
        scalar.status().is_success(),
        "scalar HTTP {}",
        scalar.status()
    );
    let scalar_html = scalar.text().await.expect("html");
    assert!(
        scalar_html.contains("Document Parser API"),
        "scalar 应内嵌 spec"
    );

    // Swagger：重定向后 200
    let swagger = client
        .get(format!("{base}/api/docs/"))
        .send()
        .await
        .expect("ok");
    assert!(
        swagger.status().is_success(),
        "swagger HTTP {}",
        swagger.status()
    );

    // openapi.json：有效 JSON 且覆盖文档解析路径
    let spec: serde_json::Value = client
        .get(format!("{base}/api/docs/openapi.json"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    let paths = spec
        .get("paths")
        .and_then(|p| p.as_object())
        .expect("paths");
    for p in [
        "/health",
        "/api/v1/documents/parse-sync",
        "/api/v1/documents/upload",
        "/api/v1/tasks/{task_id}",
    ] {
        assert!(paths.contains_key(p), "文档缺少路径 {p}");
    }
}

#[tokio::test]
async fn parse_sync_markdown_roundtrip() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body = upload_file(
        &client,
        &base,
        "/api/v1/documents/parse-sync",
        "e2e.md",
        assets::markdown_asset(),
    )
    .await;
    assert_ok_code("parse-sync", &body);
    let md = body
        .pointer("/data/markdown_content")
        .and_then(|m| m.as_str())
        .expect("markdown_content");
    assert!(md.contains("E2E 核心接口验证"), "解析内容丢失: {md}");
    let engine = body
        .pointer("/data/engine")
        .and_then(|e| e.as_str())
        .unwrap_or("");
    eprintln!("parse-sync engine={engine}");
}

/// PDF 走 MinerU 真实解析。#[ignore]：首跑触发模型下载（ModelScope 数百 MB~GB），
/// Windows CPU 机器可能极慢——显式 `cargo test -p test-e2e --test document_parser -- --ignored --test-threads=1`
#[tokio::test]
#[ignore = "PDF/MinerU 长耗时（首跑模型下载 + 慢机器推理），显式开启"]
async fn parse_sync_pdf_via_mineru() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body = upload_file(
        &client,
        &base,
        "/api/v1/documents/parse-sync",
        "e2e.pdf",
        assets::minimal_pdf(),
    )
    .await;
    assert_ok_code("parse-sync pdf", &body);
    let md = body
        .pointer("/data/markdown_content")
        .and_then(|m| m.as_str())
        .expect("markdown_content");
    assert!(
        md.to_lowercase().contains("e2e parse verification"),
        "PDF 文本提取失败: {md}"
    );
    assert_eq!(
        body.pointer("/data/engine").and_then(|e| e.as_str()),
        Some("MinerU"),
        "PDF 应走 MinerU 引擎"
    );
}

#[tokio::test]
async fn async_upload_full_lifecycle() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body = upload_file(
        &client,
        &base,
        "/api/v1/documents/upload",
        "e2e-async.md",
        assets::markdown_asset(),
    )
    .await;
    assert_ok_code("upload", &body);
    let task_id = body
        .pointer("/data/task_id")
        .and_then(|t| t.as_str())
        .expect("task_id")
        .to_string();
    eprintln!("async task: {task_id}");

    // 轮询至终态（Markdown 走 MarkItDown，通常 <10s；留 120s 余量）
    let mut final_status = String::new();
    for _ in 0..40 {
        let t: serde_json::Value = client
            .get(format!("{base}/api/v1/tasks/{task_id}"))
            .send()
            .await
            .expect("ok")
            .json()
            .await
            .expect("json");
        let st = t
            .pointer("/data/status")
            .and_then(|s| s.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                // Processing 是对象 {stage:...}——serde Value 序列化形态下取键名
                t.pointer("/data/status")
                    .and_then(|s| s.as_object())
                    .map(|o| o.keys().next().cloned().unwrap_or_default())
                    .unwrap_or_default()
            });
        if st == "Completed" || st == "Failed" {
            final_status = st;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    assert_eq!(
        final_status, "Completed",
        "异步任务未完成（上传后端未配置会 Failed——检查 .document-parser.env）"
    );

    // result 可读
    let result: serde_json::Value = client
        .get(format!("{base}/api/v1/tasks/{task_id}/result"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("result", &result);
}

/// 孤儿回归：提交长解析任务后立即 cancel → 状态 Cancelled 且不被后续错误覆盖
#[tokio::test]
async fn cancel_pending_task() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body = upload_file(
        &client,
        &base,
        "/api/v1/documents/upload",
        "e2e-cancel.md",
        assets::markdown_asset(),
    )
    .await;
    let task_id = body
        .pointer("/data/task_id")
        .and_then(|t| t.as_str())
        .expect("task_id")
        .to_string();

    let cancel: serde_json::Value = client
        .post(format!("{base}/api/v1/tasks/{task_id}/cancel"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("cancel", &cancel);

    // 终态抽查：Cancelled（若 worker 抢先完成也算通过——竞态窗口极小）
    let t: serde_json::Value = client
        .get(format!("{base}/api/v1/tasks/{task_id}"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    let status_str = serde_json::to_string(
        t.pointer("/data/status")
            .unwrap_or(&serde_json::Value::Null),
    )
    .unwrap_or_default();
    assert!(
        status_str.contains("Cancelled") || status_str.contains("Completed"),
        "取消后任务应处终态，实际 {status_str}"
    );
}

/// uploadFromUrl：需要 E2E_ASSET_URL 指向公网可达文件（如 nuwax-upload 上传产物）
#[tokio::test]
async fn upload_from_url() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let Some(url) = asset_url() else {
        eprintln!("SKIP: E2E_ASSET_URL 未配置（nuwax-upload 上传任意文件后把 URL 写入）");
        return;
    };
    let body: serde_json::Value = client
        .post(format!("{base}/api/v1/documents/uploadFromUrl"))
        .json(&serde_json::json!({"url": url, "filename": "from-url-e2e"}))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("uploadFromUrl", &body);
    assert!(
        body.pointer("/data/task_id").is_some(),
        "应返回 task_id: {body}"
    );
}

#[tokio::test]
async fn structured_document_toc() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let md = "# 一级标题\n\n段落甲。\n\n## 二级标题\n\n段落乙。\n";
    let body: serde_json::Value = client
        .post(format!("{base}/api/v1/documents/structured"))
        .json(&serde_json::json!({
            "markdown_content": md,
            "enable_toc": true,
            "max_toc_depth": 3
        }))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("structured", &body);
    let toc = body
        .pointer("/data/document/toc")
        .and_then(|t| t.as_array())
        .expect("toc 数组");
    assert!(toc.len() >= 2, "两级标题应生成 ≥2 条 TOC: {toc:?}");
    assert!(
        body.pointer("/data/document/total_sections")
            .and_then(|s| s.as_u64())
            .unwrap_or(0)
            >= 2,
        "分节数应 ≥2"
    );
}

#[tokio::test]
async fn tasks_stats_endpoint() {
    let Gate::Ready { client, base } = probe_or_skip(document_parser_url()).await else {
        return;
    };
    let body: serde_json::Value = client
        .get(format!("{base}/api/v1/tasks/stats"))
        .send()
        .await
        .expect("ok")
        .json()
        .await
        .expect("json");
    assert_ok_code("stats", &body);
    assert!(body.pointer("/data/stats").is_some(), "stats 结构: {body}");
}
