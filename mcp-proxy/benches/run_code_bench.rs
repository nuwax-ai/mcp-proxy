use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mcp_proxy::{RunCodeMessageRequest, run_code_handler};
use serde_json::json;
use std::{collections::HashMap, fs};
use tokio::runtime::Runtime;
use uuid::Uuid;

// 测试脚本类型枚举
enum ScriptType {
    Js,
    Ts,
    Python,
}

// 读取测试脚本文件
fn read_script_file(file_path: &str) -> String {
    fs::read_to_string(file_path).unwrap_or_else(|_| panic!("无法读取文件: {file_path}"))
}

// 创建运行代码请求
fn create_run_code_request(code: &str, script_type: ScriptType) -> RunCodeMessageRequest {
    let mut json_param = HashMap::new();
    json_param.insert("input".to_string(), json!("测试输入"));

    RunCodeMessageRequest {
        json_param,
        code: code.to_string(),
        uid: Uuid::new_v4().to_string(),
        engine_type: match script_type {
            ScriptType::Js => "js".to_string(),
            ScriptType::Ts => "ts".to_string(),
            ScriptType::Python => "python".to_string(),
        },
    }
}

// 设置基准测试
fn bench_run_code(c: &mut Criterion) {
    // 创建测试组
    let mut group = c.benchmark_group("run_code_handler");

    // 设置采样配置
    group.sample_size(10); // 减少样本数量
    group.warm_up_time(std::time::Duration::from_secs(20)); // 增加预热时间为20秒
    group.measurement_time(std::time::Duration::from_secs(10)); // 增加测量时间

    // 加载测试脚本
    let js_code = read_script_file("fixtures/test_js.js");
    let ts_code = read_script_file("fixtures/test_ts.ts");
    let py_code = read_script_file("fixtures/test_python_simple.py");

    // 创建运行时
    let rt = Runtime::new().unwrap();

    // 测试JS代码执行性能
    group.bench_function(BenchmarkId::new("javascript", "test_js.js"), |b| {
        b.iter(|| {
            let req = create_run_code_request(&js_code, ScriptType::Js);
            rt.block_on(async { run_code_handler(axum::Json(req)).await.unwrap() })
        });
    });

    // 测试TS代码执行性能
    group.bench_function(BenchmarkId::new("typescript", "test_ts.ts"), |b| {
        b.iter(|| {
            let req = create_run_code_request(&ts_code, ScriptType::Ts);
            rt.block_on(async { run_code_handler(axum::Json(req)).await.unwrap() })
        });
    });

    // 测试Python代码执行性能
    group.bench_function(BenchmarkId::new("python", "test_python_simple.py"), |b| {
        b.iter(|| {
            let req = create_run_code_request(&py_code, ScriptType::Python);
            rt.block_on(async { run_code_handler(axum::Json(req)).await.unwrap() })
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)  // 减少样本数量
        .warm_up_time(std::time::Duration::from_secs(20)) // 设置20秒的预热时间
        .measurement_time(std::time::Duration::from_secs(10)); // 设置更长的测量时间
    targets = bench_run_code
}
criterion_main!(benches);
