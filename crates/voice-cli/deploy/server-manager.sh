#!/bin/bash
# voice-cli 进程管理（改良版）
# 相比 scripts/server-manager.sh 的改进:
#   1. 启动前 cd 到 PROJECT_ROOT，让 ./models ./logs ./data/tasks.db 相对路径落对
#   2. 显式传 --config（注意: --config 必须在 server run 后，全局 -c 在 server run 时被代码忽略）
#
# 用法: ./server-manager.sh {start|stop|restart|status}
# 可用环境变量覆盖:
#   VOICE_CLI_HOME    安装根目录（默认: 本脚本上级目录）
#   VOICE_CLI_BIN     二进制路径（默认: $VOICE_CLI_HOME/voice-cli）
#   VOICE_CLI_CONFIG  配置文件（默认: $VOICE_CLI_HOME/config.yml）

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${VOICE_CLI_HOME:-$(dirname "$SCRIPT_DIR")}"
PID_FILE="${PROJECT_ROOT}/voice-cli.pid"
LOG_FILE="${PROJECT_ROOT}/logs/server.log"
CONFIG_FILE="${VOICE_CLI_CONFIG:-${PROJECT_ROOT}/config.yml}"
VOICE_CLI_BIN="${VOICE_CLI_BIN:-${PROJECT_ROOT}/voice-cli}"

mkdir -p "${PROJECT_ROOT}/logs"

start_server() {
    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "Server already running (PID: $(cat "$PID_FILE"))"
        return 1
    fi
    [ -x "$VOICE_CLI_BIN" ] || { echo "❌ binary not found: $VOICE_CLI_BIN"; return 1; }
    [ -f "$CONFIG_FILE" ] || { echo "❌ config not found: $CONFIG_FILE"; return 1; }

    # 关键 1: cd 到 PROJECT_ROOT，让相对路径（./models ./logs ./data）落对
    cd "$PROJECT_ROOT" || return 1
    # 关键 2: --config 必须在 server run 后面
    nohup "$VOICE_CLI_BIN" server run --config "$CONFIG_FILE" >> "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"
    echo "Server started (PID: $(cat "$PID_FILE"))"
    echo "  binary: $VOICE_CLI_BIN"
    echo "  config: $CONFIG_FILE"
    echo "  cwd:    $PROJECT_ROOT"
    echo "  logs:   $LOG_FILE"
}

stop_server() {
    [ -f "$PID_FILE" ] || { echo "PID file not found, not running?"; return 1; }
    local pid; pid=$(cat "$PID_FILE")
    if kill -0 "$pid" 2>/dev/null; then
        echo "Stopping (PID: $pid)..."
        kill "$pid"
        for i in {1..10}; do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
        kill -0 "$pid" 2>/dev/null && { echo "force kill"; kill -9 "$pid"; }
        rm -f "$PID_FILE"
        echo "Stopped"
    else
        echo "Not running (stale PID: $pid)"; rm -f "$PID_FILE"
    fi
}

status_server() {
    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
        echo "Running (PID: $(cat "$PID_FILE"))  binary=$VOICE_CLI_BIN  config=$CONFIG_FILE"
    else
        echo "Not running"
        return 1
    fi
}

case "${1:-}" in
    start)   start_server ;;
    stop)    stop_server ;;
    restart) stop_server; sleep 2; start_server ;;
    status)  status_server ;;
    *) echo "Usage: $0 {start|stop|restart|status}"; exit 1 ;;
esac
