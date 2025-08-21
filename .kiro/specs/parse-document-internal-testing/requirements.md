# Requirements Document

## Introduction

本规范定义了对 `parse_document_internal` 核心函数的全面测试需求。`parse_document_internal` 是文档解析服务的核心逻辑，负责完整的文档解析流程，包括文件验证、格式检测、解析引擎选择、图片处理、OSS上传和结构化文档生成。

## Requirements

### Requirement 1: 文件验证和基础检查测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数能正确处理文件存在性检查、大小限制和MIME类型检测，以确保输入文件的有效性。

#### Acceptance Criteria

1. WHEN 传入不存在的文件路径 THEN 系统 SHALL 返回 "文件不存在" 错误
2. WHEN 传入超过大小限制的文件 THEN 系统 SHALL 返回 "文件大小超过限制" 错误
3. WHEN 传入有效的Markdown文件 THEN 系统 SHALL 正确检测MIME类型为 "text/markdown"
4. WHEN 文件验证通过 THEN 系统 SHALL 更新任务状态为 FormatDetection 阶段
5. WHEN 文件信息获取成功 THEN 系统 SHALL 更新任务的文件大小和MIME类型信息

### Requirement 2: 解析引擎选择和执行测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数能根据文档格式正确选择解析引擎并执行解析，以确保不同格式文档的正确处理。

#### Acceptance Criteria

1. WHEN 文档格式为 Markdown THEN 系统 SHALL 选择 MinerU 解析引擎
2. WHEN 解析引擎选择完成 THEN 系统 SHALL 更新任务的解析引擎信息
3. WHEN 开始解析执行 THEN 系统 SHALL 更新任务状态为对应的执行阶段（MinerUExecuting 或 MarkItDownExecuting）
4. WHEN 解析成功完成 THEN 系统 SHALL 返回包含 markdown_content 和 images 的 ParseResult
5. WHEN 解析过程中 THEN 系统 SHALL 将任务进度更新到 80%

### Requirement 3: 图片处理和路径替换测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数能正确处理Markdown中的图片引用，包括图片提取、上传和路径替换，以确保图片资源的正确处理。

#### Acceptance Criteria

1. WHEN Markdown内容包含图片引用 THEN 系统 SHALL 正确提取所有图片文件名
2. WHEN 开始图片处理 THEN 系统 SHALL 更新任务状态为 UploadingImages 阶段
3. WHEN 图片上传完成 THEN 系统 SHALL 更新任务状态为 ReplacingImagePaths 阶段
4. WHEN 图片路径替换完成 THEN 系统 SHALL 返回更新后的 Markdown 内容
5. WHEN 图片处理完成 THEN 系统 SHALL 将任务进度更新到 90%

### Requirement 4: OSS上传和数据持久化测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数能正确将处理后的Markdown内容上传到OSS并保存解析结果，以确保数据的持久化存储。

#### Acceptance Criteria

1. WHEN 图片处理完成 THEN 系统 SHALL 将处理后的Markdown内容上传到OSS
2. WHEN OSS上传成功 THEN 系统 SHALL 获得OSS URL并将进度更新到 95%
3. WHEN 开始保存解析结果 THEN 系统 SHALL 创建结构化文档对象
4. WHEN 结构化文档创建完成 THEN 系统 SHALL 包含正确的标题、章节数量和TOC信息
5. WHEN 所有数据保存完成 THEN 系统 SHALL 将任务进度更新到 100% 并设置状态为已完成

### Requirement 5: 任务状态管理和错误处理测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数能正确管理任务状态变化和处理各种错误情况，以确保系统的健壮性和可观测性。

#### Acceptance Criteria

1. WHEN 解析过程中的任何阶段 THEN 系统 SHALL 正确更新任务状态和进度
2. WHEN 解析成功完成 THEN 系统 SHALL 设置任务状态为 Completed 并记录处理时间
3. WHEN 解析过程中发生错误 THEN 系统 SHALL 设置任务状态为 Failed 并记录错误信息
4. WHEN 任务状态更新失败 THEN 系统 SHALL 记录警告日志但不影响主流程
5. WHEN 解析超时 THEN 系统 SHALL 返回超时错误并更新任务状态

### Requirement 6: 真实文件测试场景

**User Story:** 作为开发者，我希望使用真实的测试文件验证 `parse_document_internal` 函数的完整流程，以确保在实际使用场景中的正确性。

#### Acceptance Criteria

1. WHEN 使用 `/Volumes/soddygo/git_work/mcp_proxy/document-parser/fixtures/upload_parse_test.md` 文件 THEN 系统 SHALL 成功解析并返回正确的内容结构
2. WHEN 测试文件包含图片引用 THEN 系统 SHALL 正确识别 `test1.jpg`、`test2.png`、`test3.gif` 三个图片文件
3. WHEN 图片文件存在于 `fixtures/images` 目录 THEN 系统 SHALL 成功处理所有图片引用
4. WHEN 解析完成 THEN 系统 SHALL 生成包含正确标题层级和章节结构的结构化文档
5. WHEN 测试执行完成 THEN 系统 SHALL 验证所有预期的内容元素（标题、代码块、表格、列表）都被正确解析

### Requirement 7: 性能和并发测试

**User Story:** 作为开发者，我希望验证 `parse_document_internal` 函数在性能和并发场景下的表现，以确保系统的可扩展性。

#### Acceptance Criteria

1. WHEN 单个文档解析 THEN 系统 SHALL 在合理时间内完成（通常 < 5秒）
2. WHEN 多个任务并发执行 THEN 系统 SHALL 正确处理并发控制和资源管理
3. WHEN 解析大文件 THEN 系统 SHALL 正确处理内存使用和性能优化
4. WHEN 长时间运行 THEN 系统 SHALL 正确清理临时资源和避免内存泄漏
5. WHEN 系统负载较高 THEN 系统 SHALL 通过信号量控制并发数量