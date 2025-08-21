#!/bin/bash

# Document Parser 统一管理脚本
# 用法: ./document-parser-manager.sh {deploy|install|start|stop|restart|status|enable|disable|uninstall|logs|help}

set -e  # 遇到错误立即退出

SERVICE_NAME="document-parser"
SERVICE_FILE="document-parser.service"
# 动态获取当前脚本所在目录作为安装目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${DOCUMENT_PARSER_INSTALL_DIR:-$SCRIPT_DIR}"
BINARY_NAME="document-parser"
CONFIG_FILE="config.yml"

# 颜色输出
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

# 检查是否为 root 用户
check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "此操作需要 root 权限"
        log_info "请使用: sudo $0 $1"
        exit 1
    fi
}

# 检查 systemd 是否可用
check_systemd() {
    if ! command -v systemctl &> /dev/null; then
        log_error "systemctl 命令不可用，请确保系统支持 systemd"
        exit 1
    fi
}

# 检查可执行文件
check_binary() {
    log_info "检查 document-parser 可执行文件..."
    
    # 检查脚本所在目录是否有可执行文件
    if [[ -f "$SCRIPT_DIR/$BINARY_NAME" ]]; then
        log_success "找到可执行文件: $SCRIPT_DIR/$BINARY_NAME"
        # 确保文件有执行权限
        chmod +x "$SCRIPT_DIR/$BINARY_NAME"
        return 0
    fi
    
    log_error "未找到 $BINARY_NAME 可执行文件"
    log_info "请确保:"
    log_info "  1. 将 document-parser 可执行文件放在脚本所在目录: $SCRIPT_DIR"
    log_info "  2. 确保可执行文件有执行权限"
    exit 1
}

# 准备部署文件
prepare_files() {
    log_info "准备部署文件..."
    
    # 检查配置文件（如果存在）
    if [[ -f "$SCRIPT_DIR/$CONFIG_FILE" ]]; then
        log_success "找到配置文件: $SCRIPT_DIR/$CONFIG_FILE"
    else
        log_warning "未找到配置文件 $CONFIG_FILE"
        log_info "请确保将 config.yml 配置文件放在脚本所在目录: $SCRIPT_DIR"
        log_info "或者服务启动时会使用默认配置"
    fi
}

# 动态生成服务文件
generate_service_file() {
    local service_content
    service_content=$(cat <<EOF
[Unit]
Description=Document Parser Service
After=network.target
Wants=network.target

[Service]
Type=simple
User=root
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/$BINARY_NAME
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=document-parser

# 安全配置
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$INSTALL_DIR

# 资源限制
LimitNOFILE=65536
LimitNPROC=4096

[Install]
WantedBy=multi-user.target
EOF
)
    
    echo "$service_content" > "/etc/systemd/system/$SERVICE_FILE"
    log_success "动态生成服务文件，安装目录: $INSTALL_DIR"
}

# 安装服务
install_service() {
    log_info "开始安装 Document Parser 服务..."
    
    # 检查二进制文件是否存在
    if [[ ! -f "$SCRIPT_DIR/$BINARY_NAME" ]]; then
        log_error "找不到 $SCRIPT_DIR/$BINARY_NAME 可执行文件，请先运行 check_binary 或手动复制文件"
        exit 1
    fi
    
    # 创建安装目录（如果不是脚本目录）
    if [[ "$INSTALL_DIR" != "$SCRIPT_DIR" ]]; then
        log_info "创建安装目录: $INSTALL_DIR"
        mkdir -p "$INSTALL_DIR"
        
        # 复制二进制文件
        log_info "复制二进制文件到 $INSTALL_DIR"
        cp "$SCRIPT_DIR/$BINARY_NAME" "$INSTALL_DIR/"
        chmod +x "$INSTALL_DIR/$BINARY_NAME"
        
        # 复制配置文件（如果存在）
        if [[ -f "$SCRIPT_DIR/$CONFIG_FILE" ]]; then
            log_info "复制配置文件到 $INSTALL_DIR"
            cp "$SCRIPT_DIR/$CONFIG_FILE" "$INSTALL_DIR/"
        fi
    else
        log_info "使用脚本目录作为安装目录: $INSTALL_DIR"
        # 确保二进制文件有执行权限
        chmod +x "$INSTALL_DIR/$BINARY_NAME"
        
        if [[ ! -f "$SCRIPT_DIR/$CONFIG_FILE" ]]; then
            log_warning "未找到配置文件 $SCRIPT_DIR/$CONFIG_FILE，请手动创建"
        fi
    fi
    
    # 动态生成并安装 systemd 服务文件
    log_info "生成并安装 systemd 服务文件"
    generate_service_file
    
    # 重新加载 systemd
    log_info "重新加载 systemd 配置"
    systemctl daemon-reload
    
    log_success "Document Parser 服务安装完成"
}

# 启动服务
start_service() {
    log_info "启动 $SERVICE_NAME 服务..."
    if systemctl start "$SERVICE_NAME"; then
        log_success "服务启动成功"
        systemctl status "$SERVICE_NAME" --no-pager -l
    else
        log_error "服务启动失败"
        exit 1
    fi
}

# 停止服务
stop_service() {
    log_info "停止 $SERVICE_NAME 服务..."
    if systemctl stop "$SERVICE_NAME"; then
        log_success "服务停止成功"
    else
        log_error "服务停止失败"
        exit 1
    fi
}

# 重启服务
restart_service() {
    log_info "重启 $SERVICE_NAME 服务..."
    if systemctl restart "$SERVICE_NAME"; then
        log_success "服务重启成功"
        systemctl status "$SERVICE_NAME" --no-pager -l
    else
        log_error "服务重启失败"
        exit 1
    fi
}

# 查看服务状态
status_service() {
    log_info "查看 $SERVICE_NAME 服务状态:"
    systemctl status "$SERVICE_NAME" --no-pager -l
}

# 启用开机自启
enable_service() {
    log_info "启用 $SERVICE_NAME 开机自启..."
    if systemctl enable "$SERVICE_NAME"; then
        log_success "开机自启启用成功"
    else
        log_error "开机自启启用失败"
        exit 1
    fi
}

# 禁用开机自启
disable_service() {
    log_info "禁用 $SERVICE_NAME 开机自启..."
    if systemctl disable "$SERVICE_NAME"; then
        log_success "开机自启禁用成功"
    else
        log_error "开机自启禁用失败"
        exit 1
    fi
}

# 卸载服务
uninstall_service() {
    log_info "开始卸载 $SERVICE_NAME 服务..."
    
    # 停止服务
    if systemctl is-active --quiet "$SERVICE_NAME"; then
        log_info "停止运行中的服务"
        systemctl stop "$SERVICE_NAME"
    fi
    
    # 禁用服务
    if systemctl is-enabled --quiet "$SERVICE_NAME"; then
        log_info "禁用开机自启"
        systemctl disable "$SERVICE_NAME"
    fi
    
    # 删除服务文件
    if [[ -f "/etc/systemd/system/$SERVICE_FILE" ]]; then
        log_info "删除 systemd 服务文件"
        rm -f "/etc/systemd/system/$SERVICE_FILE"
    fi
    
    # 重新加载 systemd
    log_info "重新加载 systemd 配置"
    systemctl daemon-reload
    
    # 删除安装目录（仅当安装目录不是脚本目录时）
    if [[ -d "$INSTALL_DIR" && "$INSTALL_DIR" != "$SCRIPT_DIR" ]]; then
        log_info "删除安装目录: $INSTALL_DIR"
        rm -rf "$INSTALL_DIR"
    elif [[ "$INSTALL_DIR" == "$SCRIPT_DIR" ]]; then
        log_info "安装目录与脚本目录相同，仅删除二进制文件和配置文件"
        if [[ -f "$INSTALL_DIR/$BINARY_NAME" ]]; then
            rm -f "$INSTALL_DIR/$BINARY_NAME"
            log_info "已删除二进制文件: $INSTALL_DIR/$BINARY_NAME"
        fi
        if [[ -f "$INSTALL_DIR/$CONFIG_FILE" ]]; then
            rm -f "$INSTALL_DIR/$CONFIG_FILE"
            log_info "已删除配置文件: $INSTALL_DIR/$CONFIG_FILE"
        fi
    fi
    
    log_success "Document Parser 服务卸载完成"
}

# 查看日志
show_logs() {
    log_info "显示 $SERVICE_NAME 服务日志:"
    journalctl -u "$SERVICE_NAME" -f --no-pager
}

# 后台启动服务（不安装到系统）
start_background() {
    log_info "后台启动 Document Parser 服务..."
    
    # 检查可执行文件
    if [[ ! -f "$SCRIPT_DIR/$BINARY_NAME" ]]; then
        log_error "找不到 $SCRIPT_DIR/$BINARY_NAME 可执行文件"
        log_info "请确保将 document-parser 可执行文件放在脚本所在目录: $SCRIPT_DIR"
        exit 1
    fi
    
    # 这部分逻辑已经在前面的PID文件检查中处理了
     # 如果执行到这里，说明没有运行中的进程
    
    # 切换到脚本目录
    cd "$SCRIPT_DIR"
    
    # 后台启动服务
    log_info "在目录 $SCRIPT_DIR 中启动服务"
    if [[ -f "$SCRIPT_DIR/$CONFIG_FILE" ]]; then
        log_info "使用配置文件: $SCRIPT_DIR/$CONFIG_FILE"
        nohup "./$BINARY_NAME" > document-parser.log 2>&1 &
    else
        log_warning "未找到配置文件，使用默认配置启动"
        nohup "./$BINARY_NAME" > document-parser.log 2>&1 &
    fi
    
    local pid=$!
    echo $pid > "$SCRIPT_DIR/document-parser.pid"
    sleep 2
    
    # 检查进程是否启动成功
    if kill -0 $pid 2>/dev/null; then
        log_success "Document Parser 服务启动成功，PID: $pid"
        log_info "PID 文件: $SCRIPT_DIR/document-parser.pid"
        log_info "日志文件: $SCRIPT_DIR/document-parser.log"
        log_info "查看日志: tail -f $SCRIPT_DIR/document-parser.log"
        log_info "停止服务: $0 stop-background"
    else
        log_error "Document Parser 服务启动失败"
        log_info "查看错误日志: cat $SCRIPT_DIR/document-parser.log"
        rm -f "$SCRIPT_DIR/document-parser.pid"
        exit 1
    fi
}

# 停止后台服务
stop_background() {
    log_info "停止后台 Document Parser 服务..."
    
    local pids=""
    
    # 首先尝试从 PID 文件读取
    if [[ -f "$SCRIPT_DIR/document-parser.pid" ]]; then
        local pid_from_file=$(cat "$SCRIPT_DIR/document-parser.pid" 2>/dev/null || true)
        if [[ -n "$pid_from_file" ]] && kill -0 "$pid_from_file" 2>/dev/null; then
            pids="$pid_from_file"
        fi
    fi
    
    # 如果 PID 文件无效，尝试通过进程名查找
    if [[ -z "$pids" ]]; then
        pids=$(pgrep -x "$BINARY_NAME" 2>/dev/null || true)
    fi
    
    if [[ -z "$pids" ]]; then
        log_warning "未找到运行中的 Document Parser 服务"
        # 清理可能存在的无效 PID 文件
        rm -f "$SCRIPT_DIR/document-parser.pid"
        return 0
    fi
    
    log_info "找到运行中的进程: $pids"
    
    # 优雅停止
    for pid in $pids; do
        log_info "正在停止进程 $pid"
        kill $pid 2>/dev/null || true
    done
    
    # 等待进程停止
    sleep 3
    
    # 检查是否还有进程在运行
    local remaining_pids=""
    for pid in $pids; do
        if kill -0 "$pid" 2>/dev/null; then
            remaining_pids="$remaining_pids $pid"
        fi
    done
    
    if [[ -n "$remaining_pids" ]]; then
        log_warning "进程未能优雅停止，强制终止"
        for pid in $remaining_pids; do
            kill -9 $pid 2>/dev/null || true
        done
        sleep 1
    fi
    
    # 最终检查
    local final_check=$(pgrep -x "$BINARY_NAME" 2>/dev/null || true)
    if [[ -n "$final_check" ]]; then
        log_error "无法停止 Document Parser 服务"
        exit 1
    else
        log_success "Document Parser 服务已停止"
        # 清理 PID 文件
        rm -f "$SCRIPT_DIR/document-parser.pid"
    fi
}

# 查看后台服务状态
status_background() {
    log_info "查看后台 Document Parser 服务状态:"
    
    local pids=""
    local pid_from_file=""
    
    # 首先尝试从 PID 文件读取
    if [[ -f "$SCRIPT_DIR/document-parser.pid" ]]; then
        pid_from_file=$(cat "$SCRIPT_DIR/document-parser.pid" 2>/dev/null || true)
        if [[ -n "$pid_from_file" ]] && kill -0 "$pid_from_file" 2>/dev/null; then
            pids="$pid_from_file"
        else
            # PID 文件存在但进程不存在，清理无效的 PID 文件
            log_warning "发现无效的 PID 文件，正在清理"
            rm -f "$SCRIPT_DIR/document-parser.pid"
        fi
    fi
    
    # 如果 PID 文件无效，尝试通过进程名查找
    if [[ -z "$pids" ]]; then
        pids=$(pgrep -f "\./document-parser$|/document-parser$" 2>/dev/null || true)
        # 如果找到了进程但没有 PID 文件，创建 PID 文件
        if [[ -n "$pids" ]] && [[ ! -f "$SCRIPT_DIR/document-parser.pid" ]]; then
            # 只取第一个PID
            local main_pid=$(echo "$pids" | head -1)
            echo "$main_pid" > "$SCRIPT_DIR/document-parser.pid"
            pids="$main_pid"
            log_info "重新创建 PID 文件: $main_pid"
        fi
    fi
    
    if [[ -z "$pids" ]]; then
        log_warning "Document Parser 服务未运行"
        return 1
    else
        log_success "Document Parser 服务正在运行"
        echo "进程信息:"
        for pid in $pids; do
            # 使用兼容性更好的ps命令格式
            ps -p "$pid" -o pid,ppid,command 2>/dev/null | tail -n +2 2>/dev/null || echo "PID $pid: 进程信息获取失败"
        done
        
        if [[ -n "$pid_from_file" ]]; then
            echo "PID 文件: $SCRIPT_DIR/document-parser.pid (PID: $pid_from_file)"
        fi
        
        if [[ -f "$SCRIPT_DIR/document-parser.log" ]]; then
            echo ""
            log_info "最近的日志 (最后10行):"
            tail -10 "$SCRIPT_DIR/document-parser.log"
        fi
    fi
}

# 一键部署（包含检查、安装、启动、启用自启）
deploy_service() {
    local no_start=false
    local no_enable=false
    
    # 解析部署选项
    while [[ $# -gt 1 ]]; do
        case $2 in
            --no-start)
                no_start=true
                shift
                ;;
            --no-enable)
                no_enable=true
                shift
                ;;
            *)
                shift
                ;;
        esac
    done
    
    log_info "开始 Document Parser 一键部署..."
    echo ""
    
    # 检查系统依赖
    check_systemd
    
    # 检查可执行文件
    check_binary
    
    # 准备文件
    prepare_files
    
    # 安装服务
    install_service
    
    # 启动服务（如果需要）
    if [[ "$no_start" != true ]]; then
        start_service
        echo ""
    fi
    
    # 启用开机自启（如果需要）
    if [[ "$no_enable" != true ]]; then
        enable_service
        echo ""
    fi
    
    # 显示部署结果
    log_success "=== 部署完成 ==="
    echo ""
    log_info "服务状态:"
    status_service
    echo ""
    log_info "常用命令:"
    echo "  查看状态: $0 status"
    echo "  重启服务: sudo $0 restart"
    echo "  查看日志: $0 logs"
    echo "  停止服务: sudo $0 stop"
    echo "  卸载服务: sudo $0 uninstall"
    echo ""
    log_info "更多命令请查看: $0 help"
}

# 显示帮助信息
show_help() {
    echo "Document Parser 统一管理脚本"
    echo ""
    echo "用法: $0 {deploy|install|start|stop|restart|status|enable|disable|uninstall|logs|start-background|stop-background|status-background|help} [选项]"
    echo ""
    echo "系统服务命令:"
    echo "  deploy    - 一键部署（检查文件 + 安装 + 启动 + 开机自启）"
    echo "  install   - 仅安装服务到系统"
    echo "  start     - 启动系统服务"
    echo "  stop      - 停止系统服务"
    echo "  restart   - 重启系统服务"
    echo "  status    - 查看系统服务状态"
    echo "  enable    - 启用开机自启"
    echo "  disable   - 禁用开机自启"
    echo "  uninstall - 卸载系统服务"
    echo "  logs      - 查看系统服务日志"
    echo ""
    echo "后台进程命令（无需安装到系统）:"
    echo "  start-background  - 后台启动服务（不安装到系统）"
    echo "  stop-background   - 停止后台服务"
    echo "  status-background - 查看后台服务状态"
    echo ""
    echo "其他命令:"
    echo "  help      - 显示此帮助信息"
    echo ""
    echo "deploy 命令选项:"
    echo "  --no-start   安装但不启动服务"
    echo "  --no-enable  不启用开机自启"
    echo ""
    echo "安装目录配置:"
    echo "  默认情况下，服务将安装到脚本所在目录: $SCRIPT_DIR"
    echo "  可通过环境变量 DOCUMENT_PARSER_INSTALL_DIR 自定义安装目录"
    echo "  当前安装目录: $INSTALL_DIR"
    echo ""
    echo "权限要求:"
    echo "  - status、logs、help、*-background 命令无需 root 权限"
    echo "  - 系统服务相关命令需要 root 权限"
    echo ""
    echo "使用示例:"
    echo "  # 系统服务方式（需要 root 权限）:"
    echo "  sudo $0 deploy                                    # 一键部署到当前目录"
    echo "  sudo DOCUMENT_PARSER_INSTALL_DIR=/opt/app $0 deploy  # 部署到指定目录"
    echo "  sudo $0 deploy --no-start                         # 安装但不启动"
    echo "  sudo $0 install                                   # 仅安装"
    echo "  sudo $0 start                                     # 启动系统服务"
    echo "  $0 status                                         # 查看系统服务状态"
    echo "  $0 logs                                           # 查看系统服务日志"
    echo ""
    echo "  # 后台进程方式（无需 root 权限）:"
    echo "  $0 start-background                               # 后台启动服务"
    echo "  $0 status-background                              # 查看后台服务状态"
    echo "  $0 stop-background                                # 停止后台服务"
    echo "  tail -f $SCRIPT_DIR/document-parser.log           # 查看后台服务日志"
}

# 主函数
main() {
    if [[ $# -eq 0 ]]; then
        log_error "缺少命令参数"
        show_help
        exit 1
    fi
    
    case "$1" in
        deploy)
            check_root "$1"
            deploy_service "$@"
            ;;
        install)
            check_root "$1"
            check_systemd
            install_service
            log_info "使用以下命令管理服务:"
            log_info "  启动服务: sudo $0 start"
            log_info "  查看状态: $0 status"
            log_info "  开机自启: sudo $0 enable"
            ;;
        start)
            check_root "$1"
            check_systemd
            start_service
            ;;
        stop)
            check_root "$1"
            check_systemd
            stop_service
            ;;
        restart)
            check_root "$1"
            check_systemd
            restart_service
            ;;
        status)
            check_systemd
            status_service
            ;;
        enable)
            check_root "$1"
            check_systemd
            enable_service
            ;;
        disable)
            check_root "$1"
            check_systemd
            disable_service
            ;;
        uninstall)
            check_root "$1"
            check_systemd
            uninstall_service
            ;;
        logs)
            check_systemd
            show_logs
            ;;
        start-background)
            start_background
            ;;
        stop-background)
            stop_background
            ;;
        status-background)
            status_background
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_error "无效的命令: $1"
            show_help
            exit 1
            ;;
    esac
}

# 执行主函数
main "$@"