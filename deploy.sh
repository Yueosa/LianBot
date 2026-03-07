#!/usr/bin/env bash
# deploy.sh — LianBot 服务器一键部署 / 更新 / 卸载脚本
#
# 使用方式（在项目根目录由 root 运行）：
#   sudo bash deploy.sh              首次安装 / 更新
#   sudo bash deploy.sh --uninstall  卸载服务
#
# 脚本会自动检测是否已有安装：
#   - 首次安装：创建系统用户、工作目录、复制三层配置文件、写入 systemd 服务并启动
#   - 更新模式：重新编译、替换二进制、同步配置、重启服务
#   - 旧版迁移：自动检测旧版 config.toml（含 [napcat]），拆分为三层配置
#
# 推荐工作流：
#   1. git clone / git pull
#   2. bash setup.sh        # 本地配置向导（无需 root）
#   3. sudo bash deploy.sh  # 安装或更新
#
# 三层配置架构（v0.2.0+）：
#   config.toml   — kernel 层（host / port）
#   runtime.toml  — 运行时基础设施（napcat / bot / pool / log / parser）
#   logic.toml    — 业务逻辑插件（smy / github / alive）
#
# 环境变量（可选）：
#   LIANBOT_USER   运行 lianbot 的系统用户，默认 lianbot
#   LIANBOT_DIR    工作目录，默认 /opt/lianbot
#   SKIP_CONFIG    设为 1 则跳过配置检查，从 example 复制（首次安装）

set -euo pipefail

# ── 颜色输出 ──────────────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; NC='\033[0m'
info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERR]${NC}   $*" >&2; exit 1; }

# ── 解析参数 ──────────────────────────────────────────────────────────────────

ARG_UNINSTALL=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) ARG_UNINSTALL=1 ;;
        *) ;;
    esac
done

# ── 权限检查 ──────────────────────────────────────────────────────────────────

[[ $EUID -eq 0 ]] || error "请使用 sudo 运行此脚本: sudo bash deploy.sh"

# ── 注入 cargo PATH（sudo 会丢弃用户的 ~/.cargo/bin）────────────────────────────

if ! command -v cargo &>/dev/null; then
    _cargo_candidates=(
        "/root/.cargo/bin"
        "${SUDO_USER:+$(getent passwd "$SUDO_USER" | cut -d: -f6)/.cargo/bin}"
        "/usr/local/cargo/bin"
    )
    for _p in "${_cargo_candidates[@]}"; do
        [[ -n "$_p" && -x "$_p/cargo" ]] && { export PATH="$_p:$PATH"; break; }
    done
fi

# ── 变量 ──────────────────────────────────────────────────────────────────────

LIANBOT_USER="${LIANBOT_USER:-lianbot}"
LIANBOT_DIR="${LIANBOT_DIR:-/opt/lianbot}"
SKIP_CONFIG="${SKIP_CONFIG:-0}"
SERVICE_FILE="/etc/systemd/system/lianbot.service"
BINARY_SRC="$(pwd)/target/release/LianBot"
BINARY_DST="$LIANBOT_DIR/lianbot"

BOT_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

[[ -f "Cargo.toml" ]] || error "请在 LianBot 项目根目录运行此脚本"

echo ""
echo -e "${BOLD}  LianBot v${BOT_VERSION}  —  部署脚本${NC}"
echo ""

# ── 三层配置文件列表 ──────────────────────────────────────────────────────────

CONFIG_FILES=("config.toml" "runtime.toml" "logic.toml")
EXAMPLE_FILES=("config.example.toml" "runtime.example.toml" "logic.example.toml")

# ── 卸载函数 ──────────────────────────────────────────────────────────────────

uninstall_lianbot() {
    echo ""
    echo -e "${YELLOW}  警告：即将卸载 LianBot 服务${NC}"
    echo ""
    read -rp "  确认卸载？这将停止并删除服务及二进制文件 (y/N): " c1
    [[ "${c1,,}" == "y" ]] || { info "已取消卸载"; exit 0; }

    info "停止并禁用 lianbot 服务..."
    systemctl stop   lianbot 2>/dev/null || true
    systemctl disable lianbot 2>/dev/null || true

    info "删除 systemd 服务文件..."
    rm -f "$SERVICE_FILE"
    systemctl daemon-reload

    info "删除二进制文件 $BINARY_DST ..."
    rm -f "$BINARY_DST"

    echo ""
    echo -e "${YELLOW}  工作目录 $LIANBOT_DIR 包含配置文件和可能的数据库文件。${NC}"
    read -rp "  是否同时删除整个工作目录（含 config/db 等数据）？(y/N): " c2
    if [[ "${c2,,}" == "y" ]]; then
        echo ""
        echo -e "${RED}  二次确认：此操作不可恢复，确认删除 $LIANBOT_DIR ？(yes/N): ${NC}"
        read -rp "  > " c3
        if [[ "$c3" == "yes" ]]; then
            rm -rf "$LIANBOT_DIR"
            info "工作目录已删除"
        else
            info "已取消删除工作目录"
        fi
    else
        info "保留工作目录 $LIANBOT_DIR"
    fi

    echo ""
    echo -e "${GREEN}  LianBot 已卸载。${NC}"
    echo ""
    exit 0
}

[[ $ARG_UNINSTALL -eq 1 ]] && uninstall_lianbot

# ── 自动检测模式 ───────────────────────────────────────────────────────────────

if [[ -f "$BINARY_DST" || -f "$SERVICE_FILE" ]]; then
    MODE="update"
else
    MODE="install"
fi
info "检测到运行模式：$MODE"

# ── 依赖检查 ──────────────────────────────────────────────────────────────────

info "检查依赖..."
command -v cargo     &>/dev/null || error "未找到 cargo，请先安装 Rust: https://rustup.rs"
command -v systemctl &>/dev/null || error "未找到 systemctl，此脚本仅支持 systemd 系统"

# ── 编译 ──────────────────────────────────────────────────────────────────────

# v0.2.0+ 所有模块 default 启用（含 core-db），直接 --all-features 编译
info "编译 release 二进制... (--all-features)"
cargo build --release --all-features
info "编译完成: $BINARY_SRC"

# ── 同步三层配置文件到部署目录 ────────────────────────────────────────────────

sync_configs() {
    local dir="$1"
    for f in "${CONFIG_FILES[@]}"; do
        if [[ -f "$f" ]]; then
            info "同步 $f → $dir/$f ..."
            cp "$f" "$dir/$f"
            chmod 640 "$dir/$f"
            chown "$LIANBOT_USER:$LIANBOT_USER" "$dir/$f"
        else
            warn "项目根目录无 $f，跳过同步（保留服务器端配置）"
        fi
    done
}

# ── 从 runtime.toml 读值（用于权限修复等） ────────────────────────────────────

rt_val() {
    local key="$1" fallback="$2" file=""
    if [[ -f "$LIANBOT_DIR/runtime.toml" ]]; then
        file="$LIANBOT_DIR/runtime.toml"
    elif [[ -f "runtime.toml" ]]; then
        file="runtime.toml"
    fi
    [[ -z "$file" ]] && { echo "$fallback"; return; }
    local val
    val=$(grep -E "^\s*${key}\s*=" "$file" | head -1 \
          | sed 's/[^=]*=[ \t]*//' | sed 's/^"//' | sed 's/"$//' | sed 's/^[ \t]*//' || true)
    echo "${val:-$fallback}"
}

# ── DB 权限修复 ───────────────────────────────────────────────────────────────

fix_db_perms() {
    local dir="$1"
    local db_path
    db_path=$(rt_val "db_path" "permissions.db")
    [[ "$db_path" != /* ]] && db_path="$dir/$db_path"
    info "创建权限 DB 文件（若不存在）：$db_path"
    touch "$db_path"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$db_path"
    chmod 660 "$db_path"
}

# ── 日志目录权限 ──────────────────────────────────────────────────────────────

fix_log_dir_perms() {
    local dir="$1"
    local log_dir
    log_dir=$(rt_val "log_dir" "")
    [[ -z "$log_dir" ]] && return
    [[ "$log_dir" != /* ]] && log_dir="$dir/$log_dir"
    info "创建日志目录：$log_dir"
    mkdir -p "$log_dir"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$log_dir"
    chmod 750 "$log_dir"
}

# ══════════════════════════════════════════════════════════════════════════════
#  旧版迁移（v0.1.x → v0.2.0 三层配置）
# ══════════════════════════════════════════════════════════════════════════════

detect_legacy_config() {
    # 旧版标志：工作目录有 config.toml 且含 [napcat] 段，但没有 runtime.toml
    if [[ -f "$LIANBOT_DIR/config.toml" && ! -f "$LIANBOT_DIR/runtime.toml" ]]; then
        grep -q '\[napcat\]' "$LIANBOT_DIR/config.toml" 2>/dev/null && return 0
    fi
    return 1
}

migrate_legacy_config() {
    local dir="$1"
    local old_cfg="$dir/config.toml"
    local old_plg="$dir/plugins.toml"

    info "检测到旧版 (v0.1.x) 配置，执行自动迁移..."
    echo ""

    # ── 备份 ──────────────────────────────────────────────────────────────────
    info "备份旧文件..."
    cp "$old_cfg" "$dir/config.toml.v1.bak"
    [[ -f "$old_plg" ]] && cp "$old_plg" "$dir/plugins.toml.v1.bak"

    # ── awk 提取函数 ──────────────────────────────────────────────────────────
    _val() {
        awk -v sec="[$1]" -v k="$2" '
            $0 == sec       { in_sec=1; next }
            /^\[/           { in_sec=0 }
            in_sec && $1==k {
                sub(/[^=]+=[ \t]*/, "")
                gsub(/["'"'"']/, "")
                sub(/[ \t]+$/, "")
                print; exit
            }
        ' "$old_cfg"
    }

    _arr() {
        awk -v sec="[$1]" -v k="$2" '
            $0 == sec       { in_sec=1; next }
            /^\[/           { in_sec=0 }
            in_sec && $1==k {
                if (match($0, /\[([^\]]*)\]/, a)) {
                    print a[1]
                }
                exit
            }
        ' "$old_cfg"
    }

    # ── 提取旧值 ──────────────────────────────────────────────────────────────
    local _host _port _url _token _wl _uw _ub _cap _evict _log_dir _log_level _log_max

    _host=$(_val "server" "host");       _host="${_host:-0.0.0.0}"
    _port=$(_val "server" "port");       _port="${_port:-8080}"
    _url=$(_val  "napcat" "url");        _url="${_url:-http://127.0.0.1:3000}"
    _token=$(_val "napcat" "token");     _token="${_token:-}"
    _wl=$(_arr "bot" "whitelist");       _wl="${_wl:-}"
    _uw=$(_arr "bot" "user_whitelist")
    _ub=$(_arr "bot" "user_blacklist")
    _cap=$(_val "pool" "per_group_capacity");  _cap="${_cap:-3000}"
    _evict=$(_val "pool" "evict_after_secs");  _evict="${_evict:-86400}"
    _log_dir=$(_val "log" "log_dir")
    _log_level=$(_val "log" "level");    _log_level="${_log_level:-info}"
    _log_max=$(_val "log" "max_days");   _log_max="${_log_max:-30}"

    # ── 询问 owner ────────────────────────────────────────────────────────────
    echo -e "  ${YELLOW}旧版无 owner 字段，请输入 Bot 主人 QQ 号（最高权限）：${NC}"
    read -rp "  owner QQ: " _owner
    [[ -z "$_owner" ]] && _owner=0

    # ── 生成 config.toml（kernel）──────────────────────────────────────────────
    cat > "$dir/config.toml" <<TOML
# LianBot 内核配置 (kernel) — 由 v0.1→v0.2 迁移自动生成
host = "$_host"
port = $_port
TOML
    info "  → config.toml  (host=$_host, port=$_port)"

    # ── 生成 runtime.toml ────────────────────────────────────────────────────
    local _bl_toml="[]"
    [[ -n "$_ub" ]] && _bl_toml="[$_ub]"

    local _log_block=""
    if [[ -n "$_log_dir" ]]; then
        _log_block="
[log]
log_dir  = \"$_log_dir\"
max_days = $_log_max
level    = \"$_log_level\""
    else
        _log_block="
[log]
level = \"$_log_level\""
    fi

    cat > "$dir/runtime.toml" <<TOML
# LianBot 运行时配置 (runtime) — 由 v0.1→v0.2 迁移自动生成

[bot]
owner          = $_owner
initial_groups = [$_wl]
blacklist      = $_bl_toml

[napcat]
url   = "$_url"
token = "$_token"

[pool]
per_group_capacity = $_cap
evict_after_secs   = $_evict
$_log_block
TOML
    info "  → runtime.toml (owner=$_owner, groups=[$_wl])"

    # ── 生成 logic.toml（从 plugins.toml 迁移）──────────────────────────────
    if [[ -f "$old_plg" ]]; then
        cp "$old_plg" "$dir/logic.toml"
        info "  → logic.toml   (从 plugins.toml 复制)"
    else
        touch "$dir/logic.toml"
        info "  → logic.toml   (空文件，使用默认值)"
    fi

    # ── 权限修复 ──────────────────────────────────────────────────────────────
    for f in config.toml runtime.toml logic.toml; do
        chmod 640 "$dir/$f"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dir/$f"
    done

    echo ""
    echo -e "${GREEN}  迁移完成！${NC}"
    echo ""
    echo "  旧文件已备份：config.toml.v1.bak / plugins.toml.v1.bak"
    echo ""
    echo -e "  ${YELLOW}迁移说明：${NC}"
    echo "  • sqlite_path / sqlite_retain_days 等字段已移除"
    echo "    权限 DB 路径改为 [bot] db_path（默认 permissions.db）"
    echo "  • 旧 lianbot.db 中的消息池/日报数据可安全保留"
    echo "  • .build_features 不再需要（v0.2.0+ 使用 --all-features）"
    echo ""
}

# ── 更新模式 ──────────────────────────────────────────────────────────────────

if [[ "$MODE" == "update" ]]; then
    if detect_legacy_config; then
        migrate_legacy_config "$LIANBOT_DIR"
        read -rp "  检查完毕后按 Enter 继续部署（Ctrl-C 中止）..." _
    fi

    info "停止服务（如已运行）..."
    systemctl stop lianbot 2>/dev/null || true

    info "替换二进制 $BINARY_DST ..."
    cp "$BINARY_SRC" "$BINARY_DST"
    chmod 755 "$BINARY_DST"

    sync_configs "$LIANBOT_DIR"
    fix_db_perms "$LIANBOT_DIR"
    fix_log_dir_perms "$LIANBOT_DIR"

    info "重载 systemd 并重启服务..."
    systemctl daemon-reload
    systemctl restart lianbot

    echo ""
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  LianBot v${BOT_VERSION} 更新完成！${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo "  服务状态:  systemctl status lianbot"
    echo "  实时日志:  journalctl -u lianbot -f"
    echo "  配置文件:  $LIANBOT_DIR/{config,runtime,logic}.toml"
    echo ""
    info "当前服务状态:"
    systemctl status lianbot --no-pager || true
    exit 0
fi

# ── 首次安装 ──────────────────────────────────────────────────────────────────

if [[ ! -f "config.toml" && ! -f "$LIANBOT_DIR/config.toml" ]]; then
    if [[ "$SKIP_CONFIG" == "1" ]]; then
        warn "SKIP_CONFIG=1，将从 example 文件复制，请事后手动编辑"
    else
        error "未找到 config.toml！\n\n  请先运行本地配置向导：bash setup.sh\n  然后再重新执行 sudo bash deploy.sh"
    fi
fi

# 创建系统用户
if ! id "$LIANBOT_USER" &>/dev/null; then
    info "创建系统用户 $LIANBOT_USER ..."
    useradd --system --no-create-home --shell /usr/sbin/nologin "$LIANBOT_USER"
else
    info "用户 $LIANBOT_USER 已存在，跳过"
fi

# 创建工作目录
info "创建工作目录 $LIANBOT_DIR ..."
mkdir -p "$LIANBOT_DIR"

# 安装二进制
info "安装二进制到 $BINARY_DST ..."
cp "$BINARY_SRC" "$BINARY_DST"
chmod 755 "$BINARY_DST"

# 复制配置文件
for i in "${!CONFIG_FILES[@]}"; do
    local_f="${CONFIG_FILES[$i]}"
    example_f="${EXAMPLE_FILES[$i]}"
    dst_f="$LIANBOT_DIR/$local_f"

    if [[ -f "$dst_f" ]]; then
        warn "$dst_f 已存在，跳过"
    elif [[ -f "$local_f" ]]; then
        info "复制 $local_f → $dst_f"
        cp "$local_f" "$dst_f"
        chmod 640 "$dst_f"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dst_f"
    elif [[ "$SKIP_CONFIG" == "1" && -f "$example_f" ]]; then
        info "从 $example_f 复制 → $dst_f"
        cp "$example_f" "$dst_f"
        chmod 640 "$dst_f"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dst_f"
    fi
done

fix_db_perms "$LIANBOT_DIR"
fix_log_dir_perms "$LIANBOT_DIR"
chown "$LIANBOT_USER:$LIANBOT_USER" "$LIANBOT_DIR"

# ── 创建 systemd 服务 ─────────────────────────────────────────────────────────

info "写入 systemd 服务文件 $SERVICE_FILE ..."
cat > "$SERVICE_FILE" <<SERVICE
[Unit]
Description=LianBot QQ Bot Service
Documentation=https://github.com/YeaSrine/LianBot
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=$LIANBOT_USER
Group=$LIANBOT_USER
WorkingDirectory=$LIANBOT_DIR
ExecStart=$BINARY_DST
Restart=on-failure
RestartSec=5s

Environment=RUST_LOG=info

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=$LIANBOT_DIR

[Install]
WantedBy=multi-user.target
SERVICE

# ── 启动服务 ──────────────────────────────────────────────────────────────────

info "重载 systemd 并启动服务..."
systemctl daemon-reload
systemctl enable --now lianbot

echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  LianBot v${BOT_VERSION} 部署完成！${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "  服务状态:  systemctl status lianbot"
echo "  实时日志:  journalctl -u lianbot -f"
echo "  重启服务:  systemctl restart lianbot"
echo "  停止服务:  systemctl stop lianbot"
echo "  卸载服务:  sudo bash deploy.sh --uninstall"
echo "  配置文件:  $LIANBOT_DIR/{config,runtime,logic}.toml"
echo ""
info "当前服务状态:"
systemctl status lianbot --no-pager || true
