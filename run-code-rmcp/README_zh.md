# MCP 代码执行工具

一个基于 Rust 的工具，提供命令行界面和 MCP（Model Context Protocol）服务器，用于在隔离环境中执行 JavaScript、TypeScript 和 Python 代码。

**项目仓库：** <https://github.com/nuwax-ai/run_code_rmcp>

[English](./README.md) | 中文

## 功能特点

- 在隔离环境中执行 JavaScript、TypeScript 和 Python 代码
- **JavaScript/TypeScript**：由 Deno 运行时驱动
- **Python**：由 `uv` 驱动，支持自动依赖管理
- 使用 blake3 哈希自动缓存代码，加快重复执行速度
- 支持 JavaScript 的 ESM 和 CommonJS 模块
- 自动解析和安装 Python 依赖
- 独立的日志捕获和执行结果处理
- 提供命令行工具（`run_code_rmcp`）和 MCP 服务器（`script_runner`）两种使用方式

## 安装方法

### 安装命令行工具

```bash
# 克隆仓库
git clone https://github.com/nuwax-ai/run_code_rmcp.git
cd run_code_rmcp

# 安装命令行工具 (run_code_rmcp)
cargo install --path .

# 或同时安装命令行工具和 MCP 服务器
cargo install --path . --features mcp
cargo install --path . --bin script_runner --features mcp
```

### 系统要求

- **Rust**：1.85 或更高版本（用于构建）
- **Deno**：JavaScript/TypeScript 执行必需
- **uv**：Python 执行和依赖管理必需

安装 Deno：
```bash
curl -fsSL https://deno.land/install.sh | sh
```

安装 uv：
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

## 使用方法

### 命令行工具：run_code_rmcp

`run_code_rmcp` 命令允许直接从命令行执行代码。

#### 基本语法

```bash
run_code_rmcp [选项] <命令>
```

**选项：**
- `--show-logs`：显示执行日志
- `--clear-cache`：执行前清除缓存
- `--use-mcp`：使用 MCP SDK 集成（需要 mcp feature）

**命令：**
- `js`：执行 JavaScript 代码
- `ts`：执行 TypeScript 代码
- `python`：执行 Python 代码
- `clear-cache`：清除缓存文件

#### 执行 JavaScript/TypeScript

```bash
# 执行 JavaScript 文件
run_code_rmcp --show-logs js -f fixtures/test_js.js

# 带参数执行
run_code_rmcp --show-logs js -f fixtures/test_js_params.js -p '{"name":"User"}'

# 执行 TypeScript
run_code_rmcp --show-logs ts -f fixtures/test_ts.ts

# 执行内联代码
run_code_rmcp js -c "function handler() { return 'Hello!'; }"
```

#### 执行 Python

```bash
# 执行 Python 文件
run_code_rmcp --show-logs python -f fixtures/test_python.py

# 带参数执行
run_code_rmcp --show-logs python -f fixtures/test_python_params.py -p '{"a":10, "b":20}'

# 执行内联代码
run_code_rmcp python -c "def handler(): return 'Hello from Python!'"
```

#### 缓存管理

```bash
# 清除所有缓存
run_code_rmcp clear-cache --language all

# 清除特定语言的缓存
run_code_rmcp clear-cache --language python
run_code_rmcp clear-cache --language js
run_code_rmcp clear-cache --language ts

# 执行前清除缓存
run_code_rmcp --clear-cache js -f fixtures/test_js.js
```

### 作为 Rust 库使用

```toml
[dependencies]
run_code_rmcp = { git = "https://github.com/nuwax-ai/run_code_rmcp.git" }
```

```rust
use run_code_rmcp::{CodeExecutor, LanguageScript};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 执行 JavaScript
    let js_result = CodeExecutor::execute_with_params_compat(
        "function handler() { return {success: true}; }",
        LanguageScript::Js,
        None
    ).await?;

    // 执行 Python
    let py_result = CodeExecutor::execute_with_params_compat(
        "def handler(): return {'success': True}",
        LanguageScript::Python,
        None
    ).await?;

    Ok(())
}
```

### MCP 服务器：script_runner

`script_runner` 是一个 MCP 服务器，为 AI 助手提供代码执行工具。

#### 启动服务器

```bash
# 启动 MCP 服务器
script_runner

# 启用详细日志
script_runner --verbose
```

服务器通过 stdio 通信，等待 MCP 协议的 JSON 请求。

#### 可用工具

MCP 服务器提供以下工具：

1. **run_javascript** - 执行 JavaScript 代码
2. **run_typescript** - 执行 TypeScript 代码
3. **run_python** - 执行 Python 代码

每个工具接受：
- `code`（字符串）：要执行的代码
- `params`（对象，可选）：传递给 handler 函数的参数

#### 与 AI 助手集成

添加到你的 MCP 客户端配置（如 Claude Desktop）：

```json
{
  "name": "script-runner",
  "command": "script_runner",
  "transport": "stdio"
}
```

## 代码结构

### Handler 函数

脚本应定义一个返回结果的 handler 函数。

#### JavaScript/TypeScript

支持 `main` 和 `handler` 函数（main 优先）：

```javascript
// 这个函数优先级更高
function main(input) {
    console.log("使用 main 处理:", input);
    return { result: "Hello from main!" };
}

// 如果没有定义 main，则使用这个函数
function handler(input) {
    console.log("使用 handler 处理:", input);
    return { result: "Hello from handler!" };
}
```

也支持 ESM 模块：

```javascript
import { serve } from "https://deno.land/std/http/server.ts";

function handler(input) {
    return { message: "ESM works!" };
}
```

#### Python

支持 `handler` 和 `main` 函数（handler 优先）：

```python
import pandas as pd

# 这个函数优先级更高
def handler(args):
    print(f"处理中: {args}")
    data = pd.DataFrame({"a": [1, 2, 3]})
    return {"result": data.to_dict()}

# 如果没有定义 handler，则使用这个函数
def main(args):
    return {"result": "使用 main 代替"}
```

### 参数传递

参数会传递给 handler 函数：

```bash
run_code_rmcp js -f script.js -p '{"name": "User", "count": 42}'
```

在代码中：
```javascript
function handler(input) {
    // input = { name: "User", count: 42 }
    return `Hello ${input.name}, count: ${input.count}`;
}
```

### Python 依赖

Python 依赖会被自动检测并安装：

```python
import requests
import pandas as pd

def handler(args):
    # 依赖会通过 uv 自动安装
    response = requests.get("https://api.example.com")
    data = pd.DataFrame(response.json())
    return data.to_dict()
```

### 日志捕获

所有控制台输出都会与返回值分开捕获：

```javascript
console.log("这是日志");
console.log("这也是日志");

function handler() {
    return "这是结果";
}
```

结果：
```json
{
  "logs": ["这是日志", "这也是日志"],
  "result": "这是结果",
  "error": null
}
```

## 开发

```bash
# 构建
cargo build

# 使用 cargo 运行
cargo run -- --show-logs js -f fixtures/test_js.js

# 运行测试
cargo test

# 使用 mcp feature 构建
cargo build --features mcp

# 使用 cargo 运行 MCP 服务器
cargo run --bin script_runner --features mcp
```

## 环境预热

项目包含预热函数来缓存常用依赖：

```rust
use run_code_rmcp::warm_up_all_envs;

#[tokio::main]
async fn main() -> Result<()> {
    // 预安装常用的 Python 和 JS/TS 依赖
    warm_up_all_envs(None, None, None, None).await?;
    Ok(())
}
```

## 待办事项

- [ ] TTS（文本转语音）功能 - 目前存在已知问题

## 许可证

本项目采用 Apache License 2.0 发布。详见根目录的 `LICENSE` 与 `NOTICE` 文件。
