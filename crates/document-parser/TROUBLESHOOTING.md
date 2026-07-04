# Document Parser 故障排除指南

本指南提供了Document Parser服务常见问题的详细解决方案，特别是关于虚拟环境和依赖管理的问题。

## 📋 目录

1. [FlashInfer编译失败](#flashinfer编译失败)
2. [虚拟环境问题](#虚拟环境问题)
3. [依赖安装问题](#依赖安装问题)
4. [网络和下载问题](#网络和下载问题)
5. [系统环境问题](#系统环境问题)
6. [常用诊断命令](#常用诊断命令)
7. [获取帮助](#获取帮助)

## 🚀 FlashInfer编译失败

### 问题: FlashInfer CUDA内核编译失败

**症状:**
- MinerU启动时出现 `fatal error: math.h: 没有那个文件或目录` 错误
- 错误发生在CUDA图捕获阶段
- FlashInfer ninja构建失败

**原因:**
系统缺少C标准库开发头文件，导致FlashInfer无法编译CUDA内核。这通常发生在Ubuntu 24.04等较新系统上。

**诊断步骤:**
```bash
# 检查系统头文件是否存在
ls -la /usr/include/math.h
ls -la /usr/include/c++/13/cmath

# 检查构建工具版本
gcc --version
ninja --version
nvcc --version
```

**解决方案:**

**步骤1: 安装缺失的开发包**
```bash
# 安装C标准库开发包
sudo apt install -y libc6-dev libm-dev

# 安装C++标准库开发包
sudo apt install -y libstdc++-13-dev

# 安装其他可能需要的包
sudo apt install -y build-essential ninja-build cmake
```

**步骤2: 设置CUDA环境变量**
```bash
# 设置CUDA相关环境变量
export CUDA_HOME=/usr/local/cuda
export PATH=$CUDA_HOME/bin:$PATH
export LD_LIBRARY_PATH=$CUDA_HOME/lib64:$LD_LIBRARY_PATH

# 将这些环境变量添加到 ~/.bashrc 或 ~/.zshrc
echo 'export CUDA_HOME=/usr/local/cuda' >> ~/.bashrc
echo 'export PATH=$CUDA_HOME/bin:$PATH' >> ~/.bashrc
echo 'export LD_LIBRARY_PATH=$CUDA_HOME/lib64:$LD_LIBRARY_PATH' >> ~/.bashrc
```

**步骤3: 清理缓存并重新安装**
```bash
# 清理FlashInfer缓存
rm -rf ~/.cache/flashinfer

# 重新安装MinerU（确保包含正确的sglang版本）
pip uninstall mineru sglang -y
pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple
```

**验证修复:**
```bash
# 检查头文件是否存在
ls -la /usr/include/math.h
ls -la /usr/include/c++/13/cmath

# 检查CUDA头文件
ls -la /usr/local/cuda/include/cuda_runtime.h

# 重新启动服务测试
document-parser server
```

**如果问题仍然存在:**
```bash
# 尝试使用系统ninja而不是虚拟环境中的ninja
which ninja
# 如果显示虚拟环境路径，使用系统ninja
sudo apt install -y ninja-build
export PATH=/usr/bin:$PATH

# 或者尝试禁用CUDA图功能（性能会下降）
# 在启动MinerU时添加 --disable-cuda-graph 参数
```

## 🏠 虚拟环境问题

### 问题1: 虚拟环境创建失败

**症状:**
- `document-parser uv-init` 失败
- 错误信息包含 "权限拒绝" 或 "Permission denied"
- 无法在当前目录创建 `venv` 文件夹

**诊断步骤:**
```bash
# 检查当前目录权限
ls -la          # Linux/macOS
dir             # Windows

# 检查磁盘空间
df -h .         # Linux/macOS
dir             # Windows

# 检查是否存在同名文件
ls -la venv     # Linux/macOS
dir venv        # Windows
```

**解决方案:**

**Linux/macOS:**
```bash
# 修改目录权限
chmod 755 .

# 修改目录所有者
chown $USER .

# 删除现有的venv文件（如果存在）
rm -rf ./venv

# 重新初始化
document-parser uv-init
```

**Windows:**
```cmd
# 以管理员身份运行命令提示符
# 删除现有的venv目录
rmdir /s .\venv

# 重新初始化
document-parser uv-init
```

### 问题2: 虚拟环境激活失败

**症状:**
- 激活命令执行后没有效果
- 命令提示符没有显示 `(venv)` 前缀
- Python路径仍指向系统Python

**解决方案:**

**Linux/macOS (Bash/Zsh):**
```bash
# 标准激活方式
source ./venv/bin/activate

# 检查是否激活成功
which python
python --version
```

**Linux/macOS (Fish Shell):**
```bash
# Fish shell激活方式
source ./venv/bin/activate.fish
```

**Windows (CMD):**
```cmd
# 激活虚拟环境
.\venv\Scripts\activate

# 检查是否激活成功
where python
python --version
```

**Windows (PowerShell):**
```powershell
# 如果遇到执行策略限制
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser

# 激活虚拟环境
.\venv\Scripts\Activate.ps1

# 检查是否激活成功
Get-Command python
python --version
```

### 问题3: 虚拟环境路径问题

**症状:**
- 找不到Python可执行文件
- 路径指向错误的位置
- 跨平台兼容性问题

**解决方案:**
```bash
# 检查虚拟环境结构
ls -la ./venv/bin/     # Linux/macOS
dir .\venv\Scripts\    # Windows

# 手动验证Python路径
./venv/bin/python --version      # Linux/macOS
.\venv\Scripts\python --version  # Windows

# 如果路径错误，重新创建虚拟环境
rm -rf ./venv                    # Linux/macOS
rmdir /s .\venv                  # Windows
document-parser uv-init
```

## 📦 依赖安装问题

### 问题1: UV工具未安装或不可用

**症状:**
- `uv: command not found`
- UV版本过旧
- UV安装路径问题

**解决方案:**

**官方安装脚本 (推荐):**
```bash
# Linux/macOS
curl -LsSf https://astral.sh/uv/install.sh | sh

# 重启终端或重新加载shell配置
source ~/.bashrc    # 或 ~/.zshrc
```

**包管理器安装:**
```bash
# macOS
brew install uv

# 通过pip安装
pip install uv

# Windows (winget)
winget install astral-sh.uv
```

**验证安装:**
```bash
uv --version
which uv    # Linux/macOS
where uv    # Windows
```

### 问题2: MinerU或MarkItDown安装失败

**症状:**
- 包下载超时
- 编译错误
- 依赖冲突

**诊断步骤:**
```bash
# 检查网络连接
ping pypi.org

# 检查Python版本
python --version

# 检查虚拟环境中的pip
./venv/bin/pip --version      # Linux/macOS
.\venv\Scripts\pip --version  # Windows
```

**解决方案:**

**使用国内镜像源:**
```bash
# 清华大学镜像源
uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]
uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ markitdown

# 阿里云镜像源
uv pip install -i https://mirrors.aliyun.com/pypi/simple/ mineru[core]
```

**分步安装:**
```bash
# 1. 升级pip
uv pip install --upgrade pip

# 2. 安装基础依赖
uv pip install wheel setuptools

# 3. 分别安装包
uv pip install mineru[core]
uv pip install markitdown
```

**增加超时时间:**
```bash
uv pip install --timeout 300 mineru[core]
```

**清理缓存后重试:**
```bash
uv cache clean
document-parser uv-init
```

## 🌐 网络和下载问题

### 问题1: 网络连接超时

**症状:**
- 下载包时超时
- 连接PyPI失败
- DNS解析问题

**解决方案:**

**检查网络连接:**
```bash
# 测试基本连接
ping pypi.org
ping pypi.tuna.tsinghua.edu.cn

# 测试HTTPS连接
curl -I https://pypi.org/simple/
```

**配置代理 (如果需要):**
```bash
# 设置HTTP代理
export HTTP_PROXY=http://proxy.company.com:8080
export HTTPS_PROXY=http://proxy.company.com:8080

# Windows
set HTTP_PROXY=http://proxy.company.com:8080
set HTTPS_PROXY=http://proxy.company.com:8080
```

**使用镜像源:**
```bash
# 配置uv使用镜像源
uv pip install --index-url https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]
```

### 问题2: 防火墙或安全软件阻止

**解决方案:**
- 临时关闭防火墙进行测试
- 将Python和uv添加到防火墙白名单
- 检查企业网络策略
- 联系网络管理员获取帮助

## ⚙️ 系统环境问题

### 问题1: Python版本不兼容

**症状:**
- Python版本低于3.8
- 缺少必要的Python模块
- 系统Python与虚拟环境冲突

**检查Python版本:**
```bash
python --version
python3 --version
```

**解决方案:**

**Linux (Ubuntu/Debian):**
```bash
# 安装Python 3.11
sudo apt update
sudo apt install python3.11 python3.11-venv python3.11-pip

# 使用特定版本创建虚拟环境
python3.11 -m venv ./venv
```

**macOS:**
```bash
# 使用Homebrew安装
brew install python@3.11

# 或使用pyenv管理多版本
brew install pyenv
pyenv install 3.11.0
pyenv local 3.11.0
```

**Windows:**
- 从 [python.org](https://python.org) 下载并安装Python 3.11+
- 确保勾选 "Add Python to PATH"
- 重启命令提示符

### 问题2: CUDA环境配置 (可选)

**检查CUDA:**
```bash
nvidia-smi
nvcc --version
```

**安装CUDA (如果需要GPU加速):**
- 安装NVIDIA驱动程序
- 下载并安装CUDA Toolkit (推荐11.8或12.x)
- 验证安装: `nvidia-smi` 和 `nvcc --version`

**注意:** CPU模式也可正常工作，GPU仅用于加速。

## 🔍 常用诊断命令

### 环境检查命令

```bash
# 完整环境检查
document-parser check

# 详细故障排除指南
document-parser troubleshoot

# 重新初始化环境
document-parser uv-init
```

### 手动验证命令

```bash
# 检查工具版本
uv --version
python --version

# 检查虚拟环境
./venv/bin/python --version      # Linux/macOS
.\venv\Scripts\python --version  # Windows

# 检查已安装的包
./venv/bin/pip list              # Linux/macOS
.\venv\Scripts\pip list          # Windows

# 测试MinerU
./venv/bin/mineru --help         # Linux/macOS
.\venv\Scripts\mineru --help     # Windows

# 测试MarkItDown
./venv/bin/python -m markitdown --help      # Linux/macOS
.\venv\Scripts\python -m markitdown --help  # Windows
```

### 日志查看

```bash
# 查看当天日志 (Linux/macOS)
tail -f logs/log.$(date +%Y-%m-%d)

# 查看最新日志
ls -la logs/
tail -f logs/log.*

# Windows
dir logs\
type logs\log.%date:~0,10%
```

### 清理和重置

```bash
# 清理UV缓存
uv cache clean

# 完全重置虚拟环境
rm -rf ./venv                    # Linux/macOS
rmdir /s .\venv                  # Windows
document-parser uv-init

# 清理日志文件
rm -rf logs/*                    # Linux/macOS
del /q logs\*                    # Windows
```

## 🆘 获取帮助

### 自助诊断步骤

1. **运行诊断命令:**
   ```bash
   document-parser check
   document-parser troubleshoot
   ```

2. **收集系统信息:**
   - 操作系统版本
   - Python版本
   - 当前工作目录
   - 完整的错误消息

3. **检查日志文件:**
   ```bash
   ls -la logs/
   tail -100 logs/log.*
   ```

4. **尝试在新目录中测试:**
   ```bash
   mkdir test-document-parser
   cd test-document-parser
   document-parser uv-init
   ```

### 常见解决方案总结

| 问题类型 | 快速解决方案 |
|---------|-------------|
| 权限问题 | `chmod 755 .` (Linux/macOS) 或以管理员身份运行 (Windows) |
| 网络问题 | 使用镜像源: `-i https://pypi.tuna.tsinghua.edu.cn/simple/` |
| 虚拟环境损坏 | `rm -rf ./venv && document-parser uv-init` |
| UV未安装 | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| Python版本过旧 | 安装Python 3.8+ |
| 磁盘空间不足 | 清理磁盘，确保至少500MB可用空间 |

### 最后的建议

如果所有方法都无法解决问题：

1. **完全重新开始:**
   ```bash
   # 创建新的工作目录
   mkdir fresh-document-parser
   cd fresh-document-parser
   
   # 重新初始化
   document-parser uv-init
   ```

2. **检查系统限制:**
   - 企业网络策略
   - 防病毒软件设置
   - 磁盘配额限制
   - 用户权限限制

3. **寻求帮助时提供:**
   - 完整的错误消息
   - 系统信息 (`uname -a` 或 `systeminfo`)
   - Python版本 (`python --version`)
   - 执行的完整命令序列
   - 相关日志文件内容

---

**记住:** 大多数问题都可以通过重新运行 `document-parser uv-init` 来解决。这个命令会自动检测和修复常见的环境问题。