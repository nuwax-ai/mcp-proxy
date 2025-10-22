# SSE 路径配置测试

## 当前问题
- 路由注册：`/mcp/sse/proxy/baidu-ocr-test-id2`
- SSE 路径：`/mcp/sse/proxy/baidu-ocr-test-id2/sse`
- Message 路径：`/mcp/sse/proxy/baidu-ocr-test-id2/message`
- 结果：404 错误

## 可能的解决方案

### 方案1：相对路径配置
```rust
let config = SseServerConfig {
    bind: addr.parse()?,
    sse_path: "/sse".to_string(),
    post_path: "/message".to_string(),
    // ...
};
```

### 方案2：绝对路径配置
```rust
let config = SseServerConfig {
    bind: addr.parse()?,
    sse_path: sse_path.sse_path.clone(),
    post_path: sse_path.message_path.clone(),
    // ...
};
```

### 方案3：路由嵌套
```rust
let nested_router = axum::Router::new()
    .nest(&format!("/{}", mcp_id), sse_router);
```

## 需要从官方 SDK 了解的信息
1. SSE 服务器的路径配置是相对路径还是绝对路径？
2. 路由注册时应该注册到什么级别的路径？
3. 是否需要特殊的路径重写逻辑？