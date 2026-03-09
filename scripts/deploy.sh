#!/usr/bin/env bash
# deploy.sh — LianBot 部署 / 更新 / 卸载
#
# 使用方式：
#   sudo bash scripts/deploy.sh              安装 / 更新
#   sudo bash scripts/deploy.sh --uninstall  卸载
#
# 前置条件：先运行 bash scripts/build.sh 编译

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
require_root
cd "$PROJECT_ROOT"

LIANBOT_USER="${LIANBOT_USER:-lianbot}"
SERVICE_FILE="/etc/systemd/system/lianbot.service"
BINARY_SRC="$PROJECT_ROOT/target/release/LianBot"
BINARY_DST="$LIANBOT_DIR/lianbot"
CONFIG_FILES=("config.toml" "runtime.toml" "logic.toml")

echo ""
title "  LianBot v${BOT_VERSION}  —  部署脚本"
echo ""

# ── 解析参数 ──────────────────────────────────────────────────────────────────

ARG_UNINSTALL=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) ARG_UNINSTALL=1 ;;
    esac
done

# ── 卸载 ──────────────────────────────────────────────────────────────────────

if [[ $ARG_UNINSTALL -eq 1 ]]; then
    echo -e "  ${C_YELLOW}警告：即将卸载 LianBot 服务${C_NC}"
    echo ""
    read -rp "  确认卸载？(y/N): " c1
    [[ "${c1,,}" == "y" ]] || { info "已取消"; exit 0; }

    info "停止并禁用服务..."
    systemctl stop lianbot 2>/dev/null || true
    systemctl disable lianbot 2>/dev/null || true

    info "删除 systemd 服务文件..."
    rm -f "$SERVICE_FILE"
    systemctl daemon-reload

    info "删除二进制 $BINARY_DST ..."
    rm -f "$BINARY_DST"

    echo ""
    echo -e "  ${C_YELLOW}工作目录 $LIANBOT_DIR 含配置和数据库文件。${C_NC}"
    read -rp "  是否删除整个工作目录？(y/N): " c2
    if [[ "${c2,,}" == "y" ]]; then
        echo -e "  ${C_RED}二次确认：此操作不可恢复，输入 yes 确认删除 $LIANBOT_DIR：${C_NC}"
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
    info "LianBot 已卸载。"
    exit 0
fi

# ── 检查二进制 ────────────────────────────────────────────────────────────────

if [[ ! -f "$BINARY_SRC" ]]; then
    error "未找到编译产物 $BINARY_SRC\n\n  请先运行: bash scripts/build.sh"
fi

# ── 自动检测模式 ──────────────────────────────────────────────────────────────

if [[ -f "$BINARY_DST" || -f "$SERVICE_FILE" ]]; then
    MODE="update"
else
    MODE="install"
fi
info "运行模式：$MODE"

# ── 工具函数 ──────────────────────────────────────────────────────────────────

sync_configs() {
    for f in "${CONFIG_FILES[@]}"; do
        if [[ -f "$f" ]]; then
            info "同步 $f → $LIANBOT_DIR/$f"
            cp "$f" "$LIANBOT_DIR/$f"
            chmod 640 "$LIANBOT_DIR/$f"
            chown "$LIANBOT_USER:$LIANBOT_USER" "$LIANBOT_DIR/$f"
        else
            warn "项目无 $f，跳过（保留服务器端配置）"
        fi
    done
}

fix_db_perms() {
    local db_path
    db_path=$(toml_section_val "$LIANBOT_DIR/runtime.toml" bot db_path "permissions.db")
    [[ "$db_path" != /* ]] && db_path="$LIANBOT_DIR/$db_path"
    info "确保权限 DB：$db_path"
    touch "$db_path"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$db_path"
    chmod 660 "$db_path"
}

fix_log_dir_perms() {
    local log_dir
    log_dir=$(toml_section_val "$LIANBOT_DIR/runtime.toml" log log_dir "")
    [[ -z "$log_dir" ]] && return
    [[ "$log_dir" != /* ]] && log_dir="$LIANBOT_DIR/$log_dir"
    info "确保日志目录：$log_dir"
    mkdir -p "$log_dir"
    chown "$LIANBOT_USER:$LIANBOT_USER" "$log_dir"
    chmod 750 "$log_dir"
}

# ── 更新模式 ──────────────────────────────────────────────────────────────────

if [[ "$MODE" == "update" ]]; then
    info "停止服务..."
    systemctl stop lianbot 2>/dev/null || true

    info "替换二进制 $BINARY_DST"
    cp "$BINARY_SRC" "$BINARY_DST"
    chmod 755 "$BINARY_DST"

    sync_configs
    fix_db_perms
    fix_log_dir_perms
    chown "$LIANBOT_USER:$LIANBOT_USER" "$LIANBOT_DIR"

    info "重启服务..."
    systemctl daemon-reload
    systemctl restart lianbot

    echo ""
    echo -e "${C_GREEN}━━━ LianBot v${BOT_VERSION} 更新完成 ━━━${C_NC}"
    echo ""
    echo "  服务状态:  systemctl status lianbot"
    echo "  实时日志:  journalctl -u lianbot -f"
    echo "  配置文件:  $LIANBOT_DIR/{config,runtime,logic}.toml"
    echo ""
    systemctl status lianbot --no-pager || true
    exit 0
fi

# ── 首次安装 ──────────────────────────────────────────────────────────────────

if [[ ! -f "config.toml" && ! -f "$LIANBOT_DIR/config.toml" ]]; then
    error "未找到 config.toml！\n\n  请先运行: bash setup.sh → 生成配置"
fi

# 创建系统用户
if ! id "$LIANBOT_USER" &>/dev/null; then
    info "创建系统用户 $LIANBOT_USER"
    useradd --system --no-create-home --shell /usr/sbin/nologin "$LIANBOT_USER"
else
    info "用户 $LIANBOT_USER 已存在"
fi

# 创建工作目录
mkdir -p "$LIANBOT_DIR"

# 安装二进制
info "安装二进制到 $BINARY_DST"
cp "$BINARY_SRC" "$BINARY_DST"
chmod 755 "$BINARY_DST"

# 复制配置
for f in "${CONFIG_FILES[@]}"; do
    dst="$LIANBOT_DIR/$f"
    if [[ -f "$dst" ]]; then
        warn "$dst 已存在，跳过"
    elif [[ -f "$f" ]]; then
        info "复制 $f → $dst"
        cp "$f" "$dst"
        chmod 640 "$dst"
        chown "$LIANBOT_USER:$LIANBOT_USER" "$dst"
    fi
done

fix_db_perms
fix_log_dir_perms
chown "$LIANBOT_USER:$LIANBOT_USER" "$LIANBOT_DIR"

# 清理旧 service 文件中可能存在的 RUST_LOG 硬编码
if [[ -f "$SERVICE_FILE" ]] && grep -q 'Environment=RUST_LOG' "$SERVICE_FILE"; then
    info "清理旧 service 中的 RUST_LOG 硬编码..."
fi

# 写 systemd 服务
info "写入 $SERVICE_FILE"
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

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=$LIANBOT_DIR

[Install]
WantedBy=multi-user.target
SERVICE

# 启动
info "启动服务..."
systemctl daemon-reload
systemctl enable --now lianbot

echo ""
echo -e "${C_GREEN}━━━ LianBot v${BOT_VERSION} 部署完成 ━━━${C_NC}"
echo ""
echo "  服务状态:  systemctl status lianbot"
echo "  实时日志:  journalctl -u lianbot -f"
echo "  重启服务:  systemctl restart lianbot"
echo "  停止服务:  systemctl stop lianbot"
echo "  卸载:      sudo bash scripts/deploy.sh --uninstall"
echo "  配置文件:  $LIANBOT_DIR/{config,runtime,logic}.toml"
echo ""
systemctl status lianbot --no-pager || true
