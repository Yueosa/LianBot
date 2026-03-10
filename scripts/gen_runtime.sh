#!/usr/bin/env bash
# gen_runtime.sh — 生成 runtime.toml（运行时层）
# 覆盖字段：time / bot / napcat / parser / pool / log

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
cd "$PROJECT_ROOT"

clear
title "  LianBot v${BOT_VERSION}  —  生成 runtime.toml（运行时层）"
sep
echo ""
[[ -f "runtime.toml" ]] && info "检测到已有 runtime.toml，回车保留原值。" && echo ""

RT="runtime.toml"

# ── [time] ────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[time]${C_NC}  时区设置"
ask TIMEZONE "UTC 偏移小时数（中国 = 8）" "$(toml_section_val "$RT" time timezone '8')"
echo ""

# ── [bot] ─────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[bot]${C_NC}  Bot 身份与权限"
ask BOT_ID "Bot 自身 QQ 号" "$(toml_section_val "$RT" bot bot_id '0')"
ask OWNER "Bot 主人 QQ 号" "$(toml_section_val "$RT" bot owner '0')"
ask DB_PATH "权限数据库路径" "$(toml_section_val "$RT" bot db_path 'permissions.db')"

_gl=$(toml_section_arr "$RT" bot initial_groups)
echo "  初始群列表（启动时导入 DB，多个用逗号分隔）"
read -rp "  initial_groups [${_gl:-（空）}]: " GROUPS_RAW
GROUPS_RAW="${GROUPS_RAW:-$_gl}"
GROUPS_TOML="[$(echo "$GROUPS_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"

_gbl=$(toml_section_arr "$RT" bot group_blacklist)
if [[ -n "$_gbl" ]]; then
    ask GBL_RAW "群聊黑名单（QQ 号逗号分隔）" "$_gbl"
else
    ask_optional GBL_RAW "群聊黑名单（QQ 号逗号分隔）" "不限制"
fi
GBL_TOML="[]"
[[ -n "${GBL_RAW:-}" ]] && GBL_TOML="[$(echo "$GBL_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"

_pbl=$(toml_section_arr "$RT" bot private_blacklist)
if [[ -n "$_pbl" ]]; then
    ask PBL_RAW "私聊黑名单（QQ 号逗号分隔）" "$_pbl"
else
    ask_optional PBL_RAW "私聊黑名单（QQ 号逗号分隔）" "不限制"
fi
PBL_TOML="[]"
[[ -n "${PBL_RAW:-}" ]] && PBL_TOML="[$(echo "$PBL_RAW" | tr ',' '\n' | tr -d ' ' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')]"
echo ""

# ── [napcat] ──────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[napcat]${C_NC}  NapCat HTTP API"
ask NAPCAT_URL "NapCat HTTP URL" "$(toml_section_val "$RT" napcat url 'http://127.0.0.1:3000')"
_tok=$(toml_section_val "$RT" napcat token "")
if [[ -n "$_tok" ]]; then
    ask NAPCAT_TOKEN "Bearer Token" "$_tok"
else
    ask_optional NAPCAT_TOKEN "Bearer Token" "未设置则留空"
fi
echo ""

# ── [parser] ──────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[parser]${C_NC}  命令解析器"
ask CMD_PREFIX "简单命令前缀" "$(toml_section_val "$RT" parser cmd_prefix '!!')"
echo ""

# ── [pool] ────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[pool]${C_NC}  消息池"
ask POOL_CAP   "每群内存缓冲最大条数" "$(toml_section_val "$RT" pool per_group_capacity '3000')"
ask POOL_EVICT "内存淘汰阈值（秒）"   "$(toml_section_val "$RT" pool evict_after_secs '86400')"
echo ""

# ── [llm] ─────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[llm]${C_NC}  LLM / AI 模型（smy AI 总结、未来 AI 对话等）"
_llm_key=$(toml_section_val "$RT" llm api_key "")
_llm_hint="N"
[[ -n "$_llm_key" ]] && _llm_hint="Y（已配置）"
read -rp "  是否启用 LLM？(y/N) [${_llm_hint}]: " _llm_confirm
ENABLE_LLM=0 LLM_URL="" LLM_KEY="" LLM_MODEL="" LLM_TIMEOUT=""
if [[ "${_llm_confirm,,}" == "y" ]] || [[ -z "$_llm_confirm" && -n "$_llm_key" ]]; then
    ENABLE_LLM=1
    ask LLM_URL     "OpenAI 兼容 API 地址" "$(toml_section_val "$RT" llm api_url 'https://api.deepseek.com/v1')"
    ask LLM_KEY     "API Key"               "$(toml_section_val "$RT" llm api_key '')"
    ask LLM_MODEL   "模型名称"               "$(toml_section_val "$RT" llm model  'deepseek-chat')"
    ask LLM_TIMEOUT "请求超时（秒）"         "$(toml_section_val "$RT" llm timeout_secs '120')"
fi
echo ""

# ── [log] ─────────────────────────────────────────────────────────────────────
echo "  ${C_BOLD}[log]${C_NC}  日志"
ask LOG_LEVEL "日志级别（trace/debug/info/warn/error）" "$(toml_section_val "$RT" log level 'info')"
_ldir=$(toml_section_val "$RT" log log_dir "")
if [[ -n "$_ldir" ]]; then
    ask LOG_DIR "log_dir" "$_ldir"
else
    ask_optional LOG_DIR "log_dir（如 /opt/lianbot/logs）" "仅 stdout"
fi
LOG_DIR_LINE="" LOG_MAX_LINE=""
if [[ -n "${LOG_DIR:-}" ]]; then
    ask LOG_MAX "日志保留天数" "$(toml_section_val "$RT" log max_days '30')"
    LOG_DIR_LINE="log_dir  = \"$LOG_DIR\""
    LOG_MAX_LINE="max_days = $LOG_MAX"
fi
echo ""

# ── 生成 ──────────────────────────────────────────────────────────────────────

CONTENT="# LianBot 运行时配置 (runtime 层)

[time]
timezone = $TIMEZONE

[bot]
bot_id         = $BOT_ID
owner          = $OWNER
db_path        = \"$DB_PATH\"
initial_groups    = $GROUPS_TOML
group_blacklist   = $GBL_TOML
private_blacklist = $PBL_TOML

[napcat]
url   = \"$NAPCAT_URL\"
token = \"${NAPCAT_TOKEN:-}\"

[parser]
cmd_prefix = \"$CMD_PREFIX\"

[pool]
per_group_capacity = $POOL_CAP
evict_after_secs   = $POOL_EVICT

[log]"
[[ -n "$LOG_DIR_LINE" ]] && CONTENT+=$'\n'"$LOG_DIR_LINE"
[[ -n "$LOG_MAX_LINE" ]] && CONTENT+=$'\n'"$LOG_MAX_LINE"
CONTENT+=$'\n'"level = \"$LOG_LEVEL\""

if [[ $ENABLE_LLM -eq 1 ]]; then
    CONTENT+="

[llm]
api_url      = \"$LLM_URL\"
api_key      = \"$LLM_KEY\"
model        = \"$LLM_MODEL\"
timeout_secs = $LLM_TIMEOUT"
else
    CONTENT+=$'\n'"# [llm]  取消注释并填入以启用 AI 功能"
    CONTENT+=$'\n'"# api_url      = \"https://api.deepseek.com/v1\""
    CONTENT+=$'\n'"# api_key      = \"sk-xxx\""
    CONTENT+=$'\n'"# model        = \"deepseek-chat\""
    CONTENT+=$'\n'"# timeout_secs = 120"
fi

echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
[[ -f "$RT" ]] && warn "runtime.toml 已存在，将被覆盖。"
read -rp "  确认写入 runtime.toml？(y/N): " confirm
if [[ "${confirm,,}" == "y" ]]; then
    echo "$CONTENT" > "$RT"
    info "runtime.toml 已生成"
else
    info "已取消"
fi
