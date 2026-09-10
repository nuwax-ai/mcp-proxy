# test-e2e — 已部署服务的端到端集成测试

对**已在运行**的 document-parser / voice-cli 服务做黑盒测试——本地开发服务或远程部署机（131/93/53 等）均可，目标地址可配置。测试资产（PDF/WAV/PCM）代码内生成，零外部文件依赖。

## 运行

```bash
# 本机服务（dp 8087 / vc 8077 默认）——服务未起则全部 SKIP
cargo test -p test-e2e -- --test-threads=1

# 远程机器
E2E_DOCUMENT_PARSER_URL=http://192.168.32.131:8087 \
E2E_VOICE_CLI_URL=http://192.168.32.131:8077 \
E2E_VOICE_MODEL=base \
cargo test -p test-e2e -- --test-threads=1

# 或复制 .env.local.example → 仓库根 .env.local 写好地址后直接跑
cp crates/test-e2e/.env.local.example .env.local

# make 入口（推荐）
make test-e2e
make test-e2e-remote DP=http://192.168.32.131:8087 VC=http://192.168.32.131:8077
```

## 覆盖

**document-parser**（tests/document_parser.rs）：health/ready、解析引擎健康、Scalar+Swagger 文档并存（spec 路径覆盖）、parse-sync Markdown 往返、`#[ignore]` PDF/MinerU 真实解析（首跑模型下载慢，`-- --ignored` 显式开）、异步任务全生命周期（upload→轮询→Completed→result）、任务取消终态、`#[skip-if-no-url]` uploadFromUrl、结构化 TOC、tasks stats。

**voice-cli**（tests/voice_cli.rs）：health 版本自报、Scalar/Swagger 文档、同步转写（正弦波链路完整性）、异步转写任务全流程、**WS 流式 STT 全事件链**（ready→partial→committed→done）、WS 流式 TTS 协议行为（disabled 部署应回 error 事件而非静默断开）。

## 语义约定

- **目标不可达 → SKIP（非失败）**：`cargo test --workspace` 在无服务机器保持全绿
- **环境性失败 → SKIP**：无 whisper 模型机器的转写场景（收到明确服务端错误即跳过）
- **断言失败 = 产品缺陷**：事件链不完整、终态异常、文档缺失等直接红
- 串行执行（`--test-threads=1`）避免任务队列互扰

## 本地起服务跑通

```bash
cargo run --bin document-parser -- --port 18087 server &   # venv 就绪的机器
cargo run --bin voice-cli -- server run &                    # 按其配置
E2E_DOCUMENT_PARSER_URL=http://127.0.0.1:18087 E2E_VOICE_CLI_URL=http://127.0.0.1:8077 \
  cargo test -p test-e2e -- --test-threads=1
```
