#!/usr/bin/env bash
# document-parser 一键部署: 建 .env 模板 + 装 systemd unit + enable
# 前置: 二进制(document-parser) + venv(./venv) 已就绪（先跑 setup-venv.sh + 放二进制）
set -euo pipefail

cd "$(dirname "$0")/../.."
INSTALL_DIR="$(pwd)"
BIN="$INSTALL_DIR/document-parser"
SERVICE_NAME="document-parser"

USER="${DOC_PARSER_USER:-$(whoami)}"
GROUP="$(id -gn "$USER")"

echo "INSTALL_DIR=$INSTALL_DIR"
echo "USER=$USER  GROUP=$GROUP"

echo
echo "=== 1) 检查二进制 ==="
[ -x "$BIN" ] || { echo "❌ 二进制不存在: $BIN（请先把编译产物放过来）"; exit 1; }

echo
echo "=== 2) 建 .document-parser.env（如不存在）==="
if [ ! -f "$INSTALL_DIR/.document-parser.env" ]; then
  cp "$INSTALL_DIR/deploy/systemd/.document-parser.env.example" "$INSTALL_DIR/.document-parser.env"
  chmod 600 "$INSTALL_DIR/.document-parser.env"
  chown "$USER:$GROUP" "$INSTALL_DIR/.document-parser.env"
  echo "  ✅ 已创建（占位），请编辑填 OSS 密钥: vim $INSTALL_DIR/.document-parser.env"
else
  echo "  已存在，跳过"
fi

echo
echo "=== 3) 装 systemd unit（用 install 装到 /etc，密码走 stdin 不冲突）==="
TMP=$(mktemp)
sed -e "s|__USER__|$USER|g" \
    -e "s|__GROUP__|$GROUP|g" \
    -e "s|__INSTALL_DIR__|$INSTALL_DIR|g" \
    "$INSTALL_DIR/deploy/systemd/document-parser.service.example" > "$TMP"
sudo install -m 644 -o root -g root "$TMP" "/etc/systemd/system/$SERVICE_NAME.service"
rm -f "$TMP"
echo "  ✅ 已安装到 /etc/systemd/system/$SERVICE_NAME.service"

echo
echo "=== 4) daemon-reload + enable ==="
sudo systemctl daemon-reload
sudo systemctl enable "$SERVICE_NAME"

echo
echo "✅ 部署完成。下一步:"
echo "  1. 填 OSS 密钥:  vim $INSTALL_DIR/.document-parser.env"
echo "  2. (可选)改配置: vim $INSTALL_DIR/config.yml"
echo "  3. 启动:        sudo systemctl start $SERVICE_NAME"
echo "  4. 日志:        sudo journalctl -u $SERVICE_NAME -f"
