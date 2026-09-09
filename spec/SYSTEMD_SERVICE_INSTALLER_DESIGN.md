# systemd 服务自动注册工具 —— 设计方案

> **文档类型**:Plan(计划文档)—— 回答「如何实现」。技术方案、架构设计、模块划分。
> **状态**:已实现 (2026-07-16)
> **日期**:2026-07-16
> **目标读者**:维护者 / 后续 Task 拆解

---

## 1. 背景与目标

### 1.1 痛点

当前 voice-cli / document-parser 的 systemd 注册**割裂且易错**:

| 服务 | 现状 | 问题 |
|---|---|---|
| **voice-cli** | 无脚本,全靠 `deploy/README.md` 手动 `sed` 占位符 + `sudo tee` + 3 条 `systemctl` | 占位符漏替换、`--config` 位置错、`heredoc + sudo` 混 stdin 把 heredoc 当密码 —— 均已实际踩坑(见 19/34 部署记录) |
| **document-parser** | 有 `deploy/scripts/install.sh`(bash,较完善) | 仅 enable 不 start、无环境预检、仅服务自身、bash 难维护/难测试 |

两服务 unit 字段差异大(见 §2.3),但注册流程相同(生成 unit → 写 `/etc/systemd/system` → `daemon-reload` → `enable`/`start`)。重复部署多台机器(19/34/nuwax),手动流程的累积成本与出错率都高。

### 1.2 目标

新增一个 **Rust 共享 lib crate `systemd-installer`**,由 voice-cli / document-parser 各自的 CLI 通过新增 `service` 子命令调用,实现:

- **一条命令完成注册**:`<bin> service install --install-dir <dir> [--enable] [--start]`
- **统一两服务流程**,单一实现复用(DRY / SOLID)
- **部署只带二进制**:unit 模板内嵌进二进制(`include_str!`),不再需要 `scp deploy/` 目录
- **Fail Fast 预检**:二进制存在、config 存在、install_dir 可写、端口未冲突、sudo 可用 —— 装之前暴露问题
- **幂等 + 可逆**:`install` 可重复(更新覆盖);提供 `uninstall` / `status` / `restart`

### 1.3 非目标(明确不做)

- 不支持非 systemd 发行版(openrc / launchd / Windows 服务)—— 目标用户是 Ubuntu systemd
- 不做 Docker / k8s 部署
- 不替代 `setup-venv.sh`(Python 环境初始化,不同范畴;`service install` 假设二进制 + 依赖已就绪)
- 不做 mcp-proxy 自身的 systemd 注册(本工具聚焦 voice-cli / document-parser;架构上可扩展,见 §8)

---

## 2. 现状分析

### 2.1 document-parser 已有 `install.sh`(可借鉴,非从零)

`crates/document-parser/deploy/scripts/install.sh` 已验证的流程:

```bash
# 1. 检查二进制存在
# 2. 从模板建 .document-parser.env(chmod 600 + chown)
# 3. sed 替换 __USER__/__GROUP__/__INSTALL_DIR__ → sudo install -m644 -o root -g root 装到 /etc/systemd/system
#    (用 install 命令而非 tee,密码走 stdin 不冲突 —— 绕开 heredoc+sudo 坑)
# 4. sudo systemctl daemon-reload && sudo systemctl enable <name>
```

**借鉴点**:`sudo install -m644` 装 unit 这招已被验证可行;`install.sh` 的步骤划分可直接映射到 Rust 实现。**不足**:无 start、无预检、无 uninstall/status、仅 document-parser、bash 不可单测。

### 2.2 voice-cli / document-parser 的 CLI 结构(集成点)

| 服务 | CLI 架构 | Commands 位置 | 加 `service` 子命令成本 |
|---|---|---|---|
| voice-cli | clap derive,`Commands { Server, Model, Tts }` | `src/cli/mod.rs` | 低 —— 加一个 `Service` variant + `src/cli/service.rs` handler,`main.rs` dispatch 一行 |
| document-parser | clap derive,`Commands { ... }` 内联在 main.rs | `src/main.rs` | 低 —— `Commands` enum 加 `Service` variant + handler 函数 |

两服务都已有成熟的 clap 子命令架构,集成是「加一个 variant + handler」,不涉及架构改动。

### 2.3 两服务 systemd unit 字段差异(参数化依据)

| 字段 | voice-cli | document-parser |
|---|---|---|
| `ExecStart` | `<dir>/voice-cli server run --config <dir>/config.yml` | `<dir>/document-parser server` |
| `EnvironmentFile` | 无 | `<dir>/.document-parser.env`(OSS 密钥) |
| `Environment` | `LD_LIBRARY_PATH=...`(CUDA/sherpa,可选) | 无 |
| drop-in | `voice-cli.service.d/cuda-sherpa.conf`(可选,LD_LIBRARY_PATH) | 无 |
| `KillSignal` | 默认 | `SIGINT`(优雅关停 mineru) |
| `TimeoutStopSec` | 默认 | `60` |
| `User/Group/WorkingDirectory/Restart/WantedBy` | 共有 | 共有 |

→ 差异全部可通过 `ServiceSpec` 的字段(含 `Option`)参数化,见 §3.2。

### 2.4 mcp-common 不适合放

`crates/mcp-common` 现职责是 proxy 共享(i18n / telemetry / backend_bridge / tool_filter),塞 systemd 部署逻辑语义不符。**新建专门 `systemd-installer` crate**(单一职责:生成 + 安装 systemd unit)。

---

## 3. 总体架构

```
┌─────────────────────────────────────────────────────────┐
│  crates/systemd-installer  (新 lib crate,单一职责)        │
│  ─ ServiceSpec 描述一个服务的 unit 字段                    │
│  ─ 通用 unit 模板渲染(spec → unit 文本)                   │
│  ─ install / uninstall / status / restart(调 systemctl)  │
│  ─ 预检 checks(二进制/config/端口/权限)                    │
└───────────────▲───────────────────────▲──────────────────┘
                │ 依赖(depend on lib)    │
    ┌───────────┴──────────┐   ┌─────────┴──────────┐
    │ voice-cli            │   │ document-parser    │
    │ cli: service install │   │ cli: service install│
    │      service uninstall│  │      service uninstall
    │      service status  │   │      service status │
    │ (构造自己的 ServiceSpec)│   │ (构造自己的 ServiceSpec)│
    └──────────────────────┘   └────────────────────┘
```

**核心原则**:
- lib **不认识** voice-cli / document-parser 的业务,只认 `ServiceSpec`。两服务各自把自身参数装进 spec 调 lib。
- 模板**内嵌**进二进制,部署只带二进制 + config + 模型。
- lib 可单测(渲染、占位符、幂等),不依赖真实 systemd(Mac 上用 `--dry-run` 验证渲染)。

---

## 4. 模块设计

### 4.1 新 crate 目录结构

```
crates/systemd-installer/
├── Cargo.toml
├── src/
│   ├── lib.rs          # 模块声明 + re-export
│   ├── spec.rs         # ServiceSpec / ServiceIdentity / DropIn 数据结构
│   ├── render.rs       # spec → unit 文本(通用模板,手写渲染无模板引擎)
│   ├── checks.rs       # 预检:二进制/config/install_dir/端口/sudo
│   ├── installer.rs    # install/uninstall/status/restart 核心(调 systemctl/sudo install)
│   ├── systemd.rs      # systemctl 命令封装(daemon-reload/enable/start/is-active/...)
│   └── error.rs        # thiserror 错误类型 InstallerError
└── tests/
    └── render_tests.rs # 渲染快照测试(各 spec 组合 → 期望 unit 文本)
```

### 4.2 核心数据结构(`spec.rs`)

```rust
use std::path::PathBuf;

/// 描述一个待注册的 systemd 服务(引擎无关,lib 不认具体业务)
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    /// 服务名(unit 文件名,如 "voice-cli" → voice-cli.service)
    pub name: String,
    /// unit Description
    pub description: String,
    /// 运行用户 / 组(默认从 whoami 推断)
    pub identity: ServiceIdentity,
    /// 安装根 = WorkingDirectory + 二进制所在目录
    pub install_dir: PathBuf,
    /// 已拼好的 ExecStart 命令(含参数,如 ["voice-cli","server","run","--config",".../config.yml"])
    pub exec_start: Vec<String>,
    /// EnvironmentFile(document-parser 的 .env;voice-cli 无)
    pub env_file: Option<PathBuf>,
    /// 额外 Environment(voice-cli 的 LD_LIBRARY_PATH 等)
    pub extra_env: Vec<(String, String)>,
    /// KillSignal(document-parser 用 SIGINT)
    pub kill_signal: Option<String>,
    /// TimeoutStopSec(秒;document-parser 用 60)
    pub timeout_stop_sec: Option<u64>,
    /// drop-in 覆盖文件(voice-cli 的 cuda-sherpa.conf)
    pub drop_ins: Vec<DropIn>,
    /// SupplementaryGroups(GPU 场景:video render)
    pub supplementary_groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub user: String,
    pub group: String,
}

#[derive(Debug, Clone)]
pub struct DropIn {
    /// 相对 unit 的 drop-in 名,如 "cuda-sherpa" → <name>.service.d/cuda-sherpa.conf
    pub name: String,
    /// drop-in 内容(已渲染好的 [Service] 覆盖段)
    pub content: String,
}
```

### 4.3 模板渲染(`render.rs`,通用 + 手写)

**不引入模板引擎**(handlebars/tera),用结构化拼接 —— 模板逻辑简单(条件字段),手写可控、零依赖、易测。

```rust
pub fn render_unit(spec: &ServiceSpec) -> Result<String, InstallerError> {
    let mut s = String::new();
    s.push_str("[Unit]\n");
    s.push_str(&format!("Description={}\n", spec.description));
    s.push_str("After=network.target\n\n");

    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    s.push_str(&format!("User={}\n", spec.identity.user));
    s.push_str(&format!("Group={}\n", spec.identity.group));
    s.push_str(&format!("WorkingDirectory={}\n", sanitize_path(&spec.install_dir)?));
    if let Some(env_file) = &spec.env_file {
        s.push_str(&format!("EnvironmentFile={}\n", sanitize_path(env_file)?));
    }
    s.push_str(&format!("ExecStart={}\n", shell_join(&spec.exec_start)?));
    for (k, v) in &spec.extra_env {
        s.push_str(&format!("Environment={k}={v}\n"));
    }
    s.push_str("Restart=on-failure\nRestartSec=5\n");
    if let Some(ks) = &spec.kill_signal { s.push_str(&format!("KillSignal={ks}\n")); }
    if let Some(t) = spec.timeout_stop_sec { s.push_str(&format!("TimeoutStopSec={t}\n")); }
    if !spec.supplementary_groups.is_empty() {
        s.push_str(&format!("SupplementaryGroups={}\n", spec.supplementary_groups.join(" ")));
    }
    s.push_str("StandardOutput=journal\nStandardError=journal\n\n");

    s.push_str("[Install]\nWantedBy=multi-user.target\n");
    Ok(s)
}
```

**`sanitize_path` / `shell_join`**:校验路径/参数无注入风险(拒绝含换行、`%`、异常字符),Fail Fast。

### 4.4 预检(`checks.rs`,Fail Fast)

```rust
pub struct PrecheckReport { /* 各项 pass/fail + 诊断信息 */ }

pub fn precheck(spec: &ServiceSpec) -> Result<PrecheckReport, InstallerError> {
    // 1. 二进制存在且可执行(spec.exec_start[0] 解析为绝对路径)
    // 2. config 存在(voice-cli 的 --config 路径 / document-parser 的 config.yml)
    // 3. install_dir 存在且当前用户可读写(运行期要写 ./logs ./data)
    // 4. env_file 存在(document-parser:OSS 密钥已填?仅警告不阻断)
    // 5. 端口冲突硬阻断(解析 config.yml 的 server.port + 调 ss/lsof 检查) ——
    //    明确占用 → 标 failure(installer 硬 Err),附占用 pid + "改 config server.port 重试"提示;
    //    检测工具缺失/无权限(无法判定) → 降级警告 + 打印手动检查命令,不误阻断合法部署
    // 6. sudo 可用(非 root 运行时,which sudo + sudo -n true 探测,失败给清晰提示)
    // 7. (可选)已存在同名 unit → 提示覆盖/更新
}
```

端口冲突检测(第 5 项)是**最有价值的预检** —— 这次改端口就踩过两服务互占端口的坑。

### 4.5 安装核心(`installer.rs`)

```rust
pub struct InstallOptions {
    pub enable: bool,   // 调用方默认传 true(注册即开机自启)
    pub start: bool,    // 调用方默认传 true(注册后立即启动);CLI --no-start 时传 false
    pub dry_run: bool,  // 只渲染不写
}

pub fn install(spec: &ServiceSpec, opts: &InstallOptions) -> Result<(), InstallerError> {
    let report = checks::precheck(spec)?;          // Fail Fast
    if report.has_failures() { return Err(...); }

    let unit_text = render::render_unit(spec)?;
    let unit_path = format!("/etc/systemd/system/{}.service", spec.name);

    if opts.dry_run {
        println!("--- {unit_path} ---\n{unit_text}");  // Mac 验证用,不实际写
        for d in &spec.drop_ins { println!("--- drop-in {} ---\n{}", d.name, d.content); }
        return Ok(());
    }

    // 写 unit:沿用 install.sh 验证过的 sudo install 套路(密码 stdin 不冲突)
    write_unit_via_sudo_install(&unit_path, &unit_text, spec)?;
    // drop-in
    for d in &spec.drop_ins { write_dropin_via_sudo(spec, d)?; }

    systemd::daemon_reload()?;                      // sudo systemctl daemon-reload
    if opts.enable { systemd::enable(&spec.name)?; }
    if opts.start  { systemd::restart(&spec.name)?; } // 用 restart 幂等(已跑则重载)
    Ok(())
}

pub fn uninstall(name: &str) -> Result<(), InstallerError> { /* stop + disable + rm unit + rm drop-in dir + daemon-reload */ }
pub fn status(name: &str) -> Result<(), InstallerError>     { /* systemctl is-enabled/is-active + cat unit + 最近日志摘要 */ }
```

`write_unit_via_sudo_install`:写到临时文件 → `sudo install -m644 -o root -g root <tmp> <unit_path>` → 删临时文件。和 `install.sh` 第 34-40 行一致(已验证)。

---

## 5. CLI 集成

### 5.1 子命令形态(两服务一致)

```bash
<bin> service install  --install-dir <dir> [--user <u>] [--no-start] [--dry-run]
<bin> service uninstall
<bin> service status
<bin> service restart
# install 默认 = 注册 + enable(开机自启) + start(立即启动),一步到位
# --no-start : 仅注册 + enable,不立即启动(如改完 config 想手动 restart)
# --dry-run  : 只渲染打印 unit,不写 /etc、不调 systemctl(Mac 验证用)
```

`--dry-run`:只渲染打印 unit,不写 `/etc`、不调 systemctl —— **Mac 本地验证渲染的唯一手段**(Mac 无 systemd)。

### 5.2 voice-cli 集成

`src/cli/mod.rs` 的 `Commands` 加 variant:
```rust
pub enum Commands {
    Server { action: ServerAction },
    Model  { action: ModelAction  },
    Tts    { action: TtsAction    },
    Service { action: ServiceAction },   // ← 新增
}
pub enum ServiceAction {
    Install { install_dir: PathBuf, user: Option<String>, no_start: bool, dry_run: bool },
    // 默认 enable + start;no_start=true 仅注册不自启(handler 转 InstallOptions{enable:true, start:!no_start})
    Uninstall,
    Status,
    Restart,
}
```
新文件 `src/cli/service.rs`:`handle_service_install` 构造 voice-cli 的 `ServiceSpec`(含 `--config`、可选 LD_LIBRARY_PATH/drop-in)调 lib;`main.rs` dispatch 加一行。

### 5.3 document-parser 集成

`src/main.rs` 的 `Commands` enum 加 `Service(ServiceAction)` variant + handler。spec 含 `env_file = .document-parser.env`、`kill_signal = SIGINT`、`timeout_stop_sec = 60`。

### 5.4 voice-cli 的 drop-in(LD_LIBRARY_PATH)怎么填

19 上的 `cuda-sherpa.conf` 是为了 sherpa CUDA 加载 cuDNN。spec 提供 `DropIn { name: "cuda-sherpa", content }`,content 由 `--cuda-lib-dir` / `--cudnn-lib-dir` 参数拼 `LD_LIBRARY_PATH`。无 CUDA(纯 whisper 静态)时不生成 drop-in。

---

## 6. 关键设计决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| **形态** | 共享 lib + 各 CLI `service` 子命令 | 复用单一实现;部署只带各自二进制;符合现有 clap 架构(用户已选) |
| **模板存储** | `include_str!` 内嵌进二进制 | 部署只带二进制,不用 scp deploy/;模板随版本走,无漂移 |
| **模板引擎** | 不用,手写渲染 | 字段逻辑简单(条件 Option);零依赖;易单测 |
| **root 权限** | installer 不强制 sudo 运行,内部对写 `/etc` + `systemctl` 调 `sudo` | `User/Group` 用 `whoami` 自然;沿用 install.sh 验证过的 `sudo install` 套路 |
| **预检** | Fail Fast,装之前全检 | 对齐项目原则;直接针对本次踩的坑 |
| **端口冲突** | **硬阻断**(占用即 Err) | 及时暴露,附 pid + 改 config 端口提示;检测工具失败/无权限时降级警告,不误阻断 |
| **install 默认动作** | 默认 enable + start | 最便捷,一步到位;`--no-start` 仅注册 |
| **幂等** | `install` 用 `restart` 而非 `start`(已跑则重载);重复 install 覆盖 unit | 安全可重复 |
| **安全** | `sanitize_path`/`shell_join` 校验路径与参数(拒换行/`%`/异常字符) | 防 unit 文件注入 |

---

## 7. 实现步骤(分阶段)

### P0 —— lib crate 骨架 + 渲染(可独立验证)
1. 新建 `crates/systemd-installer/`(Cargo.toml + 上述 src/ 文件,先 `spec.rs`/`render.rs`/`error.rs`)
2. `render::render_unit` + `sanitize_path` + `shell_join`
3. **单测**:`render_tests.rs` 覆盖 voice-cli / document-parser 两个 spec → 期望 unit 文本快照
4. workspace `Cargo.toml`:members + `[workspace.dependencies]` 加 `systemd-installer`

### P1 —— install/uninstall/status 核心 + 预检
5. `checks.rs`(二进制/config/install_dir/端口/sudo 预检)
6. `systemd.rs`(systemctl 封装,全部 `Command::new("sudo")`)
7. `installer.rs`(`install`/`uninstall`/`status`/`restart` + `--dry-run`)
8. **单测**:precheck 各 fail 分支;`--dry-run` 在 Mac 跑通渲染

### P2 —— voice-cli 集成
9. `voice-cli/Cargo.toml` 依赖 `systemd-installer`
10. `cli/mod.rs` 加 `Service` variant + `cli/service.rs` handler(构造 voice-cli spec,含 `--config` + 可选 drop-in)
11. `main.rs` dispatch
12. **验证**:Mac `voice-cli service install --dry-run` 渲染正确;Linux `install/status/uninstall` 全流程

### P3 —— document-parser 集成 + 收尾
13. `document-parser/src/main.rs` 加 `Service` variant + handler(env_file/SIGINT/timeout)
14. **验证**:同 P2
15. 文档:`voice-cli/deploy/README.md`、`document-parser/deploy/README.md` 改为 `service install` 流程(替代手动 sed/tee)
16. **并入内嵌 + 删外置模板**:把 `voice-cli/deploy/voice-cli.service`、`document-parser/deploy/systemd/document-parser.service.example` 的内容迁入 lib(`include_str!`),**删除这两个外置文件**(内嵌为唯一源,防漂移)
17. **废弃 `document-parser/deploy/scripts/install.sh`**:由 `document-parser service install` 完全替代,删除该脚本

---

## 8. 扩展性

- **新服务**:任何 clap CLI 依赖 `systemd-installer` + 构造 `ServiceSpec` + 加 `service` 子命令即可。mcp-proxy 若要 systemd 注册,同模式。
- **非 systemd**:未来加 `trait Installer { fn install(...); }`,systemd 是其中一个 impl(openrc/launchd 另实现)。当前不做。

---

## 9. 风险与对策

| 风险 | 概率 | 对策 |
|---|---|---|
| sudo 密码/交互在自动化部署(CI/无人值守)卡住 | 中 | 支持 `sudo -n`(NOPASSWD)探测 + 文档说明;非交互场景用 NOPASSWD sudoers 或 root 运行 |
| 端口冲突检测误判(ss/lsof 权限或端口复用) | 中 | 区分两种情况:**明确占用 → 硬阻断**;**检测工具失败/无权限(无法判定) → 降级警告** + 打印手动检查命令,不误阻断合法部署 |
| 内嵌模板与 deploy/ 外置模板双重维护漂移 | 中 | P3 收尾时定内嵌为唯一源,deploy/ 模板降级为参考或删除 |
| 跨发行版 systemd 路径差异(`/etc` vs `/usr/lib`) | 低 | 固定写 `/etc/systemd/system/`(用户 unit,优先级高于 `/usr/lib`),目标 Ubuntu |
| 路径/参数注入 unit 文件 | 低 | `sanitize_path`/`shell_join` 校验 + 单测覆盖恶意输入 |

---

## 10. 验证方案

### 10.1 单测(Mac 可跑)
- `render_tests.rs`:voice-cli spec / document-parser spec / 含 drop-in / 含 env_file / 纯最小 spec → 渲染文本快照
- `precheck` 各 fail 分支(缺二进制、缺 config、端口占用)
- `sanitize_path`/`shell_join` 拒绝换行、`%`、空 ExecStart

### 10.2 集成(Mac)
- `voice-cli service install --dry-run` → 打印 unit,人工核对字段
- clap 解析:`service install/uninstall/status/restart` 各子命令参数

### 10.3 端到端(Linux 19/34)
- `voice-cli service install --install-dir /home/swufe/workspace/voice-server --enable --start` → `systemctl status` active + health 200
- `document-parser service install ...` → 同上
- `service uninstall` → unit 删除 + 服务停止 + 重启不自启
- **幂等**:连跑两次 install 无副作用(restart 生效)
- **端口冲突(硬阻断)**:故意占目标端口后 install → 预检 Err + 提示占用 pid + "改 config server.port 重试";改端口后 install 成功

---

## 11. 改动清单(预估)

| 类型 | 文件 |
|---|---|
| 新建 crate | `crates/systemd-installer/`(Cargo.toml + src/{lib,spec,render,checks,installer,systemd,error}.rs + tests/) |
| workspace | `Cargo.toml`(members + workspace.dependencies) |
| voice-cli | `Cargo.toml`;`src/cli/mod.rs`(Service variant);`src/cli/service.rs`(新);`src/main.rs`(dispatch) |
| document-parser | `Cargo.toml`;`src/main.rs`(Service variant + handler) |
| 文档 | 本文件;`voice-cli/deploy/README.md`;`document-parser/deploy/README.md` |
| 删除(P3) | `voice-cli/deploy/voice-cli.service`、`document-parser/deploy/systemd/document-parser.service.example`(并入内嵌);`document-parser/deploy/scripts/install.sh`(由 service install 替代) |

预估代码量:lib ~400-500 行 + 单测;两服务各 ~50-80 行胶水。

---

## 12. 决策记录(已确认 2026-07-16)

| # | 决策点 | 结论 |
|---|---|---|
| 1 | crate 名 | **`systemd-installer`** |
| 2 | deploy/ 外置模板 | **P3 并入内嵌,删除外置文件**(内嵌为唯一源,防漂移) |
| 3 | document-parser `install.sh` | **废弃删除**(由 `document-parser service install` 替代) |
| 4 | 端口冲突检测 | **硬阻断**(占用即 Err + 提示 pid/改端口;检测工具失败时降级警告,不误阻断) |
| 5 | install 默认动作 | **默认 enable + start**(一步到位);`--no-start` 仅注册 |
