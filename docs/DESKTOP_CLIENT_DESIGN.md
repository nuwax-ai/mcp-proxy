# 桌面客户端设计方案 v2.1（一体版）

> 状态：**定稿**（全部决策已落：命名 `ai-toolbox`、按需分服务安装、GitHub Releases、STT 默认 small、TTS 排 P1）。实施计划见 plan。
> 服务端 API 契约、gpui-kit 落地要点、资源供给链均已对代码复核（v2.1 修正了 v2 的资产来源与断点续传两处错误，见 §2.6）。

## 一、Spec：做什么

### 1.1 背景与动机

voice-cli（语音转写/合成）与 document-parser（文档解析）已有服务端和 deploy-installer 部署链，但只有 HTTP API。需要一个**图形桌面客户端**承载日常操作。

**产品形态（v2 定案）**：客户端默认是**开箱即用的一体版**——安装客户端即得全部能力：它自带并在本地托管两个服务进程（随机端口、仅监听 127.0.0.1），用户**零配置**，界面上不存在"服务地址"这个概念；设置职责退化为"服务状态可见性"。

### 1.2 目标

| 目标 | 说明 |
|---|---|
| 开箱即用 | 装完客户端就能用，不填地址、不装服务、不碰命令行 |
| 自托管服务 | vc/dp 作为客户端子进程运行：随机端口、只绑 127.0.0.1、随客户端退出而停止 |
| 状态可见 | 首页即服务状态面板：下载/安装/启动/运行/失败全程可见，失败可重试、可看日志 |
| 按需开通 | 两个服务分别安装——只转写的用户不必下载解析服务（体积见 §2.5-1） |
| 三平台 | macOS（Metal）/ Windows（CPU）/ Linux（CPU），一份 Rust 代码 |

### 1.3 非目标（边界）

- **不做地址配置**：P0 不提供"连接远端部署"模式（如未来要连 19 这类共享服务器，作为 P2 的"接入模式"开关，不影响一体版默认路径）
- **不做系统服务管理**：不注册开机自启、不做 deploy-installer 级别的运维（重启/日志目录按钮有，系统服务编排没有）
- **P0 不做**：实时听写（麦克风→WS 流式）、流式 TTS 播放、批量任务、任务历史、客户端自动更新
- **不进主 workspace**：依赖隔离理由不变（§2.2）

### 1.4 用户场景

1. **首次**：打开客户端 → 状态页两张卡片显示"未安装（约 X GB）" → 点「安装」看进度 → 就绪后转写/解析页直接可用；也可只装其中一个
2. **日常**：打开即用（服务随客户端自动拉起，秒级就绪的跳过等待界面）；标题栏常驻两盏服务状态灯
3. **异常**：服务起不来/崩了 → 状态页红字 + 错误摘要 + 日志尾部 + 「重启」按钮

---

## 二、Plan：如何实现

### 2.1 技术选型

| 层 | 选型 | 理由 |
|---|---|---|
| UI 框架 | `gpui-kit = 0.7`（crates.io，Apache-2.0） | Longbridge Pro 生产同源；中文 IME 商用验证；speech 模块的可插拔识别器为 P2 实时听写预留正式接入口 |
| HTTP | `gpui-pre-reqwest-client`（=0.3.7，与 gpui-kit 同快照族）注册 `cx.set_http_client(...)`；multipart 上传若高层接口不便，退路是同族 fork `gpui-pre-reqwest`（=0.12.15）直接建 Client——封装收敛在 `api/mod.rs` 一处 | gpui 生态标准做法；注册后 Markdown 远程图片自动经它加载；应用层零 tokio |
| 进程托管 | `std::process` + 平台进程组（Unix `process_group`/`setsid`，Windows Job Object） | 服务崩/客户端崩都不能留孤儿；deploy-installer 已有同型实现可参考 |
| 资源供给 | **复用 deploy-installer 已导出的资产设施**（`crates/deploy-installer/src/lib.rs` 已导出 install/stop/start/status、`optional_*_url()` URL 族、`deploy_asset_version()` manifest、`copy_if_exists` 等）；基础二进制随客户端包分发（§2.6） | lib 入口大半现成；实改只剩"文件安装与 unit 注册解耦"（§2.7） |
| 配置持久化 | TOML（`toml` + `serde` + `dirs`） | 客户端自身设置（默认引擎、STT 模型档位等）+ 每次启动生成的服务运行时配置（随机端口） |
| 音频播放（P1） | `rodio` | gpui-kit 的 speech 只做输入侧 |
| 麦克风采集（P2） | gpui-kit `Microphone`（cpal）+ `SpeechRecognizer` trait | voice-cli 流式 WS 实现成识别器，复用 `SpeechButton`/`SpeechWaveform` |

### 2.2 仓库位置与 workspace 隔离

```
mcp-proxy/
├── Cargo.toml            # 主 workspace：exclude = ["clients/desktop"]
├── ...现有 14 成员不动（deploy-installer 有实改，见 §2.7）...
└── clients/
    └── desktop/          # 独立 [workspace]，独立 Cargo.lock；可 path 依赖主仓的 deploy-installer
        ├── Cargo.toml
        ├── .gitignore    # target/
        └── src/...
```

**代码风格**：主仓无 rustfmt/clippy 配置文件（stock rustfmt 100 列 + CI `clippy -D warnings` 约定），客户端沿用同样约定。`.gitignore` 已有 `**/target` 通配，无需加行。

**命名（已定案）**：目录 `clients/desktop`；crate/bin **`ai-toolbox`**，窗口名「AI 工具箱」。客户端版本线独立从 0.1.0 起。

**与 gpui-kit 本地源码联调**（可选，不提交）：`[patch.crates-io] gpui-kit = { path = ".../gpui-kit/crates/kit" }`。

**跨 workspace 依赖说明**：独立 workspace path 依赖主仓 `crates/deploy-installer` 是合法的，其依赖树只进客户端自己的 lockfile——这正是隔离的意义；deploy-installer 依赖量中等（reqwest/clap 等），不会把 gpui 级别的膨胀反向带进来。

### 2.3 应用架构（crate 内部）

```
src/
├── main.rs          # 入口：application().with_assets(...).run → 主窗口 + 单实例锁 + ServiceManager 启动
├── app.rs           # 全局状态：页面路由（状态/转写/解析）+ ServiceManager 的 Entity + 状态灯
├── settings.rs      # 客户端自身设置（默认引擎、STT 模型档位）持久化
├── services/
│   ├── mod.rs       # ServiceManager：每个服务一张状态机（见 §2.4-①），发事件驱动 UI
│   ├── provision.rs # 资源供给：调 deploy-installer 库模式装文件/模型/ffmpeg（带进度回调）
│   ├── supervisor.rs# 进程托管：挑随机空闲端口 → 生成运行时 config（host=127.0.0.1）→ spawn
│   │                #   → /health 轮询 → 指数退避重启 → 退出/崩溃清理 + pidfile 孤儿回收
│   └── paths.rs     # 目录布局：<appdata>/services/{voice-cli,document-parser}/（二进制、模型、venv、日志）
├── api/
│   ├── mod.rs       # ApiClient：从 ServiceManager 拿动态端口；HttpResult{code,message,data} 信封解包
│   ├── voice.rs     # voice-cli DTO + 调用：health / models / transcribe（P1: tts, voices）
│   └── document.rs  # dp DTO + 调用：health / ready / parse-sync（P1: 异步任务提交+轮询）
├── pages/
│   ├── status.rs    # 状态首页（§2.4-①）
│   ├── transcribe.rs
│   ├── tts.rs       # 合成页（P1 占位）
│   └── document.rs
└── widgets/
    └── file_picker.rs # 文件选择：App::prompt_for_paths（gpui-pre src/app.rs:1582，已验证存在）；OS 文件拖拽 gpui-kit 无现成支持，P1 自建
```

**状态流**（gpui-kit 标准桥接，stream-markdown 范例同款）：UI 事件 → `cx.background_spawn`（HTTP/进程操作）→ `smol::channel` → `cx.spawn(weak)` 消费 → `weak.update(cx)` 回填。Markdown 结果用 `TextView`（流式 `push_str`，GFM 表格开箱即用）。

**运行时配置生成**（supervisor 每次启动做）：在服务安装目录写 `config.runtime.yml`——`server.host: 127.0.0.1`、`server.port: <随机空闲端口>`、数据/日志目录指 appdata——以 `--config` 传入。已知坑备案：环境变量覆盖端口在旧版本不可靠（`VOICE_CLI_SERVER_PORT 不生效`），所以走生成配置文件这条稳路。

### 2.4 页面设计

**① 状态页（首页）——v2 核心改动**

两张服务卡片（voice-cli / document-parser），每张跑同一状态机：

```
未安装 ──安装(下载)──▶ 下载中 ──▶ 安装中(解压/venv) ──▶ 已安装
   ▲                                                     │ 随客户端启动拉起
   │                                                     ▼
 卸载(删目录) ◀── 已停止 ◀──停止── 运行中(端口·版本·uptime·已加载模型)
                          │                    │
                          └── 崩溃/起不来 ◀─────┘
                              红字错误 + 日志尾部(最近N行) + 「重启」
```

- 下载中/安装中显示：进度条、已下载/总量、速度（provision 进度回调直通 UI）
- 运行中显示：端口（只读展示，用户无需用）、服务版本、模型加载状态（`/health` 的 `models_loaded`）
- 卡片操作：安装 / 启动 / 停止 / 重启 / 打开日志目录；安装前展示体积预估（vc≈3.2GB 含 large-v3，或按所选模型档变化；dp≈1.4GB）
- 标题栏常驻两盏状态灯（绿=运行 / 黄=启动中 / 红=失败 / 灰=未安装），异常时点击跳状态页
- STT 模型档位选择放在 vc 卡片的安装确认处：tiny(75MB)/base(142MB)/small(466MB)/medium(1.5GB)/large-v3(3GB)，**默认 small（已定案）**。落地方式（已核对修正）：**large-v3 可走 OSS 预打包快路径**（manifest `optional_assets.whisper_large_v3`，whisper OSS 包只有 large-v3/all 两档）；**其余档位由客户端 provision 自行下载 `ggml-<tier>.bin`（hf-mirror.com 优先、huggingface.co 回退）**——不能用 `voice-cli model download` CLI：它硬编码 HF 直链且无镜像参数（model_service.rs:182-186），国内不可用
- 安装时机（已定案）：**首次打开按需分服务安装**——状态页两张卡片各自「安装」，转写/解析页首次使用时引导跳转，不自动全装

**② 转写页**
- 文件选择区（点击选文件为主交互，拖拽 P1）；引擎下拉：固定枚举 `whisper | sensevoice | fireredasr2 | funasrnano | qwen3asr`（multipart `engine` 字段）；选 whisper 时出模型下拉（`GET /models` 的 `available_models`）
- 「开始转写」→ `POST /transcribe`（multipart：`file` + 可选 `engine`/`model`/`language`/`initial_prompt`）
- 结果：文本（可选中复制）+ 保存 .txt + `processing_time`/`duration`/音频元信息；失败显示 HttpResult `message`
- 未安装 vc 时本页显示引导卡片 → 一键跳状态页安装

**③ 解析页**
- 文件选择区（pdf/docx/xlsx/pptx/md…，白名单跟 `DocumentFormat` 枚举走）
- `POST /api/v1/documents/parse-sync`（multipart 仅 `file`；`upload_*` 四个 query 参数 P0 不传）
- 结果：Markdown 渲染（GFM 表格）+ 导出 .md（`markdown_content` 原文落盘）
- **图片约束（已核对）**：parse-sync 不传 `upload_*` 时图片引用是服务端本地临时路径，客户端不可达——P0 同步解析=文本预览；带图体验绑定 P1 异步流（`oss_data` 全是可拉取 OSS URL，Markdown 组件注册 http_client 后远程图自动加载）
- 服务端三类错误如实转译：408（10 分钟超时）/ 413（500MB 上限）/ 排队（信号量默认并发 2）
- 首次启动 dp 的 venv 自建阶段（win/linux 数分钟）由状态页进度覆盖，本页只显示"服务就绪中"

### 2.5 关键技术点与风险

| # | 风险/要点 | 对策 |
|---|---|---|
| 1 | **首跑体积**：vc（二进制+ffmpeg+large-v3）≈3.2GB；dp（二进制+venv+mineru 模型）≈1.4GB | **已定案：按服务分别安装**（不用的不下）；STT 档位选择（large-v3 走 OSS 快路径、其余档走 `model download` CLI）；下载进度 P0 必做，**断点续传降为 P1 增强**（安装器现有下载器无 Range/续传，属新开发） |
| 2 | **dp venv 平台差异**：mac 有预编译包（330MB 直下，141 打包流程现成）；win/linux 首启自建（uv 下载+装 mineru，数分钟） | mac 走预编译；win/linux 交给 dp 自身的 uv-init（服务端已有该能力）+ 状态页如实展示长启动；uv-init 假失败自动重试一次（93 机器踩过） |
| 3 | **孤儿进程**：客户端崩溃/被强杀时子进程残留（dp 还有 mineru 孙进程） | pidfile 记进程组；客户端启动时回收陈旧 pid；退出走进程组终止——Unix 侧客户端自实现（`Command::process_group(0)` + `killpg`，std 现成）；Windows 参考安装器 `taskkill /T /F` 树杀（service_task.rs:195） |
| 4 | **单实例**：两个客户端同写一个安装目录会坏 | appdata 文件锁，P0 强制单实例（重复启动激活已有窗口） |
| 5 | **端口与暴露面** | 随机空闲端口 + 只绑 127.0.0.1：不与机器上已有 8077/8087 部署冲突、不触发 Windows 防火墙弹窗、不暴露局域网 |
| 6 | **服务版本配对** | 客户端自带 manifest 钉服务资产版本（照搬 deploy-installer 的 assetVersion 机制；发布包 manifest 须实取核对——19 迁移时的教训） |
| 7 | **服务进程崩溃** | /health 轮询（2s 间隔）+ 指数退避自动重启（上限 3 次）+ 状态灯变红人工介入 |
| 8 | gpui-pre 快照 API 会变 | 只依赖 gpui-kit 公共 API；升级跟 gpui-kit release |
| 9 | 平台成熟度：mac > win > linux | 内部工具先保 mac/win；linux 出包但不承诺打磨；linux 编 gpui/speech 需 GUI dev 库（发布期相关） |
| 10 | OS 文件拖拽 | 已核对 gpui-kit 全仓无文件拖拽配方——P0 文件选择为主，拖拽 P1 用 raw GPUI 事件自建 |

### 2.6 发布与安装（已定案：GitHub Releases，暂不上 OSS）

资产分两层（v2.1 已对代码核对修正——v2 所写"服务资产从 OSS `v{version}/` 拉取"对基础二进制不成立）：

- **随客户端包分发**：GUI + 两个服务的基础二进制。基础二进制不在 OSS（它们嵌在 deploy-installer 的 npm 包 vendor 目录里，`bundled_binary_path()` 从 `deploy_root()` 取本地文件）——所以客户端发布 workflow 打 tag 时**从对应版本的 npm tarball 提取 vendor 二进制**打进三平台压缩包（mac tar / win zip / linux tar），服务版本配对在客户端发布时一次性固定（这正是风险表 #6"manifest 钉版本"的落地）。包体量级约 100–150MB
- **运行期按需下载**（复用 manifest `optional_assets` 的 URL 构造器，`bundles.rs` 已导出）：whisper large-v3 模型包、mineru 模型、ffmpeg、mac 预编译 venv——这些资产本来就只在 OSS 上，属**复用现有资产**，不是给客户端新增 OSS 发布物；其余 STT 档位走 `voice-cli model download <tier>` CLI（走模型源镜像，不经 OSS）
- 服务资产更新：P0 客户端 manifest 钉死版本（要更新=发新版客户端）；P2 加"检查服务更新"
- 阶段二（可选）：deploy-installer 加 `client` 组件统一分发（放二进制+快捷方式，不注册服务），复用 npm 三平台嵌入骨架

### 2.7 对主仓的改动清单（v2 升级为实改）

| 文件 | 改动 |
|---|---|
| `Cargo.toml`（根） | `exclude = ["clients/desktop"]` |
| `AGENTS.md` | Development Commands 加一节：客户端独立 workspace 的构建/运行命令 |
| `crates/deploy-installer` | **实改（比 v2 预估的小）**：`lib.rs` 已导出 install/stop/start/status 与全套 URL 构造器——剩余工作只是把 install 管线里"铺文件（二进制/config/模型/ffmpeg/venv）"与"渲染并注册 unit"两段解耦（`InstallOptions` 目前只有 enable/start/dry_run，没有 no-service 模式），给客户端一个"只装文件"入口；CLI 行为不变 |
| CI workflows | 不动（客户端 CI 留到 T5 后） |

`.gitignore` 无需改动（`**/target` 已覆盖）。

### 2.8 服务端 API 契约速查（已核对）

**统一响应信封** `HttpResult<T>`：`{ code, message, data }`（voice-cli 版多一个可选 `tid`），`code == "0000"` 成功、`E0xx` 失败——客户端统一取 `message` 做错误展示。定义：voice-cli `src/models/http_result.rs:15`、dp `src/models/http_result.rs:10`。supervisor 的就绪判定：vc 用 `/health`，dp 用 `/ready`（比 `/health` 严格）。

**voice-cli（一体版由 supervisor 动态分配端口）**

| 端点 | 请求 | 响应 |
|---|---|---|
| `GET /health` | — | `{status:"healthy", models_loaded:[…含 tts: 前缀], uptime, version}` |
| `GET /models` | — | `{available_models:[whisper 五档], loaded_models, model_info}`——引擎清单不在里面，引擎是固定 5 选 1 枚举 |
| `POST /transcribe` | multipart：`file`（必）+ `model` / `language` / `initial_prompt` + `engine`（`whisper\|sensevoice\|fireredasr2\|funasrnano\|qwen3asr`，大小写不敏感） | `{text, segments[{start,end,text,confidence}], language, duration, processing_time, metadata}` |
| `POST /api/v1/tts`（P1） | **JSON body**：`{text, model?, sid?, voice?, reference_audio?, speed?, format?}` | **raw 音频字节**（默认 wav），header `X-Processing-Time`；出错时才是 HttpResult JSON |
| `GET /api/v1/tts/voices`（P1） | — | 裸 JSON（无信封），按 backend 变体：kokoro 给 `num_speakers`，zipvoice 另给 `profiles[]` |

**document-parser**

| 端点 | 请求 | 响应 |
|---|---|---|
| `GET /health` \| `/ready` | — | `data:"health"` \| ready 或错误（supervisor 用 /ready） |
| `POST /api/v1/documents/parse-sync` | multipart 仅 `file`；query 可选 `upload_base_url` / `upload_path` / `upload_api_key` / `upload_type` | `{markdown_content, format, engine(MinerU\|MarkItDown), processing_time_ms, word_count, filename, file_size}`；408=超时 / 413=超限 |
| `POST /api/v1/documents/upload`（P1） | multipart `file` + query `enable_toc` / `max_toc_depth` / `upload_*` | 202 `{task_id, message, file_info}` |
| `GET /api/v1/tasks/{id}`（P1） | — | `{task:{status: Pending \| Processing{stage, progress_details} \| Completed \| Failed \| Cancelled, progress:0-100, oss_data{markdown_url, images[].oss_url}, …}}`——11 阶段状态机可直接驱动进度条 |

**DTO 镜像来源**：voice-cli `src/models/{request.rs, http_result.rs, tts.rs}`；dp `src/models/{http_result.rs, document_task.rs, task_status.rs, oss_data.rs}` + `handlers/response.rs`。两服务都开放 `/api/docs/openapi.json`，字段漂移时拉它比对。

### 2.9 gpui-kit 落地要点（已核对）

| 要点 | 结论 |
|---|---|
| 应用骨架 | `gpui_kit::application().with_assets(...).run` → `gpui_kit::init(cx)` → `gpui_kit::open_window(...)`（自动包 Root，主题经 `ActiveTheme` 生效）——ai_recipes/bootstrap.rs 是标准入口模板 |
| Markdown 渲染 | 无需 feature；`TextView::markdown` 一次性 / `TextViewState::markdown` 流式；GFM 表格默认支持；远程图片在注册 http_client 后自动加载 |
| 侧边导航 | `Sidebar/SidebarGroup/SidebarMenu` 组件齐全但范例不含内容区切换——自存"当前页"状态 match 渲染（四页小工具不必上 Dock）；Dock 留作 P3 选项 |
| 输入组件 | `Input/InputState/InputEvent`、`Textarea`、`Form/Field`、`Select`——接线模式：owner 持 `Entity<InputState>` + `cx.subscribe_in` 存入 `_subscriptions` |
| 异步桥接 | `cx.background_spawn`（IO/HTTP/进程）+ `smol::channel` + `cx.spawn(weak)` 消费回 UI |
| speech（P2） | feature `speech`；`SpeechRecognizer` trait 实现 voice-cli WS 识别器；mac 麦克风权限用 build.rs 嵌 Info.plist（speech 范例现成）；Linux 需 `libasound2-dev` |
| 依赖基线 | `gpui-kit + anyhow` 起步，按需 `serde/serde_json/smol/toml/dirs`（hello_world 仅 2 个依赖） |

---

## 三、Task：执行拆解与完成标准

| # | 任务 | 内容 | 完成标准（可验证） |
|---|---|---|---|
| T1 | 骨架 | 独立 workspace、窗口、侧边导航四空页、单实例锁 | `cargo run` 出窗口；重复启动不产生第二实例 |
| T2 | 服务托管核心（vc） | supervisor（随机端口/生成 config/spawn/health 轮询/退避重启/退出清理/pidfile 孤儿回收）+ provision（deploy-installer 库模式装 vc 二进制+模型+ffmpeg，进度直通 UI）+ 状态页 vc 卡片全状态机 | 全新 appdata 下：卡片走完 未安装→下载→运行中；杀掉服务进程 2 秒内红字并自动重启；强杀客户端后重开无孤儿进程 |
| T3 | 转写页 | 文件选择 + 引擎/模型下拉 + /transcribe + 复制/保存 | 选 jfk.wav 级小文件出文字；切引擎生效；服务停止时页面给出引导而非报错堆栈 |
| T4 | dp 接入 + 解析页 | dp 卡片（mac 预编译 venv / win·linux 自建含长启动进度）+ parse-sync + Markdown 渲染 + 导出 .md | mac：全新安装后选 PDF 出可读 Markdown，导出一致；未安装 dp 时解析页引导安装 |
| T5 | 打磨收尾 | 加载/空/错误态、窗口图标、`clippy + fmt` 干净、README、mac+win 双平台冒烟 | 双平台 T2-T4 场景全过；边界（未装/强杀/断网下载中断重试）不崩 |

### 里程碑

- **P0（本方案）**：T1–T5 一体版可用（mac 优先，win 跟进）
- **P1**：TTS 页（rodio 播放 + 音色列表 + 流式）、dp 异步任务流（带图 Markdown）、文件拖拽
- **P2**：实时听写（`SpeechRecognizer` + WS LA2 + `SpeechButton/Waveform`）、"接入远端"模式（连 19 等共享部署）、服务资产检查更新
- **P3（可选）**：deploy-installer `client` 组件、任务历史表格（虚拟滚动数据表）、Dock 布局

---

## 四、已全部定案

- **命名 `ai-toolbox`**（窗口名「AI 工具箱」）
- **首次打开按需分服务安装**
- **客户端只发 GitHub Releases**（暂不上 OSS；模型/venv/ffmpeg 复用既有 OSS 资产）
- **STT 默认档位 small**（466MB，`model download small` CLI 下载；设置里可升级 large-v3——重跑 model download + 改 config 的 default_model）
- **TTS 排 P1**

方案齐备，可按 §三 Task 顺序实施。
