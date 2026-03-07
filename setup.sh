#!/usr/bin/env bash
# setup.sh — LianBot 本地配置向导
#
# 不需要 root，在项目根目录运行：
#   bash setup.sh
#
# 功能：
#   1. 生成 config.toml（kernel 层：host / port）
#   2. 生成 runtime.toml（运行时：napcat / bot / pool / log）
#   3. 生成 logic.toml（业务逻辑：smy / github / alive）
#   4. 编译验证（运行 check_features.sh）
#   5. 部署到服务器（调用 sudo bash deploy.sh）
#
# 三层配置架构（v0.2.0+）：
#   config.toml   — kernel 层，仅 host / port
#   runtime.toml  — 运行时基础设施，napcat / bot / pool / log / parser
#   logic.toml    — 插件配置，smy / github / alive

set -euo pipefail

# ── 颜色 & 工具 ───────────────────────────────────────────────────────────────

C_GREEN='\033[0;32m'; C_YELLOW='\033[1;33m'; C_CYAN='\033[0;36m'
C_BOLD='\033[1m'; C_NC='\033[0m'

info()  { echo -e "${C_GREEN}[INFO]${C_NC}  $*"; }
warn()  { echo -e "${C_YELLOW}[WARN]${C_NC}  $*"; }
title() { echo -e "${C_BOLD}${C_CYAN}$*${C_NC}"; }

sep() { echo "  ──────────────────────────────────────────────────"; }

BOT_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
[[ -f "Cargo.toml" ]] || { echo "请在 LianBot 项目根目录运行此脚本"; exit 1; }

# ── 通用读取函数 ──────────────────────────────────────────────────────────────

# toml_val <file> <key> <fallback>  →  输出直属于文件顶层或当前 section 的标量值
toml_val() {
    local file="$1" key="$2" fallback="$3"
    if [[ ! -f "$file" ]]; then echo "$fallback"; return; fi
    local val
    val=$(grep -E "^\s*${key}\s*=" "$file" | head -1 \
          | sed 's/[^=]*=[ \t]*//' | sed 's/^"//' | sed 's/"$//' | sed 's/^[ \t]*//' || true)
    echo "${val:-$fallback}"
}

# toml_section_val <file> <section> <key> <fallback>
toml_section_val() {
    local file="$1" section="$2" key="$3" fallback="$4"
    if [[ ! -f "$file" ]]; then echo "$fallback"; return; fi
    local val
    val=$(awk -v sec="[${section}]" -v k="${key}" '
        $0 == sec       { in_sec=1; next }
        /^\[/           { in_sec=0 }
        in_sec && $1==k {
            sub(/[^=]+=[ \t]*/, "")
            gsub(/["'"'"']/, "")
            sub(/[ \t]+$/, "")
            print; exit
        }
    ' "$file")
    echo "${val:-$fallback}"
}

# toml_section_arr <file> <section> <key>  →  逗号分隔数组值
toml_section_arr() {
    local file="$1" section="$2" key="$3"
    if [[ ! -f "$file" ]]; then echo ""; return; fi
    awk -v sec="[${section}]" -v k="${key}" '
        $0 == sec       { in_sec=1; next }
        /^\[/           { in_sec=0 }
        in_sec && $1==k {
            if (match($0, /\[([^\]]*)\]/, a)) {
                gsub(/ /, "", a[1])
                print a[1]
            }
            exit
        }
    ' "$file"
}

ask() {
    local var="$1" prompt="$2" default="$3"
    local val
    read -rp "  ${prompt} [${default}]: " val
    val="${val:-$default}"
    printf -v "$var" '%s' "$val"
}

ask_optional() {
    local var="$1" prompt="$2" note="$3"
    local val
    read -rp "  ${prompt}  (${note}，留空跳过): " val
    printf -v "$var" '%s' "$val"
}

# ── 功能 1：生成 config.toml（kernel 层）──────────────────────────────────────

gen_config() {
    clear
    title "  LianBot v${BOT_VERSION}  —  生成 config.toml（kernel 层）"
    sep
    echo ""
    [[ -f "config.toml" ]] && info "检测到已有 config.toml，回车保留原值。" && echo ""

    local HOST PORT
    ask HOST "监听地址" "$(toml_val config.toml host '0.0.0.0')"
    ask PORT "监听端口" "$(toml_val config.toml port '8080')"

    local CONTENT
    CONTENT=$(cat <<TOML
# LianBot 内核配置 (kernel 层)
host = "$HOST"
port = $PORT
TOML
)
    echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
    [[ -f "config.toml" ]] && warn "config.toml 已存在，将被覆盖。"
    read -rp "  确认写入 config.toml？(y/N): " confirm
    if [[ "${confirm,,}" == "y" ]]; then
        echo "$CONTENT" > config.toml
        info "config.toml 已生成"
    else
        info "已取消"
    fi
    echo ""; read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能 2：生成 runtime.toml ─────────────────────────────────────────────────

gen_runtime() {
    clear
    title "  LianBot v${BOT_VERSION}  —  生成 runtime.toml（运行时层）"
    sep
    echo ""
    [[ -f "runtime.toml" ]] && info "检测到已有 runtime.toml，回车保留原值。" && echo ""

    local RT="runtime.toml"

    # ── [bot] ─────────────────────────────────────────────────────────────────
    echo "  [bot]  Bot 身份与权限"
    local OWNER GROUPS_RAW BLACKLIST_RAW
    ask OWNER "Bot 主人 QQ 号" "$(toml_section_val "$RT" bot owner '0')"

    local _gl; _gl=$(toml_section_arr "$RT" bot initial_groups)
    echo "  初始群列表（启动时导入 DB，多个用逗号分隔）"
    read -rp "  initial_groups [${_gl:-（空）}]: " GROUPS_RAW
    GROUPS_RAW="${GROUPS_RAW:-$_gl}"
    local GROUPS_TOML="[$(echo "$GROUPS_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"

    local _bl; _bl=$(toml_section_arr "$RT" bot blacklist)
    if [[ -n "$_bl" ]]; then
        ask BLACKLIST_RAW "静态黑名单（QQ 号逗号分隔）" "$_bl"
    else
        ask_optional BLACKLIST_RAW "静态黑名单（QQ 号逗号分隔）" "不限制"
    fi
    local BL_TOML="[]"
    [[ -n "$BLACKLIST_RAW" ]] && BL_TOML="[$(echo "$BLACKLIST_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"
    echo ""

    # ── [napcat] ──────────────────────────────────────────────────────────────
    echo "  [napcat]  NapCat HTTP API"
    local NAPCAT_URL NAPCAT_TOKEN
    ask NAPCAT_URL   "NapCat HTTP URL"  "$(toml_section_val "$RT" napcat url 'http://127.0.0.1:3000')"
    local _tok; _tok=$(toml_section_val "$RT" napcat token "")
    if [[ -n "$_tok" ]]; then
        ask NAPCAT_TOKEN "Bearer Token" "$_tok"
    else
        ask_optional NAPCAT_TOKEN "Bearer Token" "未设置则留空"
    fi
    echo ""

    # ── [pool] ────────────────────────────────────────────────────────────────
    echo "  [pool]  消息池"
    local POOL_CAP POOL_EVICT
    ask POOL_CAP   "每群内存缓冲最大条数" "$(toml_section_val "$RT" pool per_group_capacity '3000')"
    ask POOL_EVICT "内存淘汰阈值（秒）"   "$(toml_section_val "$RT" pool evict_after_secs '86400')"
    echo ""

    # ── [log] ─────────────────────────────────────────────────────────────────
    echo "  [log]  日志"
    local LOG_LEVEL LOG_DIR LOG_MAX
    ask LOG_LEVEL "日志级别（trace/debug/info/warn/error）" "$(toml_section_val "$RT" log level 'info')"

    local _ldir; _ldir=$(toml_section_val "$RT" log log_dir "")
    if [[ -n "$_ldir" ]]; then
        ask LOG_DIR "log_dir" "$_ldir"
    else
        ask_optional LOG_DIR "log_dir（如 /opt/lianbot/logs）" "仅 stdout"
    fi
    local LOG_DIR_BLOCK="" LOG_MAX_BLOCK=""
    if [[ -n "$LOG_DIR" ]]; then
        ask LOG_MAX "日志保留天数" "$(toml_section_val "$RT" log max_days '30')"
        LOG_DIR_BLOCK="log_dir  = \"$LOG_DIR\""
        LOG_MAX_BLOCK="max_days = $LOG_MAX"
    fi
    echo ""

    # ── 生成内容 ──────────────────────────────────────────────────────────────
    local CONTENT
    CONTENT="# LianBot 运行时配置 (runtime 层)

[bot]
owner          = $OWNER
initial_groups = $GROUPS_TOML
blacklist      = $BL_TOML

[napcat]
url   = \"$NAPCAT_URL\"
token = \"$NAPCAT_TOKEN\"

[pool]
per_group_capacity = $POOL_CAP
evict_after_secs   = $POOL_EVICT

[log]"
    [[ -n "$LOG_DIR_BLOCK" ]] && CONTENT+="
$LOG_DIR_BLOCK"
    [[ -n "$LOG_MAX_BLOCK" ]] && CONTENT+="
$LOG_MAX_BLOCK"
    CONTENT+="
level = \"$LOG_LEVEL\""

    echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
    [[ -f "runtime.toml" ]] && warn "runtime.toml 已存在，将被覆盖。"
    read -rp "  确认写入 runtime.toml？(y/N): " confirm
    if [[ "${confirm,,}" == "y" ]]; then
        echo "$CONTENT" > runtime.toml
        info "runtime.toml 已生成"
    else
        info "已取消"
    fi
    echo ""; read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能 3：生成 logic.toml ───────────────────────────────────────────────────

gen_logic() {
    clear
    title "  LianBot v${BOT_VERSION}  —  生成 logic.toml（业务逻辑层）"
    sep
    echo ""
    [[ -f "logic.toml" ]] && info "检测到已有 logic.toml，回车保留原值。" && echo ""

    local LG="logic.toml"
    local CONTENT=""

    # ── [smy] ─────────────────────────────────────────────────────────────────
    echo "  [smy]  群聊日报插件"
    local SMY_WIDTH
    ask SMY_WIDTH "截图宽度（像素）" "$(toml_section_val "$LG" smy screenshot_width '1200')"
    echo ""

    local ENABLE_LLM=0 LLM_URL="" LLM_KEY="" LLM_MODEL=""
    local _llm_key; _llm_key=$(toml_section_val "$LG" "smy.llm" api_key "")
    local _llm_hint="N"
    [[ -n "$_llm_key" ]] && _llm_hint="Y（已配置）"
    read -rp "  是否启用 AI 总结（smy.llm）？(y/N) [${_llm_hint}]: " _llm_confirm
    if [[ "${_llm_confirm,,}" == "y" ]] || [[ -z "$_llm_confirm" && -n "$_llm_key" ]]; then
        ENABLE_LLM=1
        ask LLM_URL   "OpenAI 兼容 API 地址" "$(toml_section_val "$LG" "smy.llm" api_url 'https://api.deepseek.com/v1')"
        ask LLM_KEY   "API Key"               "$(toml_section_val "$LG" "smy.llm" api_key '')"
        ask LLM_MODEL "模型名称"               "$(toml_section_val "$LG" "smy.llm" model  'deepseek-chat')"
    fi
    echo ""

    CONTENT+="[smy]
screenshot_width = $SMY_WIDTH"
    if [[ $ENABLE_LLM -eq 1 ]]; then
        CONTENT+="

[smy.llm]
api_url = \"$LLM_URL\"
api_key = \"$LLM_KEY\"
model   = \"$LLM_MODEL\""
    else
        CONTENT+=$'\n# [smy.llm]  取消注释并填入以启用 AI 总结\n# api_url = "https://api.deepseek.com/v1"\n# api_key = "sk-xxx"\n# model   = "deepseek-chat"'
    fi

    # ── [alive] ───────────────────────────────────────────────────────────────
    echo "  [alive]  设备在线状态探测"
    local ALIVE_URL ALIVE_TIMEOUT
    ask ALIVE_URL     "探测 API 地址" "$(toml_section_val "$LG" alive api_url 'https://alive.example.com/api/status')"
    ask ALIVE_TIMEOUT "超时秒数"     "$(toml_section_val "$LG" alive timeout_secs '5')"
    echo ""

    CONTENT+="

[alive]
api_url      = \"$ALIVE_URL\"
timeout_secs = $ALIVE_TIMEOUT"

    # ── [github] ──────────────────────────────────────────────────────────────
    echo "  [github]  GitHub Webhook（留空则禁用）"
    local GH_SECRET
    local _gs; _gs=$(toml_section_val "$LG" github secret "")
    if [[ -n "$_gs" ]]; then
        ask GH_SECRET "Webhook Secret" "$_gs"
    else
        ask_optional GH_SECRET "Webhook Secret" "留空禁用"
    fi
    echo ""

    CONTENT+="

[github]
secret = \"${GH_SECRET:-}\""

    echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
    [[ -f "logic.toml" ]] && warn "logic.toml 已存在，将被覆盖。"
    read -rp "  确认写入 logic.toml？(y/N): " confirm
    if [[ "${confirm,,}" == "y" ]]; then
        echo "$CONTENT" > logic.toml
        info "logic.toml 已生成"
    else
        info "已取消"
    fi
    echo ""; read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能 4：编译验证 ──────────────────────────────────────────────────────────

run_check() {
    clear
    title "  LianBot v${BOT_VERSION}  —  编译验证"
    sep
    echo ""
    if [[ ! -f "check_features.sh" ]]; then
        warn "未找到 check_features.sh"
        read -rp "  按 Enter 返回..." _
        return
    fi
    bash check_features.sh || true
    echo ""; read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能 5：部署到服务器 ──────────────────────────────────────────────────────

run_deploy() {
    clear
    title "  LianBot v${BOT_VERSION}  —  部署到服务器"
    sep
    echo ""
    [[ ! -f "deploy.sh" ]] && { warn "未找到 deploy.sh"; read -rp "  按 Enter 返回..." _; return; }
    [[ ! -f "config.toml" ]] && { warn "config.toml 不存在，请先生成配置"; read -rp "  按 Enter 返回..." _; return; }

    echo "  将运行：sudo env PATH=\"$PATH\" bash deploy.sh"
    echo ""
    read -rp "  确认继续？(y/N): " confirm
    [[ "${confirm,,}" == "y" ]] || return
    echo ""
    sudo env PATH="$PATH" bash deploy.sh
    echo ""; read -rp "  按 Enter 返回主菜单..." _
}

# ── 功能 6：查看日志 ──────────────────────────────────────────────────────────

show_logs() {
    local deployed_cfg="${LIANBOT_DIR:-/opt/lianbot}/runtime.toml"
    local read_from="runtime.toml"
    [[ -f "$deployed_cfg" ]] && read_from="$deployed_cfg"

    local log_dir=""
    [[ -f "$read_from" ]] && log_dir=$(grep -E '^\s*log_dir\s*=' "$read_from" 2>/dev/null \
                  | head -1 | sed 's/.*=\s*"\(.*\)".*/\1/' || true)

    if [[ -n "$log_dir" ]]; then
        local today
        today=$(date -u +%Y-%m-%d)
        local log_file="${log_dir}/lianbot.log.utc.${today}"

        if [[ ! -f "$log_file" ]]; then
            local latest
            latest=$(ls -1t "${log_dir}"/lianbot.log.utc.* 2>/dev/null | head -1)
            if [[ -n "$latest" ]]; then
                warn "今日日志尚未生成，显示最近文件：$latest"
                echo ""
                log_file="$latest"
            else
                warn "日志目录 $log_dir 中暂无日志文件"
                read -rp "  按 Enter 返回主菜单..." _
                return
            fi
        fi
        info "实时跟踪日志文件：$log_file（Ctrl-C 退出）"
        echo ""
        tail -f "$log_file"
    else
        info "未配置 log_dir，回退到 journald（Ctrl-C 退出）"
        echo ""
        sudo journalctl -u lianbot -f
    fi
}

# ── 主菜单 ────────────────────────────────────────────────────────────────────

main_menu() {
    while true; do
        clear
        title "  LianBot v${BOT_VERSION}  —  本地配置向导"
        sep
        echo ""
        # 状态
        local cfg_s="❌" rt_s="❌" lg_s="❌"
        [[ -f "config.toml" ]]  && cfg_s="✅"
        [[ -f "runtime.toml" ]] && rt_s="✅"
        [[ -f "logic.toml" ]]   && lg_s="✅"

        echo "  config.toml   $cfg_s  (kernel: host/port)"
        echo "  runtime.toml  $rt_s  (napcat/bot/pool/log)"
        echo "  logic.toml    $lg_s  (smy/github/alive)"
        echo ""
        sep
        echo ""
        echo "  1  生成 config.toml（kernel 层）"
        echo "  2  生成 runtime.toml（运行时层）"
        echo "  3  生成 logic.toml（业务逻辑层）"
        echo "  4  编译验证（check_features.sh）"
        echo "  5  部署到服务器（deploy.sh）"
        echo "  6  实时查看日志"
        echo ""
        echo "  0  退出"
        echo ""
        read -rp "  > " choice

        case "$choice" in
            1) gen_config ;;
            2) gen_runtime ;;
            3) gen_logic ;;
            4) run_check ;;
            5) run_deploy ;;
            6) show_logs ;;
            0) echo ""; info "再见！"; echo ""; exit 0 ;;
            *) warn "请输入 0-6"; sleep 0.5 ;;
        esac
    done
}

main_menu
