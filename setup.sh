#!/usr/bin/env bash
# setup.sh — LianBot 本地配置向导
#
# 不需要 root，在项目根目录运行：
#   bash setup.sh
#
# 功能：
#   1. 选择编译模块（生成 .build_features）
#   2. 生成 config.toml（基础设施配置）
#   3. 生成 plugins.toml（插件私有配置）
#   4. 编译验证（运行 check_features.sh）
#   5. 部署到服务器（调用 sudo bash deploy.sh）
#
# .build_features 已加入 .gitignore，每次本地生成即可。

set -euo pipefail

# ── 颜色 & 工具 ───────────────────────────────────────────────────────────────

C_GREEN='\033[0;32m'; C_YELLOW='\033[1;33m'; C_CYAN='\033[0;36m'
C_BOLD='\033[1m'; C_NC='\033[0m'

info()  { echo -e "${C_GREEN}[INFO]${C_NC}  $*"; }
warn()  { echo -e "${C_YELLOW}[WARN]${C_NC}  $*"; }
title() { echo -e "${C_BOLD}${C_CYAN}$*${C_NC}"; }

sep() { echo "  ──────────────────────────────────────────────────"; }

# 读取 Cargo.toml 中的 version
BOT_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

# 检查必须在项目根目录运行
[[ -f "Cargo.toml" ]] || { echo "请在 LianBot 项目根目录运行此脚本"; exit 1; }

# ── 当前选中的 feature 数组 ───────────────────────────────────────────────────

# 从 .build_features 加载，否则使用默认值
declare -A FEAT_SELECTED
ALL_FEATURES=(cmd-ping cmd-help cmd-alive cmd-img cmd-stalk cmd-smy core-pool-sqlite)
DEFAULT_ON=(cmd-ping cmd-help cmd-alive cmd-img cmd-stalk cmd-smy)

load_features() {
    # 默认全部设为 off
    for f in "${ALL_FEATURES[@]}"; do FEAT_SELECTED[$f]=0; done
    # 加载默认值
    for f in "${DEFAULT_ON[@]}"; do FEAT_SELECTED[$f]=1; done
    # 若 .build_features 存在，覆盖
    if [[ -f ".build_features" ]]; then
        for f in "${ALL_FEATURES[@]}"; do FEAT_SELECTED[$f]=0; done
        while IFS= read -r line || [[ -n "$line" ]]; do
            [[ -z "$line" ]] && continue
            FEAT_SELECTED[$line]=1
        done < ".build_features"
    fi
}

save_features() {
    : > ".build_features"
    for f in "${ALL_FEATURES[@]}"; do
        [[ ${FEAT_SELECTED[$f]} -eq 1 ]] && echo "$f" >> ".build_features"
    done
    info ".build_features 已保存"
}

features_to_flag() {
    local parts=()
    for f in "${ALL_FEATURES[@]}"; do
        [[ ${FEAT_SELECTED[$f]} -eq 1 ]] && parts+=("$f")
    done
    local IFS=','
    echo "${parts[*]}"
}

# ── 功能：选择编译模块 ────────────────────────────────────────────────────────

FEAT_LABELS=(
    "cmd-ping           /ping 命令（极轻量）"
    "cmd-help           /help 自动生成命令列表"
    "cmd-alive          /alive 存活检查"
    "cmd-img            <img> 发图命令"
    "cmd-stalk          <stalk> 截图（需 stalk_hypr 客户端）"
    "cmd-smy            <smy> 群聊日报（含 chrono、base64）"
    "core-pool-sqlite   SQLite 消息持久化（编译较慢，非默认）"
)

select_features() {
    load_features
    while true; do
        clear
        title "  LianBot v${BOT_VERSION}  —  编译模块选择"
        sep
        echo ""
        for i in "${!ALL_FEATURES[@]}"; do
            local f="${ALL_FEATURES[$i]}"
            local num=$((i + 1))
            if [[ ${FEAT_SELECTED[$f]} -eq 1 ]]; then
                echo -e "  ${C_GREEN}[x]${C_NC}  ${num}  ${FEAT_LABELS[$i]}"
            else
                echo    "  [ ]  ${num}  ${FEAT_LABELS[$i]}"
            fi
        done
        echo ""
        sep
        echo ""
        echo "  输入编号切换选中/取消，s 保存并返回，q 取消不保存："
        echo ""
        read -rp "  > " choice

        case "$choice" in
            [1-7])
                local idx=$((choice - 1))
                local f="${ALL_FEATURES[$idx]}"
                if [[ ${FEAT_SELECTED[$f]} -eq 1 ]]; then
                    FEAT_SELECTED[$f]=0
                else
                    FEAT_SELECTED[$f]=1
                fi
                # cmd-stalk 强制联动 core-ws（内部处理，无需暴露给用户）
                ;;
            s|S)
                save_features
                echo ""
                info "已选模块：$(features_to_flag)"
                echo ""
                read -rp "  按 Enter 返回主菜单..." _
                return
                ;;
            q|Q)
                load_features  # 恢复
                return
                ;;
            *)
                warn "请输入 1-7、s 或 q"
                sleep 0.5
                ;;
        esac
    done
}

# ── 功能：生成 config.toml ────────────────────────────────────────────────────

ask() {
    # ask <变量名> <提示> <默认值>
    local var="$1" prompt="$2" default="$3"
    local val
    read -rp "  ${prompt} [${default}]: " val
    val="${val:-$default}"
    printf -v "$var" '%s' "$val"
}

ask_optional() {
    # ask_optional <变量名> <提示> <留空说明>
    local var="$1" prompt="$2" note="$3"
    local val
    read -rp "  ${prompt}  (${note}，留空跳过): " val
    printf -v "$var" '%s' "$val"
}

gen_config() {
    load_features
    clear
    title "  LianBot v${BOT_VERSION}  —  生成 config.toml"
    sep
    echo ""

    local has_sqlite=0
    [[ ${FEAT_SELECTED[core-pool-sqlite]} -eq 1 ]] && has_sqlite=1

    # ── NapCat ────────────────────────────────────────────────────────────────
    echo "  [napcat]"
    ask NAPCAT_URL  "NapCat HTTP URL"   "http://127.0.0.1:3000"
    ask_optional NAPCAT_TOKEN "Bearer Token" "未设置则留空"
    echo ""

    # ── Server ────────────────────────────────────────────────────────────────
    echo "  [server]"
    ask SERVER_HOST "监听地址" "0.0.0.0"
    ask SERVER_PORT "监听端口" "8080"
    echo ""

    # ── Bot ───────────────────────────────────────────────────────────────────
    echo "  [bot]"
    echo "  群白名单（多个群号用英文逗号分隔，如 123456,789012）"
    read -rp "  whitelist: " WHITELIST_RAW
    WHITELIST_TOML="[$(echo "$WHITELIST_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"
    [[ "$WHITELIST_TOML" == "[]" ]] && warn "群白名单为空，Bot 将不响应任何群消息！"
    echo ""
    echo "  用户级过滤（留空 = 不限制，user_whitelist 优先级高于 user_blacklist）"
    ask_optional USER_WHITELIST "user_whitelist（QQ 号逗号分隔）" "不限制"
    ask_optional USER_BLACKLIST "user_blacklist（QQ 号逗号分隔）" "不限制"
    echo ""

    # ── Pool ──────────────────────────────────────────────────────────────────
    echo "  [pool]"
    ask POOL_CAPACITY    "每群内存缓冲最大条数" "2000"
    ask POOL_EVICT_SECS  "内存淘汰阈值（秒）" "86400"

    local SQLITE_BLOCK=""
    if [[ $has_sqlite -eq 1 ]]; then
        echo ""
        echo "  已选 core-pool-sqlite，配置 SQLite 参数："
        ask SQLITE_PATH      "SQLite 文件路径" "lianbot.db"
        ask SQLITE_RETAIN    "保留天数（超出则清理）" "30"
        ask SQLITE_MAX_ROWS  "每群最大保留条数" "50000"
        SQLITE_BLOCK=$(cat <<TOML

sqlite_path               = "$SQLITE_PATH"
sqlite_retain_days        = $SQLITE_RETAIN
sqlite_max_rows_per_group = $SQLITE_MAX_ROWS
TOML
)
    fi

    # 格式化 user 列表为 TOML 数组
    local UW_TOML="[]" UB_TOML="[]"
    if [[ -n "$USER_WHITELIST" ]]; then
        UW_TOML="[$(echo "$USER_WHITELIST" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"
    fi
    if [[ -n "$USER_BLACKLIST" ]]; then
        UB_TOML="[$(echo "$USER_BLACKLIST" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"
    fi

    # 预览
    echo ""
    sep
    local CONTENT
    CONTENT=$(cat <<TOML
[server]
host = "$SERVER_HOST"
port = $SERVER_PORT

[napcat]
url   = "$NAPCAT_URL"
token = "$NAPCAT_TOKEN"

[bot]
whitelist      = $WHITELIST_TOML
user_whitelist = $UW_TOML
user_blacklist = $UB_TOML

[pool]
per_group_capacity = $POOL_CAPACITY
evict_after_secs   = $POOL_EVICT_SECS$SQLITE_BLOCK
TOML
)
    echo ""
    echo "$CONTENT"
    echo ""
    sep
    echo ""

    if [[ -f "config.toml" ]]; then
        warn "config.toml 已存在，以上内容将覆盖它。"
    fi
    read -rp "  确认写入 config.toml？(y/N): " confirm
    if [[ "${confirm,,}" == "y" ]]; then
        echo "$CONTENT" > config.toml
        info "config.toml 已生成"
    else
        info "已取消，config.toml 未修改"
    fi
    echo ""
    read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能：生成 plugins.toml ───────────────────────────────────────────────────

gen_plugins() {
    load_features
    clear
    title "  LianBot v${BOT_VERSION}  —  生成 plugins.toml"
    sep
    echo ""

    local has_smy=0  has_alive=0
    [[ ${FEAT_SELECTED[cmd-smy]}   -eq 1 ]] && has_smy=1
    [[ ${FEAT_SELECTED[cmd-alive]} -eq 1 ]] && has_alive=1

    if [[ $has_smy -eq 0 && $has_alive -eq 0 ]]; then
        warn "当前未选择 cmd-smy 或 cmd-alive，plugins.toml 无需配置。"
        echo ""
        read -rp "  按 Enter 返回主菜单..." _
        return
    fi

    local CONTENT=""

    if [[ $has_smy -eq 1 ]]; then
        echo "  [smy]  群聊日报插件配置"
        ask SMY_COUNT  "默认拉取消息条数（10-2000）" "200"
        ask SMY_WIDTH  "截图宽度（像素）" "1200"
        echo ""
        local ENABLE_LLM=0 LLM_URL="" LLM_KEY="" LLM_MODEL=""
        read -rp "  是否启用 AI 总结（smy.llm）？(y/N): " _llm_confirm
        if [[ "${_llm_confirm,,}" == "y" ]]; then
            ENABLE_LLM=1
            ask LLM_URL   "OpenAI 兼容 API 地址" "https://api.deepseek.com/v1"
            ask LLM_KEY   "API Key" ""
            ask LLM_MODEL "模型名称" "deepseek-chat"
        fi
        echo ""
        CONTENT+=$(cat <<TOML
[smy]
default_count    = $SMY_COUNT
screenshot_width = $SMY_WIDTH
TOML
)
        if [[ $ENABLE_LLM -eq 1 ]]; then
            CONTENT+=$(cat <<TOML

[smy.llm]
api_url = "$LLM_URL"
api_key = "$LLM_KEY"
model   = "$LLM_MODEL"
TOML
)
        else
            CONTENT+=$'\n# [smy.llm]  取消注释并填入以启用 AI 总结\n# api_url = "https://api.deepseek.com/v1"\n# api_key  = "sk-xxx"\n# model    = "deepseek-chat"'
        fi
        CONTENT+=$'\n'
    fi

    if [[ $has_alive -eq 1 ]]; then
        echo "  [alive]  存活探测插件配置"
        ask ALIVE_URL     "探测 API 地址" "https://alive.example.com/api/status"
        ask ALIVE_TIMEOUT "超时秒数" "5"
        echo ""
        CONTENT+=$(cat <<TOML

[alive]
api_url      = "$ALIVE_URL"
timeout_secs = $ALIVE_TIMEOUT
TOML
)
        CONTENT+=$'\n'
    fi

    # 预览
    sep
    echo ""
    echo "$CONTENT"
    echo ""
    sep
    echo ""

    if [[ -f "plugins.toml" ]]; then
        warn "plugins.toml 已存在，以上内容将覆盖它。"
    fi
    read -rp "  确认写入 plugins.toml？(y/N): " confirm
    if [[ "${confirm,,}" == "y" ]]; then
        echo "$CONTENT" > plugins.toml
        info "plugins.toml 已生成"
    else
        info "已取消，plugins.toml 未修改"
    fi
    echo ""
    read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能：编译验证 ────────────────────────────────────────────────────────────

run_check() {
    clear
    title "  LianBot v${BOT_VERSION}  —  编译验证"
    sep
    echo ""
    if [[ ! -f "check_features.sh" ]]; then
        warn "未找到 check_features.sh，请确认在项目根目录"
        read -rp "  按 Enter 返回..." _
        return
    fi
    bash check_features.sh || true
    echo ""
    read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能：调用 deploy.sh ──────────────────────────────────────────────────────

run_deploy() {
    clear
    title "  LianBot v${BOT_VERSION}  —  部署到服务器"
    sep
    echo ""
    if [[ ! -f "deploy.sh" ]]; then
        warn "未找到 deploy.sh，请确认在项目根目录"
        read -rp "  按 Enter 返回..." _
        return
    fi
    if [[ ! -f ".build_features" ]]; then
        warn ".build_features 不存在，请先在「选择编译模块」中保存配置"
        read -rp "  按 Enter 返回..." _
        return
    fi
    if [[ ! -f "config.toml" ]]; then
        warn "config.toml 不存在，请先在「生成 config.toml」中生成配置"
        read -rp "  按 Enter 返回..." _
        return
    fi
    echo "  将运行：sudo bash deploy.sh"
    echo ""
    read -rp "  确认继续？(y/N): " confirm
    [[ "${confirm,,}" == "y" ]] || return
    echo ""
    sudo bash deploy.sh
    echo ""
    read -rp "  按 Enter 返回主菜单..." _
}

# ── 主菜单 ────────────────────────────────────────────────────────────────────

main_menu() {
    load_features
    while true; do
        clear
        title "  LianBot v${BOT_VERSION}  —  本地配置向导"
        sep
        echo ""
        # 显示当前状态
        local feat_str
        feat_str=$(features_to_flag)
        [[ -z "$feat_str" ]] && feat_str="（无，使用 --no-default-features）"
        echo "  当前模块  : $feat_str"
        local cfg_status="未生成" plg_status="未生成"
        [[ -f "config.toml" ]]  && cfg_status="已存在"
        [[ -f "plugins.toml" ]] && plg_status="已存在"
        echo "  config.toml   : $cfg_status"
        echo "  plugins.toml  : $plg_status"
        echo ""
        sep
        echo ""
        echo "  1  选择编译模块"
        echo "  2  生成 config.toml"
        echo "  3  生成 plugins.toml"
        echo "  4  编译验证（check_features.sh）"
        echo "  5  部署到服务器（deploy.sh）"
        echo ""
        echo "  0  退出"
        echo ""
        read -rp "  > " choice

        case "$choice" in
            1) select_features; load_features ;;
            2) gen_config ;;
            3) gen_plugins ;;
            4) run_check ;;
            5) run_deploy ;;
            0) echo ""; info "再见！"; echo ""; exit 0 ;;
            *) warn "请输入 0-5"; sleep 0.5 ;;
        esac
    done
}

main_menu
