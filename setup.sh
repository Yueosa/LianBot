#!/usr/bin/env bash
# setup.sh — LianBot 管理入口
#
# 在项目根目录运行：bash setup.sh
#
# 三层配置架构（v0.2.0+）：
#   config.toml   — kernel 层，host / port
#   runtime.toml  — 运行时，napcat / bot / pool / log / parser / time
#   logic.toml    — 业务逻辑，smy / github / alive / acg / world

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/scripts"
source "$SCRIPT_DIR/lib.sh"

[[ -f "Cargo.toml" ]] || { echo "请在 LianBot 项目根目录运行此脚本"; exit 1; }

main_menu() {
    while true; do
        clear
        title "  LianBot v${BOT_VERSION}  —  管理面板"
        sep
        echo ""

        # 配置状态
        local cfg_s="❌" rt_s="❌" lg_s="❌"
        [[ -f "config.toml" ]]  && cfg_s="✅"
        [[ -f "runtime.toml" ]] && rt_s="✅"
        [[ -f "logic.toml" ]]   && lg_s="✅"

        echo "  config.toml   $cfg_s  (kernel: host/port)"
        echo "  runtime.toml  $rt_s  (runtime: napcat/bot/pool/log/parser/time)"
        echo "  logic.toml    $lg_s  (logic: smy/github/alive/acg/world)"
        echo ""

        # 服务状态
        if systemctl is-active lianbot &>/dev/null 2>&1; then
            echo -e "  lianbot.service  ${C_GREEN}● 运行中${C_NC}"
        else
            echo -e "  lianbot.service  ${C_RED}● 已停止${C_NC}"
        fi
        echo ""
        sep
        echo ""
        echo "  ${C_BOLD}配置生成${C_NC}"
        echo "  1  生成 config.toml（kernel 层）"
        echo "  2  生成 runtime.toml（运行时层）"
        echo "  3  生成 logic.toml（业务逻辑层）"
        echo ""
        echo "  ${C_BOLD}运维操作${C_NC}"
        echo "  4  查看部署状态"
        echo "  5  编译项目"
        echo "  6  部署到服务器"
        echo "  7  服务管理（restart/stop/start）"
        echo "  8  实时查看日志"
        echo ""
        echo "  0  退出"
        echo ""
        read -rp "  > " choice

        case "$choice" in
            1) bash "$SCRIPT_DIR/gen_config.sh"  ; read -rp "  按 Enter 返回主菜单..." _ ;;
            2) bash "$SCRIPT_DIR/gen_runtime.sh" ; read -rp "  按 Enter 返回主菜单..." _ ;;
            3) bash "$SCRIPT_DIR/gen_logic.sh"   ; read -rp "  按 Enter 返回主菜单..." _ ;;
            4) bash "$SCRIPT_DIR/status.sh"      ; read -rp "  按 Enter 返回主菜单..." _ ;;
            5) bash "$SCRIPT_DIR/build.sh"       ; read -rp "  按 Enter 返回主菜单..." _ ;;
            6)
                echo ""
                echo "  将运行：sudo env PATH=\"$PATH\" bash scripts/deploy.sh"
                read -rp "  确认继续？(y/N): " confirm
                if [[ "${confirm,,}" == "y" ]]; then
                    sudo env PATH="$PATH" bash "$SCRIPT_DIR/deploy.sh"
                fi
                read -rp "  按 Enter 返回主菜单..." _
                ;;
            7) bash "$SCRIPT_DIR/service.sh"     ; read -rp "  按 Enter 返回主菜单..." _ ;;
            8) bash "$SCRIPT_DIR/logs.sh"        ;;
            0) echo ""; info "再见！"; echo ""; exit 0 ;;
            *) warn "请输入 0-8"; sleep 0.5 ;;
        esac
    done
}

main_menu
