#!/usr/bin/env bash
# setup.sh — LianBot 管理入口
#
# 在项目根目录运行：bash setup.sh
#
# 三层配置架构（v0.2.0+）：
#   config.toml   — kernel 层，host / port
#   runtime.toml  — 运行时，napcat / bot / pool / log / parser / time
#   logic.toml    — 业务逻辑，smy / github / yiban / alive / acg / world / chat / dress

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/scripts"
source "$SCRIPT_DIR/lib.sh"

[[ -f "Cargo.toml" ]] || { echo "请在 LianBot 项目根目录运行此脚本"; exit 1; }

# ── 检测可用编辑器 ────────────────────────────────────────────────────────────

detect_editor() {
    if [[ -n "${EDITOR:-}" ]] && command -v "$EDITOR" &>/dev/null; then
        echo "$EDITOR"
    elif command -v nano &>/dev/null; then
        echo "nano"
    elif command -v vim &>/dev/null; then
        echo "vim"
    elif command -v vi &>/dev/null; then
        echo "vi"
    else
        echo ""
    fi
}

# ── 配置文件子菜单 ────────────────────────────────────────────────────────────
# config_menu <name> <target> <example> <gen_script>

config_menu() {
    local name="$1" target="$2" example="$3" gen_script="$4"
    local editor
    editor=$(detect_editor)

    clear
    title "  LianBot v${BOT_VERSION}  —  ${target} 管理"
    sep
    echo ""

    if [[ -f "$target" ]]; then
        echo -e "  ${target}  ${C_GREEN}✅ 已存在${C_NC}"
    else
        echo -e "  ${target}  ${C_RED}❌ 不存在${C_NC}"
    fi
    if [[ -f "$example" ]]; then
        echo -e "  ${example}  ${C_DIM}（模板）${C_NC}"
    fi
    echo ""
    sep
    echo ""

    echo "  1  交互式生成（向导模式）"
    if [[ -f "$example" ]]; then
        if [[ -f "$target" ]]; then
            echo "  2  从 ${example} 重新复制${C_DIM}（覆盖现有）${C_NC}"
        else
            echo "  2  从 ${example} 复制"
        fi
    else
        echo -e "  2  ${C_DIM}从模板复制（模板不存在）${C_NC}"
    fi
    if [[ -n "$editor" ]]; then
        if [[ -f "$target" ]]; then
            echo "  3  用 ${editor} 编辑 ${target}"
        else
            echo -e "  3  ${C_DIM}用编辑器打开（文件不存在，请先生成或复制）${C_NC}"
        fi
    else
        echo -e "  3  ${C_DIM}用编辑器打开（未检测到可用编辑器）${C_NC}"
    fi
    echo ""
    echo "  0  返回主菜单"
    echo ""
    read -rp "  > " choice

    case "$choice" in
        1)
            bash "$SCRIPT_DIR/$gen_script"
            read -rp "  按 Enter 返回主菜单..." _
            ;;
        2)
            if [[ ! -f "$example" ]]; then
                warn "${example} 不存在，无法复制"
                read -rp "  按 Enter 返回..." _
                return
            fi
            if [[ -f "$target" ]]; then
                echo ""
                warn "${target} 已存在，将被覆盖。"
                read -rp "  确认覆盖？(y/N): " confirm
                if [[ "${confirm,,}" != "y" ]]; then
                    info "已取消"
                    read -rp "  按 Enter 返回..." _
                    return
                fi
            fi
            cp "$example" "$target"
            info "已复制 ${example} → ${target}"
            echo ""
            if [[ -n "$editor" ]]; then
                read -rp "  是否立即编辑？(y/N): " edit_now
                if [[ "${edit_now,,}" == "y" ]]; then
                    "$editor" "$target"
                fi
            fi
            read -rp "  按 Enter 返回主菜单..." _
            ;;
        3)
            if [[ -z "$editor" ]]; then
                warn "未检测到可用编辑器，请设置 EDITOR 环境变量"
            elif [[ ! -f "$target" ]]; then
                warn "${target} 不存在，请先通过选项 1 或 2 创建"
            else
                "$editor" "$target"
            fi
            read -rp "  按 Enter 返回主菜单..." _
            ;;
        0) ;;
        *) warn "无效选项"; sleep 0.5 ;;
    esac
}

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
        echo "  logic.toml    $lg_s  (logic: smy/github/yiban/alive/acg/world/chat/dress)"
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
        echo "  ${C_BOLD}配置管理${C_NC}"
        echo "  1  config.toml（kernel 层）"
        echo "  2  runtime.toml（运行时层）"
        echo "  3  logic.toml（业务逻辑层）"
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
            1) config_menu "config"  "config.toml"  "config.example.toml"  "gen_config.sh"  ;;
            2) config_menu "runtime" "runtime.toml" "runtime.example.toml" "gen_runtime.sh" ;;
            3) config_menu "logic"   "logic.toml"   "logic.example.toml"   "gen_logic.sh"   ;;
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
            8) bash "$SCRIPT_DIR/logs.sh" || true ;;
            0) echo ""; info "再见！"; echo ""; exit 0 ;;
            *) warn "请输入 0-8"; sleep 0.5 ;;
        esac
    done
}

main_menu
