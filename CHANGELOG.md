# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.52] - 2026-02-15

### Fixed

- **PATH 按段去重**: `ensure_runtime_path` 从整段 `starts_with` 改为按分隔符拆段去重，
  解决上层多次前置导致的 `node/bin` 重复条目问题
- **config PATH 覆盖**: 用户在 MCP config env 中指定自定义 PATH 时，
  仍然确保应用内置运行时路径（`NUWAX_APP_RUNTIME_PATH`）在最前面
- **env value 泄露**: `log_command_details` 不再打印 env 变量的 value，
  仅输出 key 列表，避免泄露敏感信息（如 token、secret）

### Added

- **`mcp-common::diagnostic` 模块**: 提取子进程启动诊断日志为独立模块，
  提供 `log_stdio_spawn_context`、`format_spawn_error`、`format_path_summary` 等公共函数，
  减少业务代码中的日志侵入
- **启动阶段环境诊断**: `env_init` 结束后输出 PATH 摘要和镜像环境变量最终值
- **spawn 失败上下文**: SSE/Stream 子进程 spawn 失败时输出完整错误上下文
  （command、args、PATH），便于快速定位可执行文件找不到的问题
- **build 失败上下文**: `mcp_start_task` 中 SSE/Stream server build 失败时
  通过 `anyhow::Context` 附加 MCP ID 和服务类型
- **`ensure_runtime_path` 单元测试**: 新增 5 个测试覆盖前置、部分去重、
  全部已存在、双重重复等场景

### Changed

- **server_builder PATH 逻辑简化**: 两侧 `connect_stdio` 从三分支
  （继承/config/缺失）统一为：取基础 PATH → `ensure_runtime_path` → 传递给子进程
- **无镜像提示**: 未配置镜像源时输出提示行，而非静默跳过

## [0.1.51] - 2026-02-14

### Changed

- 版本号更新

## [0.1.49] - 2026-02-13

### Added

- **镜像源配置**: 支持通过 `config.yml` 配置 npm/PyPI 镜像源，
  环境变量优先级高于配置文件
- **环境变量初始化**: `env_init` 模块统一管理子进程环境（镜像源 + 内置运行时 PATH）
- **`UV_INSECURE_HOST` 支持**: HTTP 类型的 PyPI 镜像自动提取 host 并设置

## [0.1.48] - 2026-02-12

### Added

- **跨平台进程管理**: `process_compat` 模块提供 `wrap_process_v8` / `wrap_process_v9` 宏，
  统一 Unix（ProcessGroup）和 Windows（JobObject + CREATE_NO_WINDOW）的进程包装

### Fixed

- Windows 平台隐藏控制台窗口配置
