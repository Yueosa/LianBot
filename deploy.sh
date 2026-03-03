#!/usr/bin/env bash
# deploy.sh — LianBot 服务器一键部署 / 更新 / 卸载脚本
#
# 使用方式（在项目根目录由 root 运行）：
#   sudo bash deploy.sh              首次安装 / 更新
#   sudo bash deploy.sh --uninstall  卸载服务
#
# 脚本会自动检测是否已有安装：
#   - 首次安装：创建系统用户、工作目录、复制配置文件、写入 systemd 服务并启动
#   - 更新模式：重新编译（读取 .build_features）、替换二进制、同步配置、重启服务
#
# 推荐工作流：
#   1. git clone / git pull
#   2. bash setup.sh        # 本地配置向导（无需 root），生成 config.toml / .build_features
#   3. sudo bash deploy.sh  # 安装或更新
#
# 环境变量（可选）：
#   LIANBOT_USER   运行 lianbot 的系统用户，默认 lianbot
#   LIANBOT_DIR    工作目录，默认 /opt/lianbot
#   SKIP_CONFIG    设为 1 则跳过配置检查，直接从 config.example.toml 复制（首次安装）

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

# 优先使用调用方通过 SUDO_USER 找到的 cargo
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

# 脚本必须在项目根目录运行
[[ -f "Cargo.toml" ]] || error "请在 LianBot 项目根目录运行此脚本"

echo ""
echo -e "${BOLD}  LianBot v${BOT_VERSION}  —  部署脚本${NC}"
echo ""

# ── 读取编译特性 ──────────────────────────────────────────────────────────────

read_build_features() {
    # 优先读取服务器端 .build_features，其次本地，最后为空（使用 Cargo.toml 默认值）
    if [[ -f "$LIANBOT_DIR/.build_features" ]]; then
        CARGO_FEATURE_ARGS=$(grep -v '^#' "$LIANBOT_DIR/.build_features" | tr '\n' ',' | sed 's/,$//' | sed 's/^/--features /')
        info "使用服务器端编译配置: $CARGO_FEATURE_ARGS"
    elif [[ -f ".build_features" ]]; then
        CARGO_FEATURE_ARGS=$(grep -v '^#' ".build_features" | tr '\n' ',' | sed 's/,$//' | sed 's/^/--features /')
        info "使用本地编译配置: $CARGO_FEATURE_ARGS"
    else
        CARGO_FEATURE_ARGS=""
        info "未找到 .build_features，使用 Cargo.toml 默认 features"
    fi
}

# ── 获取已选 feature 列表（用于权限处理等） ───────────────────────────────────

has_feature() {
    local feat="$1"
    # 检查服务器端或本地 .build_features 中是否有该 feature
    if [[ -f "$LIANBOT_DIR/.build_features" ]]; then
        grep -q "^${feat}$" "$LIANBOT_DIR/.build_features" && return 0
    elif [[ -f ".build_features" ]]; then
        grep -q "^${feat}$" ".build_features" && return 0
    fi
    return 1
}

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
    read -rp "  是否同时删除整个工作目录（含 config.toml、SQLite 等数据）？(y/N): " c2
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

# ── 如有 --uninstall 则执行卸载 ───────────────────────────────────────────────

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

# ── 读取编译 features ─────────────────────────────────────────────────────────

read_build_features

# ── 编译 ──────────────────────────────────────────────────────────────────────

info "编译 release 二进制... (${CARGO_FEATURE_ARGS:-默认 features})"
# shellcheck disable=SC2086
cargo build --release $CARGO_FEATURE_ARGS
info "编译完成: $BINARY_SRC"

# ── 同步配置文件到部署目录（通用工具函数） ────────────────────────────────────

sync_configs() {
    local dir="$1"
    if [[ -f "config.toml" ]]; then
        info "同步 config.toml → $dir/config.toml ..."
        cp "config.toml" "$dir/config.toml"
        chmod 640 "$dir/config.toml"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dir/config.toml"
    else
        warn "项目根目录无 config.toml，跳过同步（保留旧配置）"
    fi

    if [[ -f "plugins.toml" ]]; then
        info "同步 plugins.toml → $dir/plugins.toml ..."
        cp "plugins.toml" "$dir/plugins.toml"
        chmod 640 "$dir/plugins.toml"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dir/plugins.toml"
    fi

    if [[ -f ".build_features" ]]; then
        info "同步 .build_features → $dir/.build_features ..."
        cp ".build_features" "$dir/.build_features"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dir/.build_features"
    fi
}

# ── 设置 SQLite 数据库权限（若已启用） ───────────────────────────────────────

fix_sqlite_perms() {
    local dir="$1"
    local sqlite_path
    # 从 config.toml 中提取 sqlite_path（若存在）
    if [[ -f "$dir/config.toml" ]]; then
        sqlite_path=$(grep 'sqlite_path' "$dir/config.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/' || true)
    fi
    sqlite_path="${sqlite_path:-lianbot.db}"
    # 若路径不是绝对路径，拼接工作目录
    [[ "$sqlite_path" != /* ]] && sqlite_path="$dir/$sqlite_path"

    if has_feature "core-pool-sqlite"; then
        info "创建 SQLite 数据库文件（若不存在）并设置权限..."
        touch "$sqlite_path"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$sqlite_path"
        chmod 660 "$sqlite_path"
    fi
}

# ── 创建并授权日志目录（若已启用 core-log-file） ─────────────────────────────

fix_log_dir_perms() {
    local dir="$1"
    if ! has_feature "core-log-file"; then return; fi
    local log_dir
    if [[ -f "$dir/config.toml" ]]; then
        log_dir=$(grep -E '^\s*log_dir\s*=' "$dir/config.toml" | head -1 \
                  | sed 's/.*=\s*"\(.*\)".*/\1/' || true)
    fi
    [[ -z "$log_dir" ]] && return          # 未配置 log_dir，无需处理
    [[ "$log_dir" != /* ]] && log_dir="$dir/$log_dir"  # 相对路径转绝对
    info "创建日志目录并设置权限：$log_dir"
    mkdir -p "$log_dir"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$log_dir"
    chmod 750 "$log_dir"
}

# ── 更新模式 ──────────────────────────────────────────────────────────────────

if [[ "$MODE" == "update" ]]; then
    info "停止服务（如已运行）..."
    systemctl stop lianbot 2>/dev/null || true

    info "替换二进制 $BINARY_DST ..."
    cp "$BINARY_SRC" "$BINARY_DST"
    chmod 755 "$BINARY_DST"

    sync_configs "$LIANBOT_DIR"
    fix_sqlite_perms "$LIANBOT_DIR"
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
    echo "  配置文件:  $LIANBOT_DIR/config.toml"
    echo ""
    info "当前服务状态:"
    systemctl status lianbot --no-pager || true
    exit 0
fi

# ── 首次安装 ──────────────────────────────────────────────────────────────────

# 检查 config.toml 是否就绪
CONFIG_DST="$LIANBOT_DIR/config.toml"
if [[ ! -f "config.toml" && ! -f "$CONFIG_DST" ]]; then
    if [[ "$SKIP_CONFIG" == "1" ]]; then
        warn "SKIP_CONFIG=1，将从 config.example.toml 复制，请事后手动编辑 $CONFIG_DST"
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
if [[ -f "$CONFIG_DST" ]]; then
    warn "配置文件 $CONFIG_DST 已存在，跳过（如需重置请手动删除后重新运行）"
elif [[ "$SKIP_CONFIG" == "1" ]]; then
    info "从示例文件复制配置..."
    cp config.example.toml "$CONFIG_DST"
    chmod 640 "$CONFIG_DST"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$CONFIG_DST"
else
    sync_configs "$LIANBOT_DIR"
fi

fix_sqlite_perms "$LIANBOT_DIR"
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

# 日志级别（可选：trace/debug/info/warn/error）
Environment=RUST_LOG=info

# 安全加固
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

# ── 完成 ──────────────────────────────────────────────────────────────────────

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
echo "  配置文件:  $CONFIG_DST"
echo ""
info "当前服务状态:"
systemctl status lianbot --no-pager || true
