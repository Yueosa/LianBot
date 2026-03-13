#!/usr/bin/env bash
# gen_logic.sh — 生成 logic.toml（业务逻辑层）
# 覆盖字段：smy / smy.llm / github / github.subscriptions / yiban / alive / acg / world

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
cd "$PROJECT_ROOT"

clear
title "  LianBot v${BOT_VERSION}  —  生成 logic.toml（业务逻辑层）"
sep
echo ""
[[ -f "logic.toml" ]] && info "检测到已有 logic.toml，回车保留原值。" && echo ""

LG="logic.toml"

# ── [smy] ─────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[smy]${C_NC}  群聊日报插件"
ask SMY_WIDTH "截图宽度（像素）" "$(toml_section_val "$LG" smy screenshot_width '1200')"
echo ""

# ── [github] ──────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[github]${C_NC}  GitHub Webhook"
_gs=$(toml_section_val "$LG" github secret "")
if [[ -n "$_gs" ]]; then
    ask GH_SECRET "Webhook Secret" "$_gs"
else
    ask_optional GH_SECRET "Webhook Secret" "留空禁用"
fi
echo ""

# ── [[github.subscriptions]] ─────────────────────────────────────────────────
echo "  ${C_BOLD}[[github.subscriptions]]${C_NC}  仓库订阅列表"

# 读取已有订阅
SUBS=()
if [[ -f "$LG" ]]; then
    # 解析已有 [[github.subscriptions]] 块（用 NUL 分隔完整块）
    while IFS= read -r -d '' block; do
        [[ -n "$block" ]] && SUBS+=("$block")
    done < <(awk '
        /^\[\[github\.subscriptions\]\]/ {
            if (in_sub && block!="") printf "%s\0", block
            in_sub=1; block=""; next
        }
        in_sub && /^\[/ {
            if (block!="") printf "%s\0", block
            in_sub=0; block=""
        }
        in_sub && !/^\s*$/ { block = block (block=="" ? "" : "\n") $0 }
        END { if (in_sub && block!="") printf "%s\0", block }
    ' "$LG")
fi

if [[ ${#SUBS[@]} -gt 0 ]]; then
    echo ""
    info "已有 ${#SUBS[@]} 条订阅规则："
    for i in "${!SUBS[@]}"; do
        _repo=$(echo -e "${SUBS[$i]}" | sed -n 's/.*repo.*=.*"\(.*\)".*/\1/p' | head -1)
        _user=$(echo -e "${SUBS[$i]}" | sed -n 's/.*user.*=.*"\(.*\)".*/\1/p' | head -1)
        _grp=$(echo -e "${SUBS[$i]}"  | sed -n 's/.*group.*=[ \t]*\([0-9]*\).*/\1/p' | head -1)
        _evt=$(echo -e "${SUBS[$i]}"  | sed -n 's/.*events.*=.*\[\(.*\)\].*/\1/p' | head -1)
        target="${_repo:-user:$_user}"
        echo "    $((i+1)). ${target}  →  群 ${_grp}  [${_evt}]"
    done
    echo ""
    echo "  操作：(k)保留已有  (c)清空重建  (a)追加新规则"
    read -rp "  > " sub_action
    case "${sub_action,,}" in
        c) SUBS=() ; info "已清空，从头添加" ;;
        a) ;;
        *) ;;  # 默认保留
    esac
fi

# 追加新订阅的交互循环
if [[ "${sub_action:-a}" != "k" ]] || [[ ${#SUBS[@]} -eq 0 ]]; then
    while true; do
        echo ""
        read -rp "  添加订阅规则？(y/N): " _add
        [[ "${_add,,}" == "y" ]] || break

        _s_repo="" _s_user="" _s_events="" _s_group="" _s_at=""
        echo "    repo 和 user 二选一（指定 repo 则忽略 user）"
        ask_optional _s_repo "repo (owner/repo 格式)" "如 YeaSrine/LianBot"
        if [[ -z "$_s_repo" ]]; then
            ask _s_user "user (账号/组织名)" ""
        fi
        ask _s_events "监听事件（逗号分隔）" "push,pull_request,issues,release"
        ask _s_group  "推送群号" "0"
        ask_optional _s_at "@ 的 QQ 号（逗号分隔）" "不 @"

        # 构建 TOML 块
        block=""
        if [[ -n "$_s_repo" ]]; then
            block+="repo   = \"$_s_repo\""$'\n'
        elif [[ -n "$_s_user" ]]; then
            block+="user   = \"$_s_user\""$'\n'
        fi
        # events array
        events_toml=$(echo "$_s_events" | tr ',' '\n' | sed 's/^ *//;s/ *$//' | grep -v '^$' \
                      | sed 's/.*/"&"/' | tr '\n' ',' | sed 's/,$//')
        block+="events = [$events_toml]"$'\n'
        block+="group  = $_s_group"
        # at array
        if [[ -n "$_s_at" ]]; then
            at_toml=$(echo "$_s_at" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')
            block+=$'\n'"at     = [$at_toml]"
        fi
        SUBS+=("$block")
        info "已添加（共 ${#SUBS[@]} 条）"
    done
fi
echo ""

# ── [yiban] ───────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[yiban]${C_NC}  易班签到 Webhook"
_ys=$(toml_section_val "$LG" yiban secret "")
if [[ -n "$_ys" ]]; then
    ask YIBAN_SECRET "HMAC Secret" "$_ys"
else
    ask_optional YIBAN_SECRET "HMAC Secret" "留空跳过验签"
fi
_ya=$(toml_section_val "$LG" yiban api_url "")
if [[ -n "$_ya" ]]; then
    ask YIBAN_API_URL "LianSign HTTP 地址" "$_ya"
else
    ask_optional YIBAN_API_URL "LianSign HTTP 地址" "如 http://127.0.0.1:9090（sign 命令需要）"
fi
_yt=$(toml_section_val "$LG" yiban api_token "")
if [[ -n "$_yt" ]]; then
    ask YIBAN_API_TOKEN "LianSign Bearer Token" "$_yt"
else
    ask_optional YIBAN_API_TOKEN "LianSign Bearer Token" "与 LianSign server.token 一致"
fi
ask YIBAN_REPLY_ORIGIN "命令触发后回源推送结果（true/false）" "$(toml_section_val "$LG" yiban reply_origin 'true')"
ask_optional YIBAN_AUTO_SIGN_HOUR "每日自动签到时间（0-23 整点，留空禁用）" "$(toml_section_val "$LG" yiban auto_sign_hour '')"
echo ""

# ── [[yiban.targets]] ────────────────────────────────────────────────────────
echo "  ${C_BOLD}[[yiban.targets]]${C_NC}  签到推送目标列表"

# 读取已有 targets
YIBAN_TARGETS=()
if [[ -f "$LG" ]]; then
    while IFS= read -r -d '' block; do
        [[ -n "$block" ]] && YIBAN_TARGETS+=("$block")
    done < <(awk '
        /^\[\[yiban\.targets\]\]/ {
            if (in_sub && block!="") printf "%s\0", block
            in_sub=1; block=""; next
        }
        in_sub && /^\[/ {
            if (block!="") printf "%s\0", block
            in_sub=0; block=""
        }
        in_sub && !/^\s*$/ { block = block (block=="" ? "" : "\n") $0 }
        END { if (in_sub && block!="") printf "%s\0", block }
    ' "$LG")
fi

if [[ ${#YIBAN_TARGETS[@]} -gt 0 ]]; then
    echo ""
    info "已有 ${#YIBAN_TARGETS[@]} 条推送规则："
    for i in "${!YIBAN_TARGETS[@]}"; do
        _users=$(echo -e "${YIBAN_TARGETS[$i]}" | sed -n 's/.*users.*=.*\[\(.*\)\].*/\1/p' | head -1)
        _grp=$(echo -e "${YIBAN_TARGETS[$i]}"  | sed -n 's/.*group.*=[ \t]*\([0-9]*\).*/\1/p' | head -1)
        _at=$(echo -e "${YIBAN_TARGETS[$i]}"   | sed -n 's/.*at.*=.*\[\(.*\)\].*/\1/p' | head -1)
        echo "    $((i+1)). users=[${_users}] → 群 ${_grp}  at=[${_at}]"
    done
    echo ""
    echo "  操作：(k)保留已有  (c)清空重建  (a)追加新规则"
    read -rp "  > " yiban_target_action
    case "${yiban_target_action,,}" in
        c) YIBAN_TARGETS=() ; info "已清空，从头添加" ;;
        a) ;;
        *) ;;  # 默认保留
    esac
fi

if [[ "${yiban_target_action:-a}" != "k" ]] || [[ ${#YIBAN_TARGETS[@]} -eq 0 ]]; then
    while true; do
        echo ""
        read -rp "  添加推送规则？(y/N): " _add_target
        [[ "${_add_target,,}" == "y" ]] || break

        _t_users="" _t_group="" _t_at=""
        ask_optional _t_users "匹配用户名（逗号分隔）" "留空匹配全部"
        ask _t_group "推送群号" "0"
        ask_optional _t_at "@ 的 QQ 号（逗号分隔）" "不 @"

        block=""
        if [[ -n "$_t_users" ]]; then
            users_toml=$(echo "$_t_users" | tr ',' '\n' | sed 's/^ *//;s/ *$//' | grep -v '^$' \
                         | sed 's/.*/"&"/' | tr '\n' ',' | sed 's/,$//')
            block+="users = [$users_toml]"$'\n'
        else
            block+="users = []"$'\n'
        fi
        block+="group = $_t_group"
        if [[ -n "$_t_at" ]]; then
            at_toml=$(echo "$_t_at" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')
            block+=$'\n'"at    = [$at_toml]"
        fi
        YIBAN_TARGETS+=("$block")
        info "已添加（共 ${#YIBAN_TARGETS[@]} 条）"
    done
fi
echo ""

# ── [alive] ───────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[alive]${C_NC}  设备在线状态探测"
ask ALIVE_URL     "探测 API 地址" "$(toml_section_val "$LG" alive api_url 'https://alive.example.com/api/status')"
ask ALIVE_TIMEOUT "超时秒数"     "$(toml_section_val "$LG" alive timeout_secs '5')"
echo ""

# ── [acg] ─────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[acg]${C_NC}  随机二次元图片"
ask ACG_URL "API 地址" "$(toml_section_val "$LG" acg api_url 'https://www.loliapi.com/bg/')"
echo ""

# ── [world] ───────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[world]${C_NC}  60 秒看世界"
ask WORLD_URL "API 地址" "$(toml_section_val "$LG" world api_url 'https://api.ecylt.com/v1/world_60s')"
echo ""

# ── [chat] ────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[chat]${C_NC}  AI 对话（@Bot 触发）"
echo "  所有字段均可选，使用默认值可直接回车跳过"
ask CHAT_CTX_SIZE   "上下文条数"         "$(toml_section_val "$LG" chat context_size '50')"
ask CHAT_CTX_WINDOW "上下文时间窗口（秒）" "$(toml_section_val "$LG" chat context_window '7200')"
ask CHAT_TEMP       "LLM temperature"   "$(toml_section_val "$LG" chat temperature '0.8')"
ask CHAT_MAX_TOKENS "max_tokens"        "$(toml_section_val "$LG" chat max_tokens '2048')"
ask CHAT_TOOLS      "启用 Tool-Call（true/false）" "$(toml_section_val "$LG" chat enable_tools 'false')"
echo "  人格设定（persona）请直接编辑 logic.toml [chat] 段修改"
echo ""

# ── 生成内容 ──────────────────────────────────────────────────────────────────

CONTENT="# LianBot 业务逻辑配置 (logic 层)

[smy]
screenshot_width = $SMY_WIDTH"

CONTENT+="

[github]
secret = \"${GH_SECRET:-}\""

for sub in "${SUBS[@]}"; do
    CONTENT+=$'\n\n'"[[github.subscriptions]]"$'\n'"$(printf '%s' "$sub")"
done

CONTENT+="

[yiban]
secret = \"${YIBAN_SECRET:-}\"
reply_origin = $YIBAN_REPLY_ORIGIN"
if [[ -n "${YIBAN_AUTO_SIGN_HOUR:-}" ]]; then
    CONTENT+=$'\n'"auto_sign_hour = $YIBAN_AUTO_SIGN_HOUR"
fi
if [[ -n "${YIBAN_API_URL:-}" ]]; then
    CONTENT+=$'\n'"api_url   = \"$YIBAN_API_URL\""
fi
if [[ -n "${YIBAN_API_TOKEN:-}" ]]; then
    CONTENT+=$'\n'"api_token = \"$YIBAN_API_TOKEN\""
fi

for target in "${YIBAN_TARGETS[@]}"; do
    CONTENT+=$'\n\n'"[[yiban.targets]]"$'\n'"$(printf '%s' "$target")"
done

CONTENT+="

[alive]
api_url      = \"$ALIVE_URL\"
timeout_secs = $ALIVE_TIMEOUT

[acg]
api_url = \"$ACG_URL\"

[world]
api_url = \"$WORLD_URL\"

[chat]
context_size   = $CHAT_CTX_SIZE
context_window = $CHAT_CTX_WINDOW
temperature    = $CHAT_TEMP
max_tokens     = $CHAT_MAX_TOKENS
enable_tools   = $CHAT_TOOLS"

echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
[[ -f "$LG" ]] && warn "logic.toml 已存在，将被覆盖。"
read -rp "  确认写入 logic.toml？(y/N): " confirm
if [[ "${confirm,,}" == "y" ]]; then
    echo "$CONTENT" > "$LG"
    info "logic.toml 已生成"
else
    info "已取消"
fi
