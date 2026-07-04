# 性能基准测试

这个目录包含使用 Criterion.rs 对 `/api/run_code_with_log` 端点进行性能测试的代码。

## 可用的基准测试

1. `run_code_bench` - 基本性能测试，测试三种语言（JS、TS、Python）的简单脚本执行性能
2. `run_code_advanced_bench` - 高级性能测试，测试多种场景、不同复杂度的脚本和参数组合

## 如何运行测试

使用以下命令运行基本测试:

```bash
cargo bench --bench run_code_bench
```

使用以下命令运行高级测试:

```bash
cargo bench --bench run_code_advanced_bench
```

运行特定的测试场景:

```bash
cargo bench --bench run_code_advanced_bench -- js_basic
```

## 查看测试结果

测试结果将保存在 `target/criterion` 目录下，你可以在浏览器中打开 HTML 报告查看详细的结果:

```bash
open target/criterion/report/index.html
```

## 配置测试参数

在基准测试文件中可以修改以下参数，以适应不同性能的机器：

- `sample_size`: 每个测试场景运行的样本数量（默认为10）
- `warm_up_time`: 预热时间（默认为20秒）
- `measurement_time`: 测量时间（默认为10秒）
- `significance_level`: 统计显著性水平（默认为0.05，即5%）

如果在较慢的机器上运行，可能需要进一步减少样本数或增加测量时间。可以通过命令行参数指定：

```bash
# 使用命令行参数减少样本数
cargo bench --bench run_code_bench -- --sample-size 5

# 使用命令行参数增加预热时间
cargo bench --bench run_code_bench -- --warm-up-time 30

# 使用命令行参数增加测量时间
cargo bench --bench run_code_bench -- --measurement-time 15
```

## 测试脚本说明

测试使用了 `fixtures` 目录下的各种测试脚本:

- `test_js.js`、`test_ts.ts`、`test_python_simple.py` - 基本语言测试脚本
- `test_js_params.js`、`test_ts_params.ts`、`test_python_params.py` - 带参数的测试脚本
- `import_lodash_example.js`、`test_python_logging.py` - 复杂功能测试脚本