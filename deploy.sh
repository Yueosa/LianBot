#!/usr/bin/env bash
# deploy.sh — LianBot 服务器一键部署 / 更新脚本
#
# 使用方式（git clone 后在项目根目录运行）：
#   sudo bash deploy.sh
#
# 脚本会自动检测是否已有安装：
#   - 首次安装：创建用户、目录、配置文件、systemd 服务并启动
#   - 更新模式：重新编译、替换二进制、重启服务，保留已有配置
#
# 环境变量（可选，用于非交互式部署）：
#   LIANBOT_USER      运行 lianbot 的系统用户，默认 lianbot
#   LIANBOT_DIR       工作目录，默认 /opt/lianbot
#   SKIP_CONFIG       设为 1 则跳过配置文件交互（仅首次安装有效），默认 0

set -euo pipefail

# ── 颜色输出 ──────────────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERR]${NC}   $*" >&2; exit 1; }

# ── 权限检查 ──────────────────────────────────────────────────────────────────

[[ $EUID -eq 0 ]] || error "请使用 sudo 运行此脚本: sudo bash deploy.sh"

# ── 变量 ──────────────────────────────────────────────────────────────────────

LIANBOT_USER="${LIANBOT_USER:-lianbot}"
LIANBOT_DIR="${LIANBOT_DIR:-/opt/lianbot}"
SKIP_CONFIG="${SKIP_CONFIG:-0}"
SERVICE_FILE="/etc/systemd/system/lianbot.service"
BINARY_SRC="$(pwd)/target/release/LianBot"
BINARY_DST="$LIANBOT_DIR/lianbot"

# 脚本必须在项目根目录运行
[[ -f "Cargo.toml" ]] || error "请在 LianBot 项目根目录运行此脚本"

# ── 自动检测模式 ───────────────────────────────────────────────────────────────

if [[ -f "$BINARY_DST" || -f "$SERVICE_FILE" ]]; then
    MODE="update"
else
    MODE="install"
fi
info "检测到运行模式：$MODE"

# ── 依赖检查 ──────────────────────────────────────────────────────────────────

info "检查依赖..."
command -v cargo   &>/dev/null || error "未找到 cargo，请先安装 Rust: https://rustup.rs"
command -v systemctl &>/dev/null || error "未找到 systemctl，此脚本仅支持 systemd 系统"

# ── 编译 ──────────────────────────────────────────────────────────────────────

info "编译 release 二进制..."
cargo build --release
info "编译完成: $BINARY_SRC"

# ── 更新模式：替换二进制并重启服务后直接退出 ──────────────────────────────────

if [[ "$MODE" == "update" ]]; then
    info "停止服务（如已运行）..."
    systemctl stop lianbot 2>/dev/null || true

    info "替换二进制 $BINARY_DST ..."
    cp "$BINARY_SRC" "$BINARY_DST"
    chmod 755 "$BINARY_DST"

    info "重载 systemd 并重启服务..."
    systemctl daemon-reload
    systemctl restart lianbot

    echo ""
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  LianBot 更新完成！${NC}"
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

# ── 创建系统用户 ──────────────────────────────────────────────────────────────

if ! id "$LIANBOT_USER" &>/dev/null; then
    info "创建系统用户 $LIANBOT_USER ..."
    useradd --system --no-create-home --shell /usr/sbin/nologin "$LIANBOT_USER"
else
    info "用户 $LIANBOT_USER 已存在，跳过"
fi

# ── 创建工作目录 ──────────────────────────────────────────────────────────────

info "创建工作目录 $LIANBOT_DIR ..."
mkdir -p "$LIANBOT_DIR"

# ── 复制二进制 ────────────────────────────────────────────────────────────────

info "安装二进制到 $BINARY_DST ..."
cp "$BINARY_SRC" "$BINARY_DST"
chmod 755 "$BINARY_DST"

# ── 配置文件 ──────────────────────────────────────────────────────────────────

CONFIG_DST="$LIANBOT_DIR/config.toml"

if [[ -f "$CONFIG_DST" ]]; then
    warn "配置文件 $CONFIG_DST 已存在，跳过（如需重置请手动删除后重新运行）"
elif [[ "$SKIP_CONFIG" == "1" ]]; then
    info "SKIP_CONFIG=1，从示例文件复制配置，请事后手动编辑 $CONFIG_DST"
    cp config.example.toml "$CONFIG_DST"
else
    info "开始交互式配置（直接回车使用括号内的默认值）..."
    echo ""

    read -rp "  NapCat HTTP URL       [http://127.0.0.1:3000]: " NAPCAT_URL
    NAPCAT_URL="${NAPCAT_URL:-http://127.0.0.1:3000}"

    read -rp "  NapCat Bearer Token   [留空则不使用]: " NAPCAT_TOKEN

    read -rp "  Bot 监听端口          [8080]: " BOT_PORT
    BOT_PORT="${BOT_PORT:-8080}"

    read -rp "  群白名单（逗号分隔群号，例如 123456,789012）: " WHITELIST_RAW
    # 将逗号分隔转为 TOML 数组格式  123456,789012 → [123456, 789012]
    WHITELIST_TOML="[$(echo "$WHITELIST_RAW" | tr ',' ' ' | xargs | tr ' ' ', ')]"
    [[ "$WHITELIST_TOML" == "[]" ]] && warn "白名单为空，Bot 将不响应任何群消息！"

    echo ""
    info "写入配置文件 $CONFIG_DST ..."
    cat > "$CONFIG_DST" <<TOML
[server]
host = "0.0.0.0"
port = $BOT_PORT

[napcat]
url   = "$NAPCAT_URL"
token = "$NAPCAT_TOKEN"

[bot]
whitelist = $WHITELIST_TOML
TOML
fi

chmod 640 "$CONFIG_DST"
chown "$LIANBOT_USER:$LIANBOT_USER" "$CONFIG_DST"
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
echo -e "${GREEN}  LianBot 部署完成！${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "  服务状态:  systemctl status lianbot"
echo "  实时日志:  journalctl -u lianbot -f"
echo "  重启服务:  systemctl restart lianbot"
echo "  停止服务:  systemctl stop lianbot"
echo "  配置文件:  $CONFIG_DST"
echo ""
info "当前服务状态:"
systemctl status lianbot --no-pager || true
