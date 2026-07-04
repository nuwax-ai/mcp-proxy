#!/usr/bin/env bash
# 初始化 document-parser 的 Python venv（mineru + markitdown）
# 关键: mineru 锁 3.4.2（3.4.0 有 PageChars bug），huggingface-hub<1.0（避 mineru/vllm 冲突）
set -euo pipefail

# 切到部署根目录（document-parser/，即本脚本的上级目录的上级）
cd "$(dirname "$0")/../.."
INSTALL_DIR="$(pwd)"
VENV="$INSTALL_DIR/venv"

echo "INSTALL_DIR=$INSTALL_DIR"
echo "VENV=$VENV"

echo
echo "=== 1) 检查 uv ==="
if ! command -v uv >/dev/null 2>&1; then
  echo "❌ 未安装 uv，请先装:"
  echo "   curl -LsSf https://astral.sh/uv/install.sh | sh"
  exit 1
fi

echo
echo "=== 2) 创建 venv (Python 3.12) ==="
# 用绝对路径 /usr/bin/python3 避免 anaconda 的旧版本污染（ mineru 要求 >=3.10）
if [ -x /usr/bin/python3.12 ]; then
  uv venv --python /usr/bin/python3.12 "$VENV"
elif [ -x /usr/bin/python3 ]; then
  uv venv --python /usr/bin/python3 "$VENV"
else
  uv venv --python 3.12 "$VENV"
fi

PY="$VENV/bin/python"
echo "Python: $($PY --version)"

echo
echo "=== 3) 装 mineru[core]==3.4.2（锁版本避 PageChars bug）==="
uv pip install "mineru[core]==3.4.2" --python "$PY"

echo
echo "=== 4) 装 markitdown ==="
uv pip install markitdown --python "$PY"

echo
echo "=== 5) 修 huggingface-hub<1.0（避 mineru 3.4.2 依赖冲突）==="
uv pip install "huggingface-hub>=0.34,<1.0" --python "$PY"

echo
echo "=== 6) 验证 ==="
"$VENV/bin/mineru" --version
"$PY" -c "import torch; print('torch', torch.__version__, 'cuda', torch.cuda.is_available())"

echo
echo "✅ venv 初始化完成: $VENV"
echo "下一步: bash deploy/scripts/install.sh"
