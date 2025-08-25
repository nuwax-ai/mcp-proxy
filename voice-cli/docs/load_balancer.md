# VoiceCliLoadBalancer

VoiceCliLoadBalancer 是 voice-cli 集群架构中的核心组件，提供了完整的负载均衡、健康检查和服务发现功能。

## 功能特性

### 🔄 自动节点发现
- 自动从元数据存储中发现集群节点
- 实时监控节点状态变化
- 支持动态节点加入和离开

### 🎯 智能请求路由
- 轮询算法路由到健康的 leader 节点
- 自动跳过不健康的节点
- 支持请求重试和故障转移

### 💓 健康检查
- 定期检查所有集群节点的健康状态
- 可配置的检查间隔和超时时间
- 支持自定义健康检查端点

### 🔌 熔断器机制
- 自动检测连续失败的节点
- 临时隔离不健康的节点
- 支持自动恢复和重试

### 📊 监控和统计
- 实时路由统计信息
- 节点响应时间监控
- 熔断器状态跟踪

## 架构组件

### VoiceCliLoadBalancer
主要的负载均衡器类，协调所有子组件：
- `LoadBalancerService`: HTTP 代理服务
- `HealthChecker`: 健康检查器
- `ServiceManager`: 服务管理器

### LoadBalancerService
HTTP 代理服务，负责：
- 接收客户端请求
- 选择健康的后端节点
- 转发请求并返回响应
- 提供负载均衡器状态端点

### HealthChecker
健康检查器，负责：
- 定期检查节点健康状态
- 更新元数据存储中的节点状态
- 发送健康状态变化事件
- 管理熔断器状态

### ServiceManager
服务管理器，负责：
- 服务注册和发现
- 节点生命周期管理
- 集群拓扑变化通知

## 使用方法

### 基本使用

```rust
use std::sync::Arc;
use voice_cli::{
    models::{LoadBalancerConfig, MetadataStore},
    load_balancer::VoiceCliLoadBalancer,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化元数据存储
    let metadata_store = Arc::new(MetadataStore::new("./metadata.db")?);
    
    // 配置负载均衡器
    let config = LoadBalancerConfig {
        enabled: true,
        bind_address: "0.0.0.0".to_string(),
        port: 8090,
        health_check_interval: 5,
        health_check_timeout: 3,
        pid_file: "./voice-cli-lb.pid".to_string(),
        log_file: "./logs/lb.log".to_string(),
    };
    
    // 创建并启动负载均衡器
    let mut load_balancer = VoiceCliLoadBalancer::new(config, metadata_store).await?;
    load_balancer.start().await?;
    
    Ok(())
}
```

### CLI 使用

```bash
# 启动负载均衡器（前台模式）
voice-cli lb run --port 8090

# 启动负载均衡器（后台模式）
voice-cli lb start --port 8090

# 停止负载均衡器
voice-cli lb stop

# 重启负载均衡器
voice-cli lb restart --port 8090

# 查看负载均衡器状态
voice-cli lb status
```

## 配置选项

### LoadBalancerConfig

```yaml
load_balancer:
  enabled: true                    # 是否启用负载均衡器
  bind_address: "0.0.0.0"         # 绑定地址
  port: 8090                      # 监听端口
  health_check_interval: 5        # 健康检查间隔（秒）
  health_check_timeout: 3         # 健康检查超时（秒）
  pid_file: "./voice-cli-lb.pid"  # PID 文件路径
  log_file: "./logs/lb.log"       # 日志文件路径
```

### 环境变量

```bash
# 负载均衡器配置
export VOICE_CLI_LB_ENABLED=true
export VOICE_CLI_LB_PORT=8090
export VOICE_CLI_LB_BIND_ADDRESS=0.0.0.0
export VOICE_CLI_LB_HEALTH_CHECK_INTERVAL=5
export VOICE_CLI_LB_HEALTH_CHECK_TIMEOUT=3
```

## API 端点

### 负载均衡器状态
- `GET /health` - 负载均衡器健康检查
- `GET /lb/status` - 集群状态信息
- `GET /lb/stats` - 负载均衡统计信息

### 代理转发
- `/*` - 所有其他请求转发到健康的 leader 节点

## 监控指标

### 路由统计
- `total_requests`: 总请求数
- `successful_requests`: 成功请求数
- `failed_requests`: 失败请求数
- `requests_per_node`: 每个节点的请求数
- `avg_response_time_per_node`: 每个节点的平均响应时间
- `circuit_breaker_activations`: 熔断器激活次数

### 集群状态
- `total_nodes`: 总节点数
- `healthy_nodes`: 健康节点数
- `leader_node`: 当前 leader 节点 ID

### 熔断器状态
- `activated_at`: 激活时间
- `failure_count`: 失败次数
- `timeout_duration`: 超时时长

## 故障处理

### 节点故障
1. 健康检查器检测到节点不响应
2. 更新节点状态为不健康
3. 从路由表中移除该节点
4. 激活熔断器（如果连续失败超过阈值）

### 网络分区
1. 负载均衡器继续服务可达的节点
2. 不可达的节点被标记为不健康
3. 网络恢复后自动重新加入路由表

### 全部节点故障
1. 返回 503 Service Unavailable
2. 包含 Retry-After 头部
3. 继续尝试健康检查等待恢复

## 性能优化

### 连接池
- 使用连接池减少连接开销
- 支持 HTTP/1.1 keep-alive
- 自动管理连接生命周期

### 缓存
- 缓存健康节点列表
- 缓存 leader 节点信息
- 定期刷新缓存数据

### 异步处理
- 所有 I/O 操作都是异步的
- 并发处理多个健康检查
- 非阻塞的请求转发

## 安全考虑

### 访问控制
- 支持基于 IP 的访问控制
- 可配置的请求速率限制
- 防止 DDoS 攻击

### 数据保护
- 不记录敏感请求数据
- 安全的错误信息处理
- 支持 HTTPS 代理

## 故障排除

### 常见问题

1. **负载均衡器无法启动**
   - 检查端口是否被占用
   - 验证配置文件格式
   - 查看日志文件获取详细错误信息

2. **节点无法被发现**
   - 确认元数据存储连接正常
   - 检查节点是否正确注册
   - 验证网络连通性

3. **健康检查失败**
   - 检查节点的健康检查端点
   - 验证超时配置是否合理
   - 确认防火墙设置

### 日志分析

负载均衡器提供详细的日志信息：
- 节点发现和状态变化
- 健康检查结果
- 请求路由决策
- 熔断器状态变化

## 示例

查看 `examples/load_balancer_example.rs` 获取完整的使用示例。

## 测试

运行负载均衡器相关测试：

```bash
# 运行所有负载均衡器测试
cargo test load_balancer

# 运行特定测试
cargo test voice_cli_load_balancer
```