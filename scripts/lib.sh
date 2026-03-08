#!/usr/bin/env bash
# lib.sh — LianBot 脚本公共库
# 用法：source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

set -euo pipefail

# ── 路径 ──────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LIANBOT_DIR="${LIANBOT_DIR:-/opt/lianbot}"

# ── 版本 ──────────────────────────────────────────────────────────────────────

BOT_VERSION=$(grep '^version' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

# ── 颜色 ──────────────────────────────────────────────────────────────────────

C_RED=$'\033[0;31m'; C_GREEN=$'\033[0;32m'; C_YELLOW=$'\033[1;33m'
C_CYAN=$'\033[0;36m'; C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'; C_NC=$'\033[0m'

info()  { echo -e "${C_GREEN}[INFO]${C_NC}  $*"; }
warn()  { echo -e "${C_YELLOW}[WARN]${C_NC}  $*"; }
error() { echo -e "${C_RED}[ERR]${C_NC}   $*" >&2; exit 1; }
title() { echo -e "${C_BOLD}${C_CYAN}$*${C_NC}"; }
dim()   { echo -e "${C_DIM}$*${C_NC}"; }

sep() { echo "  ──────────────────────────────────────────────────"; }

# ── TOML 读取 ─────────────────────────────────────────────────────────────────

# toml_val <file> <key> <fallback>
#   读取文件顶层标量值
toml_val() {
    local file="$1" key="$2" fallback="$3"
    if [[ ! -f "$file" ]]; then echo "$fallback"; return; fi
    local val
    val=$(grep -E "^\s*${key}\s*=" "$file" | head -1 \
          | sed 's/[^=]*=[ \t]*//' | sed 's/^"//' | sed 's/"$//' | sed 's/^[ \t]*//' || true)
    echo "${val:-$fallback}"
}

# toml_section_val <file> <section> <key> <fallback>
#   读取 [section] 下的标量值
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

# toml_section_arr <file> <section> <key>
#   读取 [section] 下的数组值（返回逗号分隔字符串）
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

# ── 交互工具 ──────────────────────────────────────────────────────────────────

# ask <var> <prompt> <default>
ask() {
    local var="$1" prompt="$2" default="$3"
    local val
    read -rp "  ${prompt} [${default}]: " val
    val="${val:-$default}"
    printf -v "$var" '%s' "$val"
}

# ask_optional <var> <prompt> <note>
ask_optional() {
    local var="$1" prompt="$2" note="$3"
    local val
    read -rp "  ${prompt}  (${note}，留空跳过): " val
    printf -v "$var" '%s' "$val"
}

# ── 前置检查 ──────────────────────────────────────────────────────────────────

require_project_root() {
    [[ -f "$PROJECT_ROOT/Cargo.toml" ]] || error "请在 LianBot 项目根目录运行此脚本"
}

require_root() {
    [[ $EUID -eq 0 ]] || error "请使用 sudo 运行: sudo bash $0"
}
