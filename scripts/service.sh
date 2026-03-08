#!/usr/bin/env bash
# service.sh — LianBot 服务管理（restart / stop / start / status）

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SERVICE="lianbot"

do_cmd() {
    local action="$1"
    info "${action} ${SERVICE}..."
    sudo systemctl "$action" "$SERVICE"
    echo ""
    systemctl status "$SERVICE" --no-pager || true
}

if [[ $# -ge 1 ]]; then
    case "$1" in
        restart|stop|start|status) do_cmd "$1" ; exit 0 ;;
        *) ;;
    esac
fi

# 交互式菜单
clear
title "  LianBot v${BOT_VERSION}  —  服务管理"
sep
echo ""

# 当前状态
if systemctl is-active "$SERVICE" &>/dev/null; then
    echo -e "  当前状态: ${C_GREEN}● 运行中${C_NC}"
else
    echo -e "  当前状态: ${C_RED}● 已停止${C_NC}"
fi
echo ""

echo "  1  restart  重启服务"
echo "  2  stop     停止服务"
echo "  3  start    启动服务"
echo "  4  status   查看状态"
echo ""
echo "  0  返回"
echo ""
read -rp "  > " choice

case "$choice" in
    1) do_cmd restart ;;
    2) do_cmd stop ;;
    3) do_cmd start ;;
    4) do_cmd status ;;
    0) ;;
    *) warn "无效选项" ;;
esac
