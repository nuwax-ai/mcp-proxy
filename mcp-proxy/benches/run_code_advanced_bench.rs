use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mcp_stdio_proxy::{RunCodeMessageRequest, run_code_handler};
use serde_json::json;
use std::{collections::HashMap, fs};
use tokio::runtime::Runtime;
use uuid::Uuid;

// 测试脚本类型
#[derive(Debug, Clone, Copy)]
enum ScriptType {
    Js,
    Ts,
    Python,
}

// 测试场景
#[allow(dead_code)]
struct TestScenario {
    name: &'static str,
    file_path: &'static str,
    script_type: ScriptType,
    description: &'static str,
}

// 获取所有测试场景
fn get_test_scenarios() -> Vec<TestScenario> {
    vec![
        // 基本测试场景
        TestScenario {
            name: "js_basic",
            file_path: "fixtures/test_js.js",
            script_type: ScriptType::Js,
            description: "基本JavaScript执行",
        },
        TestScenario {
            name: "ts_basic",
            file_path: "fixtures/test_ts.ts",
            script_type: ScriptType::Ts,
            description: "基本TypeScript执行",
        },
        TestScenario {
            name: "python_basic",
            file_path: "fixtures/test_python_simple.py",
            script_type: ScriptType::Python,
            description: "基本Python执行",
        },
        // 参数传递测试场景
        TestScenario {
            name: "js_params",
            file_path: "fixtures/test_js_params.js",
            script_type: ScriptType::Js,
            description: "带参数的JavaScript执行",
        },
        TestScenario {
            name: "ts_params",
            file_path: "fixtures/test_ts_params.ts",
            script_type: ScriptType::Ts,
            description: "带参数的TypeScript执行",
        },
        TestScenario {
            name: "python_params",
            file_path: "fixtures/test_python_params.py",
            script_type: ScriptType::Python,
            description: "带参数的Python执行",
        },
        // 复杂测试场景
        TestScenario {
            name: "js_import",
            file_path: "fixtures/import_lodash_example.js",
            script_type: ScriptType::Js,
            description: "导入lodash的JavaScript执行",
        },
        TestScenario {
            name: "python_logging",
            file_path: "fixtures/test_python_logging.py",
            script_type: ScriptType::Python,
            description: "使用logging的Python执行",
        },
        TestScenario {
            name: "ts_complex",
            file_path: "fixtures/test_ts.ts",
            script_type: ScriptType::Ts,
            description: "复杂TypeScript执行",
        },
    ]
}

// 读取测试脚本文件
fn read_script_file(file_path: &str) -> String {
    fs::read_to_string(file_path).unwrap_or_else(|_| panic!("无法读取文件: {file_path}"))
}

// 创建运行代码请求
fn create_run_code_request(
    code: &str,
    script_type: ScriptType,
    with_complex_params: bool,
) -> RunCodeMessageRequest {
    let mut json_param = HashMap::new();

    if with_complex_params {
        // 创建复杂参数
        json_param.insert("input".to_string(), json!("测试输入"));
        json_param.insert("number".to_string(), json!(42));
        json_param.insert("boolean".to_string(), json!(true));
        json_param.insert("array".to_string(), json!([1, 2, 3, 4, 5]));
        json_param.insert(
            "object".to_string(),
            json!({
                "name": "测试对象",
                "properties": {
                    "a": 1,
                    "b": "string",
                    "c": [true, false]
                }
            }),
        );
    } else {
        // 创建简单参数
        json_param.insert("input".to_string(), json!("测试输入"));
    }

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
fn bench_run_code_advanced(c: &mut Criterion) {
    // 创建测试组
    let mut group = c.benchmark_group("run_code_handler_advanced");

    // 设置采样大小和预热次数
    group.sample_size(10); // 减少到10次取平均值
    group.warm_up_time(std::time::Duration::from_secs(20)); // 20秒预热时间
    group.measurement_time(std::time::Duration::from_secs(10)); // 10秒测量时间

    // 获取所有测试场景
    let scenarios = get_test_scenarios();

    // 创建tokio运行时
    let rt = Runtime::new().unwrap();

    // 遍历测试场景
    for scenario in scenarios {
        // 读取脚本内容
        let code = read_script_file(scenario.file_path);

        // 设置吞吐量计数为脚本字节大小
        // 这样可以比较不同大小脚本的执行效率
        group.throughput(Throughput::Bytes(code.len() as u64));

        // 测试使用简单参数
        group.bench_function(
            BenchmarkId::new(format!("{}_simple", scenario.name), "simple_params"),
            |b| {
                b.iter(|| {
                    let req = create_run_code_request(&code, scenario.script_type, false);
                    rt.block_on(async { run_code_handler(axum::Json(req)).await.unwrap() })
                });
            },
        );

        // 测试使用复杂参数
        group.bench_function(
            BenchmarkId::new(format!("{}_complex", scenario.name), "complex_params"),
            |b| {
                b.iter(|| {
                    let req = create_run_code_request(&code, scenario.script_type, true);
                    rt.block_on(async { run_code_handler(axum::Json(req)).await.unwrap() })
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05) // 设置显著性水平为5%
        .sample_size(10) // 设置样本大小为10
        .warm_up_time(std::time::Duration::from_secs(20)) // 20秒预热时间
        .measurement_time(std::time::Duration::from_secs(10)); // 10秒测量时间
    targets = bench_run_code_advanced
);
criterion_main!(benches);
