#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# AxAgent 统一开发入口
#
# 用法：
#   ./scripts/dev.sh <subrepo> [args...]
#
# 示例：
#   ./scripts/dev.sh core              # cargo check（axagent-core）
#   ./scripts/dev.sh core test -p axagent-harness
#   ./scripts/dev.sh frontend          # npm run dev（axagent-frontend）
#   ./scripts/dev.sh frontend typecheck
#   ./scripts/dev.sh infra             # cargo check（axagent-infra）
#
# 子仓库命令映射：
#   core       → cargo check（axagent-core）
#   kit        → cargo check（axagent-kit）
#   infra      → cargo check（axagent-infra）
#   runtime    → cargo check（axagent-runtime）
#   ai         → cargo check（axagent-ai）
#   quant      → cargo check（axagent-quant）
#   plugins    → cargo check（axagent-plugins）
#   frontend   → npm run dev（axagent-frontend）
#   （axagent-mobile 随 src-tauri 应用壳层保留，不在此入口下）
#
# 依赖：
#   - 已执行 git submodule update --init --recursive
#   - 或已运行 ./scripts/setup-submodules.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SUBREPO="$1"
shift 2>/dev/null || true

case "$SUBREPO" in
  core|kit|infra|runtime|ai|quant|plugins)
    SUBREPO_DIR="$REPO_ROOT/axagent-$SUBREPO"
    CMD="$SUBREPO_DIR"
    ARGS=("$@")
    ;;
  frontend)
    SUBREPO_DIR="$REPO_ROOT/axagent-frontend"
    CMD="$SUBREPO_DIR"
    ARGS=("$@")
    ;;
  *)
    echo "用法: $0 <subrepo> [args...]"
    echo ""
    echo "子仓库: core, kit, infra, runtime, ai, quant, plugins, frontend"
    echo ""
    echo "示例:"
    echo "  $0 core                  # cargo check"
    echo "  $0 frontend              # npm run dev"
    echo "  $0 frontend typecheck    # npm run typecheck"
    exit 1
    ;;
esac

if [ ! -d "$SUBREPO_DIR" ]; then
  echo "✗ 子仓库不存在：$SUBREPO_DIR"
  echo "  请先运行：git submodule update --init --recursive"
  exit 1
fi

if [ "$SUBREPO" = "frontend" ]; then
  cd "$CMD"
  npm run "$@" 2>/dev/null || {
    # 如果 npm run 失败，尝试直接执行
    case "$1" in
      dev) npm run dev ;;
      build) npm run build ;;
      typecheck) npm run typecheck ;;
      test:run) npm run test:run ;;
      *) npm run "$@" ;;
    esac
  }
else
  cd "$CMD"
  cmd="${1:-check}"
  shift 2>/dev/null || true
  cargo "$cmd" "$@"
fi
