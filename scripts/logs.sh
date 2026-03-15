#!/usr/bin/env bash
# logs.sh — 实时查看 LianBot 日志

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

# Ctrl-C 只退出本脚本，不传播到父进程 (setup.sh)
trap 'exit 0' INT

# 尝试从运行时配置读 log_dir
_rt="$LIANBOT_DIR/runtime.toml"
[[ ! -f "$_rt" ]] && _rt="$PROJECT_ROOT/runtime.toml"

log_dir=""
[[ -f "$_rt" ]] && log_dir=$(toml_section_val "$_rt" log log_dir "")

find_latest_log() {
    local dir="$1"
    find "$dir" -maxdepth 1 -type f -name 'lianbot.log.20*' -printf '%T@ %p\n' 2>/dev/null \
        | sort -nr \
        | head -1 \
        | cut -d' ' -f2-
}

if [[ -n "$log_dir" ]]; then
    [[ "$log_dir" != /* ]] && log_dir="$LIANBOT_DIR/$log_dir"

    # 使用本地时间而不是 UTC
    today=$(date +%Y-%m-%d)
    log_file="${log_dir}/lianbot.log.${today}"

    if [[ ! -f "$log_file" ]]; then
        latest=$(find_latest_log "$log_dir")
        if [[ -n "$latest" ]]; then
            warn "今日日志尚未生成，显示最近: $latest"
            echo ""
            log_file="$latest"
        else
            warn "日志目录 $log_dir 中暂无日志文件，回退 journald"
            echo ""
            sudo journalctl -u lianbot -f --no-pager
            exit 0
        fi
    fi

    info "实时跟踪: $log_file  (Ctrl-C 退出)"
    _total=$(wc -l < "$log_file" 2>/dev/null || echo 0)
    dim "  文件总行数: ${_total}"
    echo ""

    # 初始展示最近 100 行
    _tail_preview=$(tail -n 100 "$log_file" 2>/dev/null || true)
    _boot_line=$(printf '%s\n' "$_tail_preview" | grep -n '配置加载成功' | tail -1 | cut -d: -f1 || true)
    if [[ -n "$_boot_line" ]]; then
        printf '%s\n' "$_tail_preview" | tail -n +"$_boot_line"
    else
        printf '%s\n' "$_tail_preview"
    fi
    echo ""

    if [[ -r "$log_file" ]]; then
        tail -n 0 -F "$log_file"
    else
        sudo tail -n 0 -F "$log_file"
    fi
else
    info "未配置 log_dir，使用 journald  (Ctrl-C 退出)"
    echo ""
    sudo journalctl -u lianbot -f --no-pager
fi
