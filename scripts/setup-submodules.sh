#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# AxAgent 子仓库一键初始化
#
# 用法：
#   ./scripts/setup-submodules.sh
#
# 功能：
#   1. git submodule update --init --recursive
#   2. 验证每个子仓库已正确 checkout
#   3. 输出后续操作指引

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

echo "▶ 初始化 AxAgent 子仓库..."
git submodule update --init --recursive

echo ""
echo "✓ 子仓库初始化完成"
echo ""
echo "已初始化的子仓库："
git submodule status | head -20

echo ""
echo "下一步："
echo "  1. 进入子仓库开发：./scripts/dev.sh <subrepo>"
echo "  2. 查看 CI 配置：.github/workflows/ci-split.yml"
echo "  3. 阅读拆分方案：docs/plans/PLAN-repo-split.md"
echo ""
echo "提示：各子仓库的详细 README 位于对应目录下。"
