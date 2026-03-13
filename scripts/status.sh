#!/usr/bin/env bash
# status.sh — 查看 LianBot 部署状态
# 展示：服务状态 / 部署目录 / 配置摘要 / 磁盘占用

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

clear
title "  LianBot v${BOT_VERSION}  —  部署状态"
sep
echo ""

# ── 服务状态 ──────────────────────────────────────────────────────────────────

echo "  ${C_BOLD}服务状态${C_NC}"
if systemctl is-active lianbot &>/dev/null; then
    echo -e "    lianbot.service: ${C_GREEN}● 运行中${C_NC}"
    _uptime=$(systemctl show lianbot --property=ActiveEnterTimestamp --value 2>/dev/null || true)
    [[ -n "$_uptime" ]] && echo "    启动时间: $_uptime"
else
    echo -e "    lianbot.service: ${C_RED}● 已停止${C_NC}"
fi
echo ""

# ── 部署目录 ──────────────────────────────────────────────────────────────────

echo "  ${C_BOLD}部署目录${C_NC}  $LIANBOT_DIR"
if [[ -d "$LIANBOT_DIR" ]]; then
    ls -lhA "$LIANBOT_DIR" 2>/dev/null | tail -n +2 | sed 's/^/    /'
    echo ""
    _size=$(du -sh "$LIANBOT_DIR" 2>/dev/null | cut -f1)
    echo "    总计: ${_size:-未知}"
else
    warn "  目录不存在"
fi
echo ""

# ── 配置摘要 ──────────────────────────────────────────────────────────────────

# 优先读部署目录的配置，否则读项目本地
# 部署目录文件可能是 640:lianbot，用 _read_cfg 处理权限
_cfg_dir="$LIANBOT_DIR"
[[ -f "$_cfg_dir/config.toml" ]] || _cfg_dir="$PROJECT_ROOT"

_read_cfg() {
    if [[ -r "$1" ]]; then cat "$1"; else sudo cat "$1" 2>/dev/null; fi
}

echo "  ${C_BOLD}配置摘要${C_NC}"

# kernel
if [[ -f "$_cfg_dir/config.toml" ]]; then
    _host=$(toml_val "$_cfg_dir/config.toml" host "?")
    _port=$(toml_val "$_cfg_dir/config.toml" port "?")
    echo "    [kernel]  ${_host}:${_port}"
else
    dim "    config.toml 不存在"
fi

# runtime
_rt="$_cfg_dir/runtime.toml"
if [[ -f "$_rt" ]]; then
    _owner=$(toml_section_val "$_rt" bot owner "?")
    _groups=$(toml_section_arr "$_rt" bot initial_groups)
    _nurl=$(toml_section_val "$_rt" napcat url "?")
    _level=$(toml_section_val "$_rt" log level "info")
    _ldir=$(toml_section_val "$_rt" log log_dir "")
    _prefix=$(toml_section_val "$_rt" parser cmd_prefix "!!")
    _tz=$(toml_section_val "$_rt" time timezone "8")

    echo "    [bot]     owner=$_owner  groups=[$_groups]"
    echo "    [napcat]  $_nurl"
    echo "    [parser]  prefix=\"$_prefix\"  timezone=UTC+$_tz"
    echo "    [log]     level=$_level  dir=${_ldir:-(stdout)}"
else
    dim "    runtime.toml 不存在"
fi

# logic
_lg="$_cfg_dir/logic.toml"
if [[ -f "$_lg" ]]; then
    _smy_w=$(toml_section_val "$_lg" smy screenshot_width "1200")
    _llm=$(toml_section_val "$_lg" "smy.llm" model "")
    _gh_secret=$(toml_section_val "$_lg" github secret "")
    _gh_subs=$(grep -c '^\[\[github\.subscriptions\]\]' "$_lg" 2>/dev/null || echo 0)
    _alive=$(toml_section_val "$_lg" alive api_url "")
    _yiban_targets=$(grep -c '^\[\[yiban\.targets\]\]' "$_lg" 2>/dev/null || echo 0)
    _yiban_api=$(toml_section_val "$_lg" yiban api_url "")

    echo "    [smy]     width=${_smy_w}  llm=${_llm:-(禁用)}"
    echo "    [github]  secret=${_gh_secret:+(已设置)}${_gh_secret:-(空)}  subscriptions=${_gh_subs}条"
    echo "    [alive]   ${_alive:-(未配置)}"
    echo "    [yiban]   targets=${_yiban_targets}条  api=${_yiban_api:-(未配置)}"
else
    dim "    logic.toml 不存在"
fi
echo ""

# ── 日志目录 ──────────────────────────────────────────────────────────────────

if [[ -n "${_ldir:-}" ]]; then
    _log_path="$_ldir"
    [[ "$_log_path" != /* ]] && _log_path="$LIANBOT_DIR/$_log_path"
    if [[ -d "$_log_path" ]]; then
        echo "  ${C_BOLD}日志文件${C_NC}  $_log_path"
        ls -lht "$_log_path" 2>/dev/null | head -6 | sed 's/^/    /'
        _log_size=$(du -sh "$_log_path" 2>/dev/null | cut -f1)
        echo "    日志总计: ${_log_size:-未知}"
        echo ""
    fi
fi

# ── 二进制信息 ────────────────────────────────────────────────────────────────

if [[ -f "$LIANBOT_DIR/lianbot" ]]; then
    echo "  ${C_BOLD}二进制${C_NC}"
    ls -lh "$LIANBOT_DIR/lianbot" | awk '{print "    " $5 "  " $6 " " $7 " " $8}'
fi
echo ""
