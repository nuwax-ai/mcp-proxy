# Document Parser 服务管理指南

本目录包含用于在 Linux 系统上管理 document-parser 服务的脚本和配置文件。

## 文件说明

- `document-parser.service` - systemd 服务配置文件模板（脚本会动态生成实际的服务文件）
- `document-parser-manager.sh` - 统一管理脚本，整合了部署和管理功能，支持动态路径配置
- `README.md` - 本说明文档

## 快速开始

### 1. 准备工作

确保你已经编译好了 `document-parser` 可执行文件，并且有相应的配置文件 `config.yml`。

### 2. 一键部署（推荐）

```bash
# 给管理脚本添加执行权限
chmod +x document-parser-manager.sh

# 自动部署服务到脚本所在目录（包含安装、启动、开机自启）
sudo ./document-parser-manager.sh deploy

# 或者部署到指定目录
sudo DOCUMENT_PARSER_INSTALL_DIR=/opt/document-parser ./document-parser-manager.sh deploy

# 仅部署但不启动服务
sudo ./document-parser-manager.sh deploy --no-start

# 部署但不启用开机自启
sudo ./document-parser-manager.sh deploy --no-enable
```

部署过程会：
- 将可执行文件复制到安装目录（默认为脚本所在目录）
- 复制配置文件到安装目录
- 动态生成并安装 systemd 服务文件
- 重新加载 systemd 配置
- 启动服务（除非使用 --no-start）
- 启用开机自启（除非使用 --no-enable）

### 3. 查看服务状态

```bash
# 查看服务状态
./document-parser-manager.sh status
```

## 完整命令列表

### document-parser-manager.sh 统一脚本

```bash
# 部署命令
sudo ./document-parser-manager.sh deploy           # 一键部署
sudo ./document-parser-manager.sh deploy --no-start    # 部署但不启动
sudo ./document-parser-manager.sh deploy --no-enable   # 部署但不启用开机自启

# 服务管理
sudo ./document-parser-manager.sh install      # 仅安装服务
sudo ./document-parser-manager.sh start        # 启动服务
sudo ./document-parser-manager.sh stop         # 停止服务
sudo ./document-parser-manager.sh restart      # 重启服务
./document-parser-manager.sh status            # 查看状态

# 开机自启管理
sudo ./document-parser-manager.sh enable       # 启用开机自启
sudo ./document-parser-manager.sh disable      # 禁用开机自启

# 其他操作
sudo ./document-parser-manager.sh uninstall    # 卸载服务
./document-parser-manager.sh logs              # 查看日志
./document-parser-manager.sh help              # 显示帮助
```

### 常用操作示例

#### 完整的服务部署流程

```bash
# 1. 编译项目（在项目根目录）
cargo build --release --bin document-parser

# 2. 复制可执行文件到脚本目录
cp target/release/document-parser scripts/

# 3. 复制配置文件到脚本目录（如果有）
cp document-parser/config.yml scripts/

# 4. 进入脚本目录
cd scripts/

# 5. 一键部署（推荐）
sudo ./document-parser-manager.sh deploy

# 或者手动步骤部署
sudo ./document-parser-manager.sh install
sudo ./document-parser-manager.sh start
sudo ./document-parser-manager.sh enable

# 6. 检查服务状态
./document-parser-manager.sh status
```

#### 更新服务

```bash
# 1. 停止服务
sudo ./document-parser-manager.sh stop

# 2. 复制新的可执行文件
cp ../target/release/document-parser /opt/document-parser/

# 3. 启动服务
sudo ./document-parser-manager.sh start

# 或者重新部署（会覆盖旧文件）
sudo ./document-parser-manager.sh deploy
```

#### 查看日志和调试

```bash
# 实时查看日志
./document-parser-manager.sh logs

# 查看最近的日志
journalctl -u document-parser -n 50

# 查看服务状态详情
systemctl status document-parser -l
```

## 服务配置说明

### 服务配置说明

服务配置文件会根据实际安装路径动态生成，包含以下主要配置：

- **工作目录**: 安装目录（默认为脚本所在目录）
- **可执行文件**: `{安装目录}/document-parser`
- **自动重启**: 服务异常退出时会自动重启
- **日志记录**: 使用 systemd journal 记录日志
- **安全配置**: 启用了多项安全限制
- **资源限制**: 设置了文件描述符和进程数限制
- **动态配置**: 根据 `DOCUMENT_PARSER_INSTALL_DIR` 环境变量或脚本所在目录生成

### 环境变量

服务默认设置了以下环境变量：
- `RUST_LOG=info` - 设置日志级别
- `CONFIG_PATH={安装目录}/config.yml` - 配置文件路径（动态生成）

部署时可以通过以下环境变量控制：
- `DOCUMENT_PARSER_INSTALL_DIR` - 指定安装目录（默认为脚本所在目录）

可以通过编辑 `/etc/systemd/system/document-parser.service` 文件来修改这些配置。

## 故障排除

### 常见问题

1. **服务启动失败**
   ```bash
   # 查看详细错误信息
   systemctl status document-parser -l
   journalctl -u document-parser -n 20
   ```

2. **权限问题**
   ```bash
   # 确保可执行文件有执行权限
   chmod +x /opt/document-parser/document-parser
   ```

3. **配置文件问题**
   ```bash
   # 检查配置文件是否存在和格式正确
   ls -la /opt/document-parser/config.yml
   ```

4. **端口占用**
   ```bash
   # 检查端口是否被占用
   netstat -tlnp | grep :端口号
   ```

### 手动操作

如果脚本出现问题，也可以手动使用 systemctl 命令：

```bash
# 手动启动服务
sudo systemctl start document-parser

# 手动停止服务
sudo systemctl stop document-parser

# 手动重启服务
sudo systemctl restart document-parser

# 查看服务状态
systemctl status document-parser

# 启用开机自启
sudo systemctl enable document-parser

# 禁用开机自启
sudo systemctl disable document-parser
```

## 安全注意事项

1. **文件权限**: 确保只有 root 用户可以修改服务文件
2. **配置安全**: 不要在配置文件中存储明文密码
3. **网络安全**: 根据需要配置防火墙规则
4. **日志安全**: 定期清理日志文件，避免磁盘空间不足

## 卸载服务

如果需要完全移除服务：

```bash
# 卸载服务（会停止服务、禁用自启、删除文件）
sudo ./document-parser-manager.sh uninstall
```

这会完全清理所有相关文件和配置。