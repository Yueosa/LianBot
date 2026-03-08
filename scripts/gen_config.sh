#!/usr/bin/env bash
# gen_config.sh — 生成 config.toml（kernel 层：host / port）

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
cd "$PROJECT_ROOT"

clear
title "  LianBot v${BOT_VERSION}  —  生成 config.toml（kernel 层）"
sep
echo ""
[[ -f "config.toml" ]] && info "检测到已有 config.toml，回车保留原值。" && echo ""

local_cfg="config.toml"

ask HOST "监听地址" "$(toml_val "$local_cfg" host '0.0.0.0')"
ask PORT "监听端口" "$(toml_val "$local_cfg" port '8080')"

CONTENT="# LianBot 内核配置 (kernel 层)
host = \"$HOST\"
port = $PORT"

echo ""; sep; echo ""; echo "$CONTENT"; echo ""; sep; echo ""
[[ -f "$local_cfg" ]] && warn "config.toml 已存在，将被覆盖。"
read -rp "  确认写入 config.toml？(y/N): " confirm
if [[ "${confirm,,}" == "y" ]]; then
    echo "$CONTENT" > "$local_cfg"
    info "config.toml 已生成"
else
    info "已取消"
fi
