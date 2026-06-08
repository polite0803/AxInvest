#!/usr/bin/env bash
# scripts/upstream-merge.sh
# 严格上游合并流程 — 保护 i18n locale 文件永不丢失
set -euo pipefail
cd "$(dirname "$0")/.."

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
pass() { echo -e "  ${GREEN}✓${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $1"; }

echo ""
echo "=============================================="
echo "  严格上游合并流程"
echo "=============================================="
echo ""

# ---- Step 0: 前置条件检查 ----
echo "【Step 0】前置条件检查"
UPSTREAM_REMOTE=$(git remote -v | grep upstream | head -1 || true)
if [ -z "$UPSTREAM_REMOTE" ]; then
  fail "没有配置 upstream remote"
  exit 1
fi
UPSTREAM_URL=$(echo "$UPSTREAM_REMOTE" | awk '{print $2}')
pass "upstream: $UPSTREAM_URL"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
pass "当前分支: $BRANCH"

# ---- Step 1: 工作区干净检查 ----
echo ""
echo "【Step 1】工作区状态检查"
if [ -n "$(git status --porcelain)" ]; then
  warn "工作区有未提交的修改："
  git status --short
  echo ""
  echo "  检查 locale 文件是否有变更..."
  LOCALE_CHANGED=$(git diff --name-only | grep 'src/i18n/locales/' || true)
  if [ -n "$LOCALE_CHANGED" ]; then
    echo "  检测到 locale 文件变更，这些文件必须优先提交（防止 stash 丢失）："
    echo "    $LOCALE_CHANGED"
    echo ""
    echo "  请先提交 locale 变更再运行此脚本。"
    echo "    git add src/i18n/locales/"
    echo '    git commit -m "i18n: ..."'
    echo "    $0"
  else
    echo "  无 locale 变更，使用 git stash 暂存其他修改..."
    git stash push -m "upstream-merge: 暂存非 locale 修改 $(date +%Y%m%d-%H%M%S)"
    trap "echo '正在恢复暂存...'; git stash pop 2>/dev/null || true" EXIT
  fi
else
  pass "工作区干净"
fi

# ---- Step 2: 拉取上游 ----
echo ""
echo "【Step 2】获取上游更新"
git fetch upstream --prune
BEFORE=$(git rev-list --count HEAD..upstream/master)
if [ "$BEFORE" -eq 0 ]; then
  echo "  已是最新，无需合并。"
  exit 0
fi
echo "  上游有 $BEFORE 个新提交："
git log HEAD..upstream/master --oneline --no-merges | sed 's/^/    /'

# ---- Step 3: 合并前完整性检查 ----
echo ""
echo "【Step 3】合并前前置检查"
echo "  • 前端格式化 (dprint)..."
npm run format 2>/dev/null
echo "  • Rust 格式化..."
(cd src-tauri && cargo fmt 2>/dev/null)
echo "  • 类型检查..."
npx tsc --noEmit 2>/dev/null && pass "TypeScript 类型检查" || warn "类型检查未通过（继续）"
echo "  • i18n key 完整性..."
node scripts/ci-check.mjs --quick 2>&1 | grep -q '全部检查通过' && pass "i18n key 完整性" || warn "i18n key 有缺失（合并后修复）"
echo "  • 硬编码字符串检查（diff-only）..."
bash scripts/check-hardcoded-i18n.sh --diff-only 2>&1 | tail -3

# ---- Step 4: 执行合并 ----
echo ""
echo "【Step 4】合并上游 master"
git merge upstream/master --no-edit || {
  fail "合并冲突，请手动解决后重新运行。"
  echo "  冲突文件：$(git diff --name-only --diff-filter=U | tr '\n' ' ')"
  exit 1
}
pass "合并成功"

# ---- Step 5: 格式化 ----
echo ""
echo "【Step 5】格式化"
npm run format 2>/dev/null && pass "dprint 格式化"
(cd src-tauri && cargo fmt 2>/dev/null) && pass "cargo fmt"

# ---- Step 6: 完整 CI 检查 ----
echo ""
echo "【Step 6】CI 检查"

echo "  • TypeScript 类型检查..."
npx tsc --noEmit 2>/dev/null && pass "TypeScript" || fail "TypeScript 有类型错误"

echo "  • i18n key 完整性..."
node scripts/ci-check.mjs --quick 2>&1 | grep -q '全部检查通过' && pass "i18n key 完整性" || {
  fail "i18n key 缺失！"
  node scripts/ci-check.mjs --quick 2>&1 | grep "MISSING" || true
  echo ""
  echo "  修复指引："
  echo "    node scripts/fix-i18n.mjs    # 自动补全"
  echo "    或手动检查源码中 t() 调用对应的 locale key"
  echo "    修复后运行: node scripts/ci-check.mjs --quick"
  exit 1
}

echo "  • 硬编码字符串检查（diff-only）..."
bash scripts/check-hardcoded-i18n.sh --diff-only 2>&1 | grep -q "No violations\|PASS" && pass "无新增硬编码字符串" || {
  warn "检测到新增硬编码字符串"
  bash scripts/check-hardcoded-i18n.sh --diff-only 2>&1 | grep "FAIL\|MISSING" || true
  echo "  请将这些字符串迁移到 t() 调用"
}

echo "  • Rust 格式..."
(cd src-tauri && cargo fmt --check 2>/dev/null) && pass "cargo fmt"

# ---- Step 7: 提交合并结果 ----
echo ""
echo "【Step 7】合并结果"

# 如果有 format 修复带来的变更
if [ -n "$(git status --porcelain)" ]; then
  echo "  合并后有 fmt 变更，提交为 style commit..."
  git add -A
  git commit -m "style: 合并上游后的 fmt 修复" 2>/dev/null || true
fi

echo ""
echo "=============================================="
echo -e "  ${GREEN}合并完成${NC}"
echo "  上游提交: $(git log --oneline HEAD~$((BEFORE + 1))..HEAD | head -1)"
echo "  当前 HEAD: $(git rev-parse --short HEAD)"
echo ""
echo "  下一步："
echo "    验证:  node scripts/ci-check.mjs --quick"
echo "    推送:  git push origin $BRANCH"
echo "=============================================="
