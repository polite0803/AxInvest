#!/usr/bin/env bash
# scripts/check-line-endings.sh
# Line ending (CRLF/LF) check for CI / pre-commit / local
# Modes: (default check) | --fix | --quiet
#
# 检测核心在 scripts/check-line-endings.mjs（单进程 Node 实现），
# 避免 bash 逐文件起子 shell 在 Windows 上 fork 资源耗尽。
# 本文件仅作 CLI 入口，与 check-hardcoded-i18n.sh 保持同构。
set -euo pipefail
cd "$(dirname "$0")/.."
exec node scripts/check-line-endings.mjs "$@"
