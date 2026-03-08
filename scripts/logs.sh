#!/usr/bin/env bash
# logs.sh — 实时查看 LianBot 日志

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

# 尝试从运行时配置读 log_dir
_rt="$LIANBOT_DIR/runtime.toml"
[[ ! -f "$_rt" ]] && _rt="$PROJECT_ROOT/runtime.toml"

log_dir=""
[[ -f "$_rt" ]] && log_dir=$(toml_section_val "$_rt" log log_dir "")

if [[ -n "$log_dir" ]]; then
    [[ "$log_dir" != /* ]] && log_dir="$LIANBOT_DIR/$log_dir"

    today=$(date -u +%Y-%m-%d)
    log_file="${log_dir}/lianbot.log.utc.${today}"

    if [[ ! -f "$log_file" ]]; then
        latest=$(ls -1t "${log_dir}"/lianbot.log.utc.* 2>/dev/null | head -1)
        if [[ -n "$latest" ]]; then
            warn "今日日志尚未生成，显示最近: $latest"
            echo ""
            log_file="$latest"
        else
            warn "日志目录 $log_dir 中暂无日志文件"
            exit 1
        fi
    fi

    info "实时跟踪: $log_file  (Ctrl-C 退出)"
    echo ""
    tail -f "$log_file"
else
    info "未配置 log_dir，使用 journald  (Ctrl-C 退出)"
    echo ""
    sudo journalctl -u lianbot -f
fi
