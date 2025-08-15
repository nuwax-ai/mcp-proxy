# Document Parser API 测试指南

本指南提供了完整的API测试方法和示例，帮助你快速测试文档解析服务的核心功能。

## 📁 测试文件说明

- `test_api.rest` - REST API测试文件，包含所有API端点的测试用例
- `test_server.sh` - 服务器启动脚本
- `test_sample.md` - 示例Markdown文档，用于测试解析功能
- `TEST_README.md` - 本测试指南

## 🚀 快速开始

### 1. 环境初始化 (首次使用)

```bash
# 初始化虚拟环境和Python依赖
document-parser uv-init

# 检查环境状态
document-parser check
```

### 2. 启动服务器

```bash
# 方法1：使用测试脚本
./test_server.sh

# 方法2：直接运行
cargo run --bin document-parser

# 方法3：使用已编译的二进制文件
document-parser server
```

服务器将在 `http://localhost:8087` 启动。

### 3. 虚拟环境管理

**激活虚拟环境 (可选):**
```bash
# Linux/macOS
source ./venv/bin/activate

# Windows
.\venv\Scripts\activate
```

**验证环境:**
```bash
# 检查Python和依赖
./venv/bin/python --version      # Linux/macOS
.\venv\Scripts\python --version  # Windows

# 测试MinerU
./venv/bin/mineru --help         # Linux/macOS
.\venv\Scripts\mineru --help     # Windows
```

### 2. 基础健康检查

使用任何HTTP客户端测试：

```bash
# 健康检查
curl http://localhost:8087/health

# 就绪检查
curl http://localhost:8087/ready

# 查看支持的格式
curl http://localhost:8087/api/v1/documents/formats
```

## 🧪 核心功能测试

### 1. 文档解析测试

#### A. Markdown结构化解析（推荐先测试）

```bash
curl -X POST http://localhost:8087/api/v1/documents/structured \
  -H "Content-Type: application/json" \
  -d '{
    "markdown_content": "# 标题1\n\n这是内容\n\n## 标题2\n\n更多内容",
    "enable_toc": true,
    "max_toc_depth": 3,
    "enable_anchors": true
  }'
```

#### B. URL文档下载解析

```bash
curl -X POST http://localhost:8087/api/v1/documents/download \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/sample.pdf",
    "format": "PDF",
    "enable_toc": true,
    "max_toc_depth": 3
  }'
```

#### C. 文件上传解析

```bash
# 使用test_sample.md作为测试文件
curl -X POST http://localhost:8087/api/v1/documents/upload \
  -F "file=@test_sample.md" \
  -F "format=Md" \
  -F "enable_toc=true" \
  -F "max_toc_depth=3"
```

### 2. 任务管理测试

```bash
# 查看所有任务
curl http://localhost:8087/api/v1/tasks

# 查看特定任务（替换为实际的task_id）
curl http://localhost:8087/api/v1/tasks/{task_id}

# 获取任务统计
curl http://localhost:8087/api/v1/tasks/stats
```

### 3. 结果获取测试

```bash
# 获取文档目录（替换为实际的task_id）
curl http://localhost:8087/api/v1/tasks/{task_id}/toc

# 下载Markdown结果
curl http://localhost:8087/api/v1/documents/{task_id}/markdown/download

# 获取Markdown OSS URL
curl "http://localhost:8087/api/v1/documents/{task_id}/markdown/url?temp=true&expires_hours=24"
```

## 🛠️ 使用REST客户端测试

### 推荐的REST客户端

1. **VS Code REST Client 插件**
   - 安装 "REST Client" 插件
   - 打开 `test_api.rest` 文件
   - 点击请求上方的 "Send Request" 按钮

2. **Postman**
   - 导入 `test_api.rest` 文件
   - 设置环境变量 `baseUrl = http://localhost:8087`

3. **Insomnia**
   - 创建新的请求集合
   - 复制 `test_api.rest` 中的请求

### 测试流程建议

1. **基础检查**
   ```
   GET /health
   GET /ready
   GET /api/v1/documents/formats
   ```

2. **简单功能测试**
   ```
   POST /api/v1/documents/structured  # 测试Markdown解析
   POST /api/v1/documents/markdown/sections  # 测试章节解析
   ```

3. **文件处理测试**
   ```
   POST /api/v1/documents/upload  # 上传test_sample.md
   GET /api/v1/tasks  # 查看任务列表
   GET /api/v1/tasks/{task_id}  # 查看任务详情
   ```

4. **结果获取测试**
   ```
   GET /api/v1/tasks/{task_id}/toc  # 获取目录
   GET /api/v1/documents/{task_id}/markdown/download  # 下载结果
   ```

## 🔧 配置说明

### 服务器配置

- **端口**: 8087 (在 `config.yml` 中配置)
- **主机**: 0.0.0.0
- **日志级别**: info
- **日志路径**: logs/

### 支持的文档格式

- PDF
- Word (DOC, DOCX)
- Excel (XLS, XLSX)
- PowerPoint (PPT, PPTX)
- Text (TXT)
- Markdown (MD)
- HTML

## 🐛 常见问题

### 1. 服务器启动失败

- 检查端口8087是否被占用
- 确保所有依赖已安装：`cargo build`
- 检查配置文件 `config.yml` 是否正确
- **首次使用必须先运行:** `document-parser uv-init`
- 检查虚拟环境状态：`document-parser check`

### 2. 文件上传失败

- 检查文件大小限制
- 确保文件格式受支持
- 检查文件路径和权限

### 3. Python依赖问题

- **MinerU或MarkItDown不可用:** 运行 `document-parser uv-init`
- **虚拟环境问题:** 删除 `./venv/` 目录后重新初始化
- **网络问题:** 使用国内镜像源或配置代理
- **详细故障排除:** 运行 `document-parser troubleshoot`

### 4. OSS功能测试失败

- OSS功能需要配置OSS服务
- 检查 `config.yml` 中的OSS配置
- 确保OSS凭证正确

### 4. 任务状态查询

- 文档解析是异步处理
- 使用 `GET /api/v1/tasks/{task_id}` 查询进度
- 任务状态包括：Pending, Processing, Completed, Failed

## 📊 性能测试

### 并发测试

```bash
# 使用ab进行简单的并发测试
ab -n 100 -c 10 http://localhost:8087/health

# 使用wrk进行更详细的性能测试
wrk -t12 -c400 -d30s http://localhost:8087/health
```

### 大文件测试

- 测试不同大小的PDF文件
- 监控内存使用情况
- 检查处理时间

## 📝 测试记录

建议记录测试结果：

- [ ] 环境初始化完成 (`document-parser uv-init`)
- [ ] 环境检查通过 (`document-parser check`)
- [ ] 健康检查通过
- [ ] 基础API响应正常
- [ ] 文档上传功能正常
- [ ] URL下载功能正常
- [ ] OSS集成功能正常
- [ ] 任务管理功能正常
- [ ] 目录生成功能正常
- [ ] Markdown输出正常
- [ ] 虚拟环境激活正常

## 🔗 相关链接

- [项目文档](../README.md)
- [故障排除指南](./TROUBLESHOOTING.md)
- [API规范](./src/handlers/)
- [配置说明](./config.yml)
- [错误处理](./src/error.rs)

## 🆘 获取帮助

如果遇到问题：

1. **运行诊断命令:**
   ```bash
   document-parser check         # 检查环境状态
   document-parser troubleshoot  # 详细故障排除指南
   ```

2. **查看日志文件:** `logs/` 目录下的日志信息

3. **重新初始化环境:**
   ```bash
   rm -rf ./venv                 # 删除虚拟环境
   document-parser uv-init       # 重新初始化
   ```

4. **查看详细文档:** [TROUBLESHOOTING.md](./TROUBLESHOOTING.md)

---

**提示**: 首次使用必须运行 `document-parser uv-init` 来初始化Python环境。大多数问题都可以通过重新初始化环境来解决。