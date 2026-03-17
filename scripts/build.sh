#!/usr/bin/env bash
# build.sh — 编译 LianBot（支持交互式 feature 选择）

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
cd "$PROJECT_ROOT"

# ── Feature 定义 ──────────────────────────────────────────────────────────────
# 格式: "feature名:显示名称:默认状态:依赖列表"

declare -A FEATURES
declare -A FEATURE_DESC
declare -A FEATURE_DEFAULT
declare -A FEATURE_DEPS

# Runtime 基础模块
FEATURES[runtime-api]="api - HTTP 客户端"
FEATURE_DEFAULT[runtime-api]=1
FEATURE_DEPS[runtime-api]="runtime-config runtime-http"

FEATURES[runtime-typ]="typ - OneBot 协议类型"
FEATURE_DEFAULT[runtime-typ]=1

FEATURES[runtime-dispatcher]="dispatcher - 命令分发器"
FEATURE_DEFAULT[runtime-dispatcher]=1
FEATURE_DEPS[runtime-dispatcher]="runtime-parser runtime-registry runtime-permission runtime-api runtime-typ"

FEATURES[runtime-permission]="permission - 权限控制"
FEATURE_DEFAULT[runtime-permission]=1

FEATURES[runtime-pool]="pool - 消息池"
FEATURE_DEFAULT[runtime-pool]=1
FEATURE_DEPS[runtime-pool]="runtime-typ runtime-time runtime-config"

FEATURES[runtime-llm]="llm - LLM 客户端（AI 功能）"
FEATURE_DEFAULT[runtime-llm]=0
FEATURE_DEPS[runtime-llm]="runtime-config"

FEATURES[runtime-ws]="ws - WebSocket 管理"
FEATURE_DEFAULT[runtime-ws]=0

FEATURES[runtime-logger]="logger - 日志系统"
FEATURE_DEFAULT[runtime-logger]=1
FEATURE_DEPS[runtime-logger]="runtime-time"

FEATURES[runtime-time]="time - 时间工具"
FEATURE_DEFAULT[runtime-time]=1
FEATURE_DEPS[runtime-time]="runtime-config"

# 命令模块
FEATURES[cmd-ping]="ping - 基础 ping 命令"
FEATURE_DEFAULT[cmd-ping]=1
FEATURE_DEPS[cmd-ping]="runtime-dispatcher"

FEATURES[cmd-help]="help - 帮助命令"
FEATURE_DEFAULT[cmd-help]=1
FEATURE_DEPS[cmd-help]="runtime-dispatcher"

FEATURES[cmd-alive]="alive - 设备状态查询"
FEATURE_DEFAULT[cmd-alive]=1
FEATURE_DEPS[cmd-alive]="runtime-dispatcher"

FEATURES[cmd-smy]="smy - 群聊日报"
FEATURE_DEFAULT[cmd-smy]=0
FEATURE_DEPS[cmd-smy]="runtime-dispatcher logic-smy"

FEATURES[cmd-stalk]="stalk - 截图监控"
FEATURE_DEFAULT[cmd-stalk]=0
FEATURE_DEPS[cmd-stalk]="runtime-dispatcher runtime-ws core-ws"

FEATURES[cmd-acg]="acg - 随机二次元图片"
FEATURE_DEFAULT[cmd-acg]=0
FEATURE_DEPS[cmd-acg]="runtime-dispatcher"

FEATURES[cmd-world]="world - 60秒看世界"
FEATURE_DEFAULT[cmd-world]=0
FEATURE_DEPS[cmd-world]="runtime-dispatcher"

FEATURES[cmd-dress]="dress - 随机女装图片"
FEATURE_DEFAULT[cmd-dress]=0
FEATURE_DEPS[cmd-dress]="runtime-dispatcher"

FEATURES[cmd-sign]="sign - 易班签到"
FEATURE_DEFAULT[cmd-sign]=0
FEATURE_DEPS[cmd-sign]="runtime-dispatcher logic-yiban"

FEATURES[cmd-send]="send - 图文混合消息（LLM 专用）"
FEATURE_DEFAULT[cmd-send]=0
FEATURE_DEPS[cmd-send]="runtime-dispatcher"

# 服务模块
FEATURES[svc-github]="github - GitHub Webhook"
FEATURE_DEFAULT[svc-github]=0
FEATURE_DEPS[svc-github]="logic-github"

FEATURES[svc-yiban]="yiban - 易班 Webhook"
FEATURE_DEFAULT[svc-yiban]=0
FEATURE_DEPS[svc-yiban]="logic-yiban"

# 基础设施
FEATURES[core-db]="db - SQLite 数据库"
FEATURE_DEFAULT[core-db]=0

FEATURES[core-webhook]="webhook - HMAC-SHA256 验签"
FEATURE_DEFAULT[core-webhook]=0

FEATURES[core-log-file]="log-file - 滚动日志文件"
FEATURE_DEFAULT[core-log-file]=0

FEATURES[core-ws]="ws-route - WebSocket 路由"
FEATURE_DEFAULT[core-ws]=0
FEATURE_DEPS[core-ws]="runtime-ws"

# Logic 层（自动依赖，不显示在菜单）
FEATURE_DEPS[logic-smy]="runtime-api runtime-typ"
FEATURE_DEPS[logic-github]="runtime-api runtime-typ core-webhook"
FEATURE_DEPS[logic-yiban]="runtime-api runtime-typ core-webhook"

# ── 主菜单 ────────────────────────────────────────────────────────────────────

show_main_menu() {
    clear
    title "  LianBot v${BOT_VERSION}  —  编译选项"
    sep
    echo ""
    echo "  ${C_BOLD}[1]${C_NC}  全量编译 (--all-features)"
    echo "       编译所有功能，适合生产环境"
    echo ""
    echo "  ${C_BOLD}[2]${C_NC}  自定义编译"
    echo "       选择需要的模块，自动处理依赖关系"
    echo ""
    echo "  ${C_BOLD}[0]${C_NC}  返回"
    echo ""
    read -rp "  > " choice

    case "$choice" in
        1) build_all_features ;;
        2) build_custom ;;
        0) exit 0 ;;
        *) warn "无效选项" ; sleep 1 ; show_main_menu ;;
    esac
}

# ── 全量编译 ──────────────────────────────────────────────────────────────────

build_all_features() {
    clear
    title "  全量编译"
    sep
    echo ""
    info "编译模式: --release --all-features"
    echo ""

    cargo build --release --all-features

    if [[ $? -eq 0 ]]; then
        echo ""
        info "✅ 编译完成: target/release/LianBot"
        ls -lh "$PROJECT_ROOT/target/release/LianBot" 2>/dev/null || true
        echo ""
        read -rp "按回车返回..."
    else
        echo ""
        warn "❌ 编译失败"
        read -rp "按回车返回..."
    fi

    show_main_menu
}

# ── 自定义编译 ────────────────────────────────────────────────────────────────

build_custom() {
    # 初始化选中状态（从默认值）
    declare -A SELECTED
    for feat in "${!FEATURES[@]}"; do
        SELECTED[$feat]=${FEATURE_DEFAULT[$feat]}
    done

    # 加载上次保存的选择（如果存在）
    if [[ -f ".build_features" ]]; then
        while IFS= read -r feat; do
            [[ -n "$feat" ]] && SELECTED[$feat]=1
        done < .build_features
    fi

    while true; do
        show_feature_menu
        read -rp "  > " input

        case "$input" in
            c|C)
                # 确认编译
                do_build
                break
                ;;
            a|A)
                # 全选
                for feat in "${!FEATURES[@]}"; do
                    SELECTED[$feat]=1
                done
                ;;
            n|N)
                # 全不选
                for feat in "${!FEATURES[@]}"; do
                    SELECTED[$feat]=0
                done
                ;;
            d|D)
                # 恢复默认
                for feat in "${!FEATURES[@]}"; do
                    SELECTED[$feat]=${FEATURE_DEFAULT[$feat]}
                done
                ;;
            q|Q|0)
                show_main_menu
                return
                ;;
            *)
                # 切换指定模块
                toggle_features "$input"
                ;;
        esac
    done
}

show_feature_menu() {
    clear
    title "  自定义编译 — 选择模块"
    sep
    echo ""

    local idx=1

    echo "  ${C_BOLD}Runtime 基础模块${C_NC}"
    for feat in runtime-api runtime-typ runtime-dispatcher runtime-permission runtime-pool runtime-llm runtime-ws runtime-logger runtime-time; do
        [[ -z "${FEATURES[$feat]}" ]] && continue
        local mark=$([[ ${SELECTED[$feat]} -eq 1 ]] && echo "√" || echo " ")
        printf "    [%2d] [%s] %s\n" $idx "$mark" "${FEATURES[$feat]}"
        FEATURE_IDX[$idx]=$feat
        ((idx++))
    done
    echo ""

    echo "  ${C_BOLD}命令模块${C_NC}"
    for feat in cmd-ping cmd-help cmd-alive cmd-smy cmd-stalk cmd-acg cmd-world cmd-dress cmd-sign cmd-send; do
        [[ -z "${FEATURES[$feat]}" ]] && continue
        local mark=$([[ ${SELECTED[$feat]} -eq 1 ]] && echo "√" || echo " ")
        printf "    [%2d] [%s] %s\n" $idx "$mark" "${FEATURES[$feat]}"
        FEATURE_IDX[$idx]=$feat
        ((idx++))
    done
    echo ""

    echo "  ${C_BOLD}服务模块${C_NC}"
    for feat in svc-github svc-yiban; do
        [[ -z "${FEATURES[$feat]}" ]] && continue
        local mark=$([[ ${SELECTED[$feat]} -eq 1 ]] && echo "√" || echo " ")
        printf "    [%2d] [%s] %s\n" $idx "$mark" "${FEATURES[$feat]}"
        FEATURE_IDX[$idx]=$feat
        ((idx++))
    done
    echo ""

    echo "  ${C_BOLD}基础设施${C_NC}"
    for feat in core-db core-webhook core-log-file core-ws; do
        [[ -z "${FEATURES[$feat]}" ]] && continue
        local mark=$([[ ${SELECTED[$feat]} -eq 1 ]] && echo "√" || echo " ")
        printf "    [%2d] [%s] %s\n" $idx "$mark" "${FEATURES[$feat]}"
        FEATURE_IDX[$idx]=$feat
        ((idx++))
    done
    echo ""

    sep
    echo "  输入编号切换模块（如: 1 2 3），输入操作："
    echo "    ${C_BOLD}a${C_NC} 全选  ${C_BOLD}n${C_NC} 全不选  ${C_BOLD}d${C_NC} 恢复默认  ${C_BOLD}c${C_NC} 确认编译  ${C_BOLD}q${C_NC} 返回"
    echo ""
}

toggle_features() {
    local input="$1"
    for num in $input; do
        if [[ "$num" =~ ^[0-9]+$ ]]; then
            local feat="${FEATURE_IDX[$num]}"
            if [[ -n "$feat" ]]; then
                if [[ ${SELECTED[$feat]} -eq 1 ]]; then
                    SELECTED[$feat]=0
                else
                    SELECTED[$feat]=1
                    # 自动勾选依赖
                    auto_select_deps "$feat"
                fi
            fi
        fi
    done
}

auto_select_deps() {
    local feat="$1"
    local deps="${FEATURE_DEPS[$feat]:-}"
    [[ -z "$deps" ]] && return

    for dep in $deps; do
        # 只处理在 FEATURES 中定义的依赖（跳过 logic 层等隐藏依赖）
        if [[ -n "${FEATURES[$dep]:-}" ]]; then
            SELECTED[$dep]=1
            # 递归处理依赖的依赖
            auto_select_deps "$dep"
        fi
    done
}

do_build() {
    # 收集选中的 features
    local features=()
    for feat in "${!SELECTED[@]}"; do
        if [[ ${SELECTED[$feat]} -eq 1 ]]; then
            features+=("$feat")
        fi
    done

    # 添加隐藏的 logic 层依赖
    local all_features=("${features[@]}")
    for feat in "${features[@]}"; do
        local deps="${FEATURE_DEPS[$feat]}"
        [[ -z "$deps" ]] && continue
        for dep in $deps; do
            # 如果依赖不在 FEATURES 中（如 logic-smy），直接添加到编译列表
            if [[ -z "${FEATURES[$dep]:-}" ]]; then
                # 检查是否已存在
                local exists=0
                for f in "${all_features[@]}"; do
                    [[ "$f" == "$dep" ]] && exists=1 && break
                done
                [[ $exists -eq 0 ]] && all_features+=("$dep")
            fi
        done
    done

    # 保存选择（只保存用户可见的 features）
    printf "%s\n" "${features[@]}" > .build_features

    clear
    title "  开始编译"
    sep
    echo ""

    if [[ ${#all_features[@]} -eq 0 ]]; then
        info "编译模式: --release --no-default-features"
        echo ""
        cargo build --release --no-default-features
    else
        local feat_str=$(IFS=,; echo "${all_features[*]}")
        info "编译模式: --release --no-default-features --features \"$feat_str\""
        echo ""
        cargo build --release --no-default-features --features "$feat_str"
    fi

    if [[ $? -eq 0 ]]; then
        echo ""
        info "✅ 编译完成: target/release/LianBot"
        ls -lh "$PROJECT_ROOT/target/release/LianBot" 2>/dev/null || true
        echo ""
        read -rp "按回车返回..."
    else
        echo ""
        warn "❌ 编译失败"
        read -rp "按回车返回..."
    fi

    show_main_menu
}

# ── 入口 ──────────────────────────────────────────────────────────────────────

command -v cargo &>/dev/null || error "未找到 cargo，请先安装 Rust: https://rustup.rs"

declare -A FEATURE_IDX

show_main_menu
