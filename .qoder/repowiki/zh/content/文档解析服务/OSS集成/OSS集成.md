# OSS集成

<cite>
**本文档引用的文件**   
- [oss_service.rs](file://document-parser/src/services/oss_service.rs)
- [oss_data.rs](file://document-parser/src/models/oss_data.rs)
- [private_oss_handler.rs](file://document-parser/src/handlers/private_oss_handler.rs)
- [config.yml](file://document-parser/config.yml)
- [lib.rs](file://oss-client/src/lib.rs)
- [private_client.rs](file://oss-client/src/private_client.rs)
- [如何使用OSS签名URL上传文件.md](file://document-parser/如何使用OSS签名URL上传文件.md)
</cite>

## 目录
1. [OSS服务核心功能](#oss服务核心功能)
2. [OSS数据结构](#oss数据结构)
3. [OSS配置说明](#oss配置说明)
4. [私有OSS资源代理访问](#私有oss资源代理访问)
5. [签名URL生成与文件上传流程](#签名url生成与文件上传流程)
6. [高级功能实现](#高级功能实现)

## OSS服务核心功能

`OssService` 是文档解析服务与阿里云OSS集成的核心组件，封装了所有与OSS交互的核心操作。该服务通过 `aliyun_oss_rust_sdk` 库与OSS进行通信，提供了上传、下载、生成签名URL、删除文件等关键功能。

`OssService` 的主要职责包括：
- **文件上传**：支持上传本地文件和内存中的内容，自动检测MIME类型，并根据文件大小决定上传策略。
- **文件下载**：提供将OSS文件下载到本地临时目录或指定路径的功能。
- **签名URL生成**：生成用于上传和下载的预签名URL，实现安全的临时访问。
- **批量操作**：支持批量上传文件，并提供进度回调机制。
- **连接验证**：在服务初始化时验证与OSS的连接是否正常。

该服务通过 `OssServiceConfig` 结构体进行配置，允许自定义最大并发上传数、重试次数、超时时间等参数，以适应不同的使用场景。

**Section sources**
- [oss_service.rs](file://document-parser/src/services/oss_service.rs#L0-L799)

## OSS数据结构

`oss_data.rs` 文件定义了与OSS操作相关的数据结构，用于在应用程序内部承载OSS元数据和访问凭证。

`OssData` 结构体是核心数据模型，包含以下字段：
- `markdown_url`: 解析后的Markdown文件在OSS上的公开访问URL。
- `markdown_object_key`: Markdown文件在OSS中的对象键名。
- `images`: 一个 `ImageInfo` 结构体的数组，包含所有上传图片的详细信息。
- `bucket`: 存储桶名称。

`ImageInfo` 结构体用于描述单个图片的信息，包含原始路径、文件名、OSS对象键、URL、文件大小、MIME类型以及可选的尺寸信息。该结构体提供了便捷的构造方法和辅助函数，如 `get_formatted_size` 用于格式化文件大小，`filename_matches` 用于检查文件名匹配。

**Section sources**
- [oss_data.rs](file://document-parser/src/models/oss_data.rs#L0-L122)

## OSS配置说明

OSS服务的配置主要在 `config.yml` 文件的 `storage.oss` 部分进行。以下是关键配置项的说明：

- `endpoint`: OSS服务的接入点，例如 `oss-rg-china-mainland.aliyuncs.com`。
- `public_bucket`: 公共存储桶名称，用于存储公开访问的文件，如文档文件。
- `private_bucket`: 私有存储桶名称，用于存储需要权限控制的文件，如模型文件。
- `access_key_id`: 阿里云访问密钥ID，用于身份验证。建议通过环境变量 `OSS_ACCESS_KEY_ID` 设置。
- `access_key_secret`: 阿里云访问密钥密钥，用于身份验证。建议通过环境变量 `OSS_ACCESS_KEY_SECRET` 设置。
- `region`: OSS区域，例如 `oss-rg-china-mainland`。
- `upload_directory`: 上传文件的统一子目录前缀，例如 `document_parser`。

为了安全起见，`access_key_id` 和 `access_key_secret` 应通过环境变量设置，而不是直接写在配置文件中。配置文件中使用 `${OSS_ACCESS_KEY_ID}` 和 `${OSS_ACCESS_KEY_SECRET}` 作为占位符。

**Section sources**
- [config.yml](file://document-parser/config.yml#L0-L77)

## 私有OSS资源代理访问

`private_oss_handler.rs` 文件定义了处理私有OSS资源代理访问的API端点。这些端点通过 `private_oss_client` 与OSS进行交互，实现了安全的权限验证。

主要API端点包括：
- `POST /api/v1/oss/upload`: 上传文件到OSS。服务端接收文件，上传到OSS，并返回包含下载URL的响应。
- `GET /api/v1/oss/upload-sign-url`: 获取上传签名URL。客户端可以使用此URL直接上传文件到OSS，无需经过服务器中转。
- `GET /api/v1/oss/download-sign-url`: 获取下载签名URL。客户端可以使用此URL直接下载OSS文件。
- `GET /api/v1/oss/delete`: 删除OSS文件。

这些端点在处理请求时，首先检查 `private_oss_client` 是否已配置，然后验证请求参数，最后调用相应的OSS操作。例如，`get_upload_sign_url` 端点会验证文件名是否为空，然后生成一个4小时有效的上传签名URL。

**Section sources**
- [private_oss_handler.rs](file://document-parser/src/handlers/private_oss_handler.rs#L0-L483)

## 签名URL生成与文件上传流程

基于“如何使用OSS签名URL上传文件.md”文档，完整的签名URL生成与文件上传流程如下：

1.  **获取上传签名URL**：客户端向 `GET /api/v1/oss/upload-sign-url` 接口发送请求，提供文件名和内容类型。服务端返回一个包含 `upload_url` 的JSON响应，该URL是一个预签名的PUT请求URL，有效期为4小时。
2.  **直接上传文件**：客户端使用返回的 `upload_url`，通过HTTP PUT方法直接将文件内容上传到OSS。请求头中必须包含正确的 `Content-Type`。
3.  **上传成功**：如果上传成功，OSS会返回200状态码。文件将被存储在指定的存储桶和路径下。
4.  **获取下载URL**：上传成功后，客户端可以调用 `GET /api/v1/oss/download-sign-url` 接口获取下载签名URL，用于后续下载文件。

此流程的优势在于，文件上传不经过应用服务器，直接从客户端到OSS，减少了服务器的带宽压力和处理负担。

**Section sources**
- [如何使用OSS签名URL上传文件.md](file://document-parser/如何使用OSS签名URL上传文件.md#L0-L217)

## 高级功能实现

OSS服务还实现了以下高级功能：

- **大文件分片上传**：虽然当前代码中大文件上传仍使用简单上传，但已预留了分片上传的接口。未来可以通过 `aliyun_oss_rust_sdk` 的分片上传API实现，以支持超大文件的稳定上传。
- **断点续传**：结合分片上传功能，可以实现断点续传。客户端在上传中断后，可以查询已上传的分片，然后从断点处继续上传。
- **上传进度监控**：`upload_batch` 方法支持通过 `ProgressCallback` 回调函数监控批量上传的进度。客户端可以实现此回调，以实时显示上传进度。

这些高级功能的实现，使得OSS服务能够满足更复杂和高性能的文件存储需求。

**Section sources**
- [oss_service.rs](file://document-parser/src/services/oss_service.rs#L0-L799)
- [private_client.rs](file://oss-client/src/private_client.rs#L0-L218)
- [lib.rs](file://oss-client/src/lib.rs#L0-L158)