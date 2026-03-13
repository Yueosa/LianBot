#!/usr/bin/env bash
# build.sh — 编译 LianBot
# 当前：--all-features 全量编译
# 预留：交互式 feature 选择（Phase 3）

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_project_root
cd "$PROJECT_ROOT"

# ── Feature 定义 ─────────────────────────────────────────────────────────────
# TODO: Phase 3 — 可选编译
#
# 当模块间依赖关系稳定后，可启用交互式 feature 选择菜单。
# 架构设计：
#
# FEATURES=(
#   "cmd-ping:!!ping 命令:true"
#   "cmd-help:!!help 命令:true"
#   "cmd-alive:!!alive 设备探测:true"
#   "cmd-smy:/smy 群聊日报（拉入 base64 + tempfile）:true"
#   "cmd-acg:!!acg 随机图片:true"
#   "cmd-stalk:!!stalk 在线监控（自动拉入 core-ws）:true"
#   "cmd-world:!!world 60秒看世界:true"
#   "cmd-dress:!!dress 随机女装图片（拉入 rand + regex + urlencoding + base64）:true"
#   "cmd-sign:!!sign 触发易班签到（自动拉入 svc-yiban）:true"
#   "svc-github:GitHub Webhook 通知（拉入 hmac + sha2 + hex）:true"
#   "svc-yiban:易班签到 Webhook 通知（拉入 hmac + sha2 + hex）:true"
#   "core-db:SQLite 权限数据库（拉入 rusqlite）:true"
#   "core-log-file:滚动日志文件（拉入 tracing-appender）:false"
# )
#
# 交互流程：
#   1. 列出所有 feature，标注默认开/关和依赖关系
#   2. 用户输入数字切换选中状态
#   3. 自动解析依赖（cmd-stalk → core-ws）
#   4. 生成 --features "cmd-ping,cmd-help,..." 参数
#   5. 将选择保存到 .build_features 供 deploy.sh 读取
#
# 依赖关系：
#   cmd-stalk  → core-ws
#   cmd-smy    → dep:base64, dep:tempfile
#   cmd-dress  → dep:rand, dep:regex, dep:urlencoding, dep:base64
#   cmd-sign   → svc-yiban
#   svc-github → core-webhook
#   svc-yiban  → core-webhook
#   core-webhook → dep:hmac, dep:sha2, dep:hex
#   core-db    → dep:rusqlite
#   core-log-file → dep:tracing-appender

# ── 编译 ──────────────────────────────────────────────────────────────────────

command -v cargo &>/dev/null || error "未找到 cargo，请先安装 Rust: https://rustup.rs"

info "编译模式: --release --all-features"
echo ""

cargo build --release --all-features

echo ""
info "编译完成: target/release/LianBot"
ls -lh "$PROJECT_ROOT/target/release/LianBot" 2>/dev/null || true
