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
# Linux 上优先用绝对路径 /usr/bin/python3.12（避开 anaconda 旧版本污染）；
# 其它平台（macOS 等）交给 uv 自解析/下载 3.12（uv 会找到系统的 3.12 或自动装一个）。
# 注意：不能用裸 /usr/bin/python3 —— macOS 上那是 Xcode 的 3.9.x，不满足 mineru >=3.10。
if [ -x /usr/bin/python3.12 ]; then
  uv venv --python /usr/bin/python3.12 "$VENV"
else
  uv venv --python 3.12 "$VENV"
fi

PY="$VENV/bin/python"
PY_VERSION=$("$PY" --version | awk '{print $2}')
echo "Python: $PY_VERSION"
# mineru 要求 >=3.10,<3.14；不满足直接退出（Fail Fast，避免后面装包才报错）
# 注意：PY_VERSION 是 X.Y.Z 全版本号（如 3.12.3），case 用 3.1[0-3]* 通配匹配 3.10–3.13.* 全部补丁号
case "$PY_VERSION" in
  3.1[0-3]*) ;;
  *) echo "❌ Python $PY_VERSION 不满足 mineru 要求（需 3.10–3.13）。请先装 3.12：uv python install 3.12" ; exit 1 ;;
esac

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
# 同时探测 CUDA(Linux+NVIDIA) 与 MPS(macOS Metal)；Mac 上 cuda=False、mps=True 为正常
"$PY" -c "import torch; print('torch', torch.__version__, '| cuda', torch.cuda.is_available(), '| mps', torch.backends.mps.is_available())"

echo
echo "✅ venv 初始化完成: $VENV"
echo "下一步: bash deploy/scripts/install.sh"
