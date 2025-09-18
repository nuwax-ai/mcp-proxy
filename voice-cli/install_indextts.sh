#!/bin/bash

# IndexTTS 环境安装部署脚本
# 基于 https://github.com/index-tts/index-tts 官方文档
# 更新时间：2025-09-02

set -e  # 遇到错误立即退出

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 日志函数
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 检查系统要求
check_requirements() {
    log_info "检查系统要求..."
    
    # 检查操作系统
    if [[ "$OSTYPE" == "darwin"* ]]; then
        OS="macos"
        log_info "检测到 macOS 系统"
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        OS="linux"
        log_info "检测到 Linux 系统"
    else
        log_error "不支持的操作系统: $OSTYPE"
        exit 1
    fi
    
    # 检查 Homebrew (macOS) 或包管理器
    if [[ "$OS" == "macos" ]]; then
        if ! command -v brew &> /dev/null; then
            log_error "Homebrew 未安装，请先安装 Homebrew"
            exit 1
        fi
    fi
    
    # 检查 Python 版本
    if ! command -v python3.10 &> /dev/null; then
        log_warning "Python 3.10 未安装，正在安装..."
        if [[ "$OS" == "macos" ]]; then
            brew install python@3.10
        else
            log_error "请手动安装 Python 3.10"
            exit 1
        fi
    fi
    
    # 验证 Python 版本
    PYTHON_VERSION=$(python3.10 --version 2>&1 | cut -d' ' -f2)
    if [[ ! "$PYTHON_VERSION" =~ ^3\.10\. ]]; then
        log_error "Python 版本必须是 3.10.x，当前版本: $PYTHON_VERSION"
        exit 1
    fi
    
    # 检查 Git
    if ! command -v git &> /dev/null; then
        log_error "Git 未安装"
        exit 1
    fi
    
    # 检查 FFmpeg
    if ! command -v ffmpeg &> /dev/null; then
        log_warning "FFmpeg 未安装，正在安装..."
        if [[ "$OS" == "macos" ]]; then
            brew install ffmpeg
        else
            sudo apt-get update && sudo apt-get install -y ffmpeg
        fi
    fi
    
    log_success "系统要求检查完成"
}

# 安装 Python 包管理器
install_package_managers() {
    log_info "安装包管理器..."
    
    # 安装 uv
    if ! command -v uv &> /dev/null; then
        log_info "安装 uv 包管理器..."
        curl -LsSf https://astral.sh/uv/install.sh | sh
        export PATH="$HOME/.local/bin:$PATH"
        log_success "uv 安装完成"
    else
        log_info "uv 已安装"
    fi
    
    # 验证 uv 安装
    if command -v uv &> /dev/null; then
        UV_VERSION=$(uv --version)
        log_info "uv 版本: $UV_VERSION"
    fi
    
    log_success "包管理器安装完成"
}

# 创建虚拟环境
create_virtualenv() {
    log_info "创建 Python 3.10 虚拟环境..."
    
    # 删除现有虚拟环境（如果存在）
    if [[ -d ".venv" ]]; then
        log_warning "删除现有虚拟环境"
        rm -rf .venv
    fi
    
    # 创建新虚拟环境
    python3.10 -m venv .venv
    source .venv/bin/activate
    
    log_success "虚拟环境创建完成"
}

# 初始化 uv 项目
init_uv_project() {
    log_info "初始化 uv 项目..."
    
    source .venv/bin/activate
    
    # 创建 pyproject.toml
    cat > pyproject.toml << 'EOF'
[project]
name = "voice-cli-tts"
version = "0.1.0"
description = "TTS dependencies for voice-cli"
requires-python = ">=3.10,<3.11"
dependencies = [
    "torch>=2.8",
    "torchaudio>=2.8",
    "numpy>=1.19.0,<2.0.0",
    "soundfile>=0.12",
    "huggingface-hub>=0.34.4",
]

[tool.uv]
dev-dependencies = []

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
EOF
    
    # 同步依赖
    uv sync
    
    log_success "uv 项目初始化完成"
}

# 安装 PyTorch
install_pytorch() {
    log_info "安装 PyTorch..."
    
    source .venv/bin/activate
    
    # 使用 uv 安装 PyTorch
    uv add torch torchaudio --index-url https://download.pytorch.org/whl/cpu
    
    log_success "PyTorch 安装完成"
}

# 安装 IndexTTS
install_indextts() {
    log_info "安装 IndexTTS..."
    
    source .venv/bin/activate
    
    # 克隆 IndexTTS 仓库
    if [[ -d "/tmp/index-tts" ]]; then
        rm -rf /tmp/index-tts
    fi
    
    log_info "克隆 IndexTTS 仓库..."
    git clone https://github.com/index-tts/index-tts.git /tmp/index-tts
    
    # 安装 IndexTTS
    log_info "安装 IndexTTS 包..."
    cd /tmp/index-tts
    pip install -e .
    
    # 处理可能的 pynini 问题
    if pip install pynini 2>/dev/null; then
        log_info "pynini 安装成功"
    else
        log_warning "pynini 安装失败，尝试通过 conda 安装..."
        # 如果 conda 可用，尝试使用 conda
        if command -v conda &> /dev/null; then
            conda install -c conda-forge pynini==2.1.6
            pip install WeTextProcessing --no-deps
        else
            log_warning "conda 不可用，跳过 pynini 安装"
        fi
    fi
    
    cd - > /dev/null
    
    log_success "IndexTTS 安装完成"
}

# 下载模型
download_models() {
    log_info "下载 IndexTTS 模型..."
    
    source .venv/bin/activate
    
    # 确保已安装 huggingface-hub
    uv add huggingface-hub
    
    # 创建 checkpoints 目录
    mkdir -p checkpoints
    
    # 下载模型文件
    log_info "正在下载 IndexTTS-1.5 模型文件..."
    
    # 先尝试下载小文件
    python -c "
from huggingface_hub import hf_hub_download
import os

files_to_download = [
    'config.yaml',
    'bpe.model',
    'unigram_12000.vocab',
    'README.md',
    '.gitattributes'
]

for file in files_to_download:
    try:
        hf_hub_download(
            repo_id='IndexTeam/IndexTTS-1.5',
            filename=file,
            local_dir='checkpoints'
        )
        print(f'✓ {file} 下载完成')
    except Exception as e:
        print(f'✗ {file} 下载失败: {e}')
"
    
    # 下载大模型文件
    log_info "下载大模型文件（可能需要较长时间）..."
    
    python -c "
from huggingface_hub import hf_hub_download
import os

large_files = [
    'gpt.pth',
    'bigvgan_generator.pth',
    'dvae.pth'
]

for file in large_files:
    try:
        print(f'正在下载 {file}...')
        hf_hub_download(
            repo_id='IndexTeam/IndexTTS-1.5',
            filename=file,
            local_dir='checkpoints'
        )
        print(f'✓ {file} 下载完成')
    except Exception as e:
        print(f'✗ {file} 下载失败: {e}')
        print(f'请手动下载: https://huggingface.co/IndexTeam/IndexTTS-1.5/blob/main/{file}')
"
    
    # 验证关键文件
    log_info "验证模型文件..."
    required_files = ['config.yaml', 'gpt.pth', 'bigvgan_generator.pth']
    missing_files = []
    
    for file in required_files; do
        if [[ ! -f "checkpoints/$file" ]]; then
            missing_files+=("$file")
        fi
    done
    
    if [[ ${#missing_files[@]} -gt 0 ]]; then
        log_warning "以下文件缺失: ${missing_files[*]}"
        log_warning "请手动下载这些文件到 checkpoints 目录"
        log_warning "下载地址: https://huggingface.co/IndexTeam/IndexTTS-1.5"
    else
        log_success "所有必需模型文件都已下载"
    fi
    
    log_success "模型下载完成"
}

# 创建参考语音文件
create_reference_voice() {
    log_info "创建参考语音文件..."
    
    # 创建参考语音文件（如果没有的话）
    if [[ ! -f "reference_voice.wav" ]]; then
        log_info "创建参考语音文件..."
        
        # 使用 say 命令创建语音文件 (macOS)
        if [[ "$OS" == "macos" ]]; then
            say -v "Alex" "This is a reference voice for IndexTTS" -o reference_voice.aiff
            ffmpeg -y -i reference_voice.aiff reference_voice.wav 2>/dev/null || true
            rm -f reference_voice.aiff
            
            if [[ -f "reference_voice.wav" ]]; then
                log_success "参考语音文件创建成功: reference_voice.wav"
            else
                log_warning "参考语音文件创建失败"
            fi
        else
            log_warning "无法自动创建语音文件，请手动提供 reference_voice.wav"
            log_info "您可以使用以下方式创建参考语音文件："
            log_info "1. 使用录音工具录制 3-30 秒的语音"
            log_info "2. 确保格式为 WAV"
            log_info "3. 保存为 reference_voice.wav"
        fi
    else
        log_info "参考语音文件已存在: reference_voice.wav"
    fi
}

# 测试安装
test_installation() {
    log_info "测试 IndexTTS 安装..."
    
    source .venv/bin/activate
    
    # 测试导入
    log_info "测试 IndexTTS 导入..."
    if python -c "import indextts; print('IndexTTS 导入成功')" 2>/dev/null; then
        log_success "IndexTTS 导入测试通过"
    else
        log_error "IndexTTS 导入失败"
        return 1
    fi
    
    # 测试 CLI
    log_info "测试 IndexTTS CLI..."
    if indextts --help > /dev/null 2>&1; then
        log_success "IndexTTS CLI 测试通过"
    else
        log_error "IndexTTS CLI 测试失败"
        return 1
    fi
    
    log_success "安装测试完成"
}

# 创建测试脚本
create_test_script() {
    log_info "创建测试脚本..."
    
    cat > test_indextts.py << 'EOF'
#!/usr/bin/env python3
"""
IndexTTS 测试脚本
"""
import os
import sys
import subprocess

def test_indextts():
    print("开始测试 IndexTTS...")
    
    # 检查模型文件
    required_files = ['checkpoints/config.yaml', 'checkpoints/gpt.pth']
    for file in required_files:
        if not os.path.exists(file):
            print(f"错误: 缺少模型文件 {file}")
            return False
    
    # 检查参考语音文件
    if not os.path.exists('reference_voice.wav'):
        print("错误: 缺少参考语音文件 reference_voice.wav")
        return False
    
    try:
        # 测试合成
        test_text = "你好，这是一个测试。"
        output_file = "test_output.wav"
        
        cmd = [
            'indextts',
            test_text,
            '--voice', 'reference_voice.wav',
            '--output_path', output_file,
            '--model_dir', 'checkpoints',
            '--config', 'checkpoints/config.yaml',
            '--force'
        ]
        
        print(f"执行命令: {' '.join(cmd)}")
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        if result.returncode == 0 and os.path.exists(output_file):
            print(f"测试成功! 输出文件: {output_file}")
            print(f"文件大小: {os.path.getsize(output_file)} bytes")
            return True
        else:
            print(f"测试失败")
            print(f"返回码: {result.returncode}")
            print(f"输出: {result.stdout}")
            print(f"错误: {result.stderr}")
            return False
            
    except Exception as e:
        print(f"测试异常: {e}")
        return False

if __name__ == "__main__":
    success = test_indextts()
    sys.exit(0 if success else 1)
EOF
    
    chmod +x test_indextts.py
    log_success "测试脚本创建完成"
}

# 显示使用说明
show_usage() {
    echo
    echo "=========================================="
    echo "              使用说明"
    echo "=========================================="
    echo
    echo "1. 激活虚拟环境:"
    echo "   source .venv/bin/activate"
    echo
    echo "2. 运行测试:"
    echo "   python test_indextts.py"
    echo
    echo "3. 使用 IndexTTS CLI:"
    echo "   indextts --help"
    echo
    echo "4. 启动 voice-cli 服务器:"
    echo "   voice-cli server run"
    echo
    echo "5. 测试 TTS 接口:"
    echo "   curl -X POST 'http://127.0.0.1:8087/tts/sync' \\"
    echo "     -H 'Content-Type: application/json' \\"
    echo "     -d '{\"text\": \"你好世界\"}' --max-time 60"
    echo
    echo "=========================================="
    echo
}

# 主函数
main() {
    echo "=========================================="
    echo "      IndexTTS 环境安装部署脚本"
    echo "=========================================="
    echo
    
    # 检查是否在正确的目录
    if [[ ! -f "tts_service.py" ]]; then
        log_error "请在包含 tts_service.py 的目录中运行此脚本"
        exit 1
    fi
    
    # 执行安装步骤
    log_info "开始安装 IndexTTS 环境..."
    echo
    
    check_requirements
    install_package_managers
    create_virtualenv
    init_uv_project
    install_pytorch
    install_indextts
    download_models
    create_reference_voice
    test_installation
    create_test_script
    
    echo
    echo "=========================================="
    log_success "IndexTTS 环境安装完成!"
    echo "=========================================="
    echo
    
    show_usage
    
    echo "注意事项:"
    echo "- 确保 checkpoints 目录中有模型文件"
    echo "- 确保 reference_voice.wav 文件存在"
    echo "- 如果遇到问题，请检查日志输出"
    echo "- IndexTTS 处理时间通常为 15-30 秒"
    echo
}

# 运行主函数
main "$@"