#!/usr/bin/env bash
# Phase 4c — Feature 组合编译验证脚本
# 用途：确保所有关键 feature 组合均可独立编译
# 运行：bash check_features.sh

set -euo pipefail

PASS=0
FAIL=0

run() {
    local label="$1"; shift
    printf "%-60s" "$label"
    if cargo check "$@" 2>/dev/null; then
        echo "✅ OK"
        PASS=$((PASS + 1))
    else
        echo "❌ FAIL"
        cargo check "$@" 2>&1 | grep "^error" | head -5
        FAIL=$((FAIL + 1))
    fi
}

run_test() {
    local label="$1"; shift
    printf "%-60s" "$label"
    if cargo test "$@" -- --test-threads=1 -q 2>/dev/null; then
        echo "✅ OK"
        PASS=$((PASS + 1))
    else
        echo "❌ FAIL"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== LianBot Feature 编译矩阵验证 ==="
echo ""

# ── 裁剪场景 ──────────────────────────────────────────────────────────────────
run  "no-default-features（裸核心）"          --no-default-features
run  "仅 cmd-ping"                             --no-default-features --features cmd-ping
run  "仅 cmd-smy（拉入 chrono + base64）"     --no-default-features --features cmd-smy
run  "cmd-stalk（自动拉入 core-ws）"          --no-default-features --features cmd-stalk

# ── 标准场景 ──────────────────────────────────────────────────────────────────
run  "default（全命令集）"                    
run  "default + SQLite"                        --features core-pool-sqlite
run  "default + 文件日志"                      --features core-log-file
run  "default + SQLite + 文件日志"              --features core-pool-sqlite,core-log-file
run  "all-features"                            --all-features

# ── 测试场景 ──────────────────────────────────────────────────────────────────
echo ""
run_test "cargo test（默认 feature）"
run_test "cargo test --features core-pool-sqlite" --features core-pool-sqlite

echo ""
echo "=== 结果: ${PASS} 通过 / ${FAIL} 失败 ==="
[ "$FAIL" -eq 0 ]
