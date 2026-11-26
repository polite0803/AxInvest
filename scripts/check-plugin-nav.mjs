#!/usr/bin/env node
// scripts/check-plugin-nav.mjs
// 插件页导航元数据消费面门禁（2026-11-25，PLAN-shell-plus-plugins 收尾）。
//
// 锁住本轮修复的三个导航异常，防止回归到修复前的形态：
//   ① 同图标 —— 插件项图标必须经 resolveIconComponent 按 manifest 解析，不得写死 Puzzle
//   ② 硬编码 —— label 必须按 labelIsI18nKey / navLabelIsI18nKey 决定是否 t()，不得直出 i18n key
//   ③ 全塞一组 —— Sidebar 必须按 navGroup 把插件项分发到对应组，不得把所有插件项塞进 packs
// 附：消费端必须过滤 hidden 项（opc 探针页不进侧栏 / 命令面板）。
//
// 判定口径：对三个消费端 + packager 逐条检查「关键模式必须存在」（正向锁住）。
//   文件缺失 ⇒ 退 1（空洞通过防护，不「跳过 = 通过」）—— 与 check-component-neutrality 同构。
//
// 用法：
//   node scripts/check-plugin-nav.mjs             # 门禁判定
//   node scripts/check-plugin-nav.mjs --selftest   # 内建正负对照（纯函数，不碰真实树）
//
// 退出码：0 = 全部命中；1 = 任一未命中 / 文件缺失 / selftest 失败。

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");

/**
 * 每条检查：file + 中文说明 + 必须命中的正则。
 * 正则锚定当前修复形态的关键语句；重构变量名时本门会红，需同步更新（门禁的正常维护方式）。
 */
const CHECKS = [
  {
    file: "src/components/layout/Sidebar.tsx",
    label: "侧栏插件图标经 resolveIconComponent 解析（防同图标回归）",
    pattern: /resolveIconComponent\(nav\.icon\)/,
  },
  {
    file: "src/components/layout/Sidebar.tsx",
    label: "侧栏过滤插件页 hidden 项（探针页不进侧栏）",
    pattern: /entry\.hidden/,
  },
  {
    file: "src/components/layout/Sidebar.tsx",
    label: "侧栏按 navGroup 分发插件项（防全塞一组回归）",
    pattern: /item\.navGroup === group\.key/,
  },
  {
    file: "src/components/layout/Sidebar.tsx",
    label: "侧栏插件项 label 按 labelIsI18nKey 翻译（防 i18n key 直出）",
    pattern: /labelIsI18nKey \? t\(item\.labelKey\)/,
  },
  {
    file: "src/components/layout/CommandPalette.tsx",
    label: "命令面板插件图标经 resolveIconComponent 解析",
    pattern: /resolveIconComponent\(entry\.icon\)/,
  },
  {
    file: "src/components/layout/CommandPalette.tsx",
    label: "命令面板过滤插件页 hidden 项",
    pattern: /entry\.hidden/,
  },
  {
    file: "src/components/layout/CommandPalette.tsx",
    label: "命令面板插件项 label 按 navLabelIsI18nKey 翻译",
    pattern: /navLabelIsI18nKey \? t\(entry\.navLabel\)/,
  },
  {
    file: "src/components/layout/AppHeader.tsx",
    label: "标题栏插件页 label 按 navLabelIsI18nKey 翻译",
    pattern: /navLabelIsI18nKey \? t\(pluginRoute\.navLabel\)/,
  },
  {
    file: "axagent-plugins/scripts/build-plugin-bundle.mjs",
    label: "packager OPC 行业包经 opcDomainPageDecl 唯一声明面（禁区 12）",
    pattern: /function opcDomainPageDecl/,
  },
];

/** 纯匹配函数：selftest 与主判定共用，保证自检与判定同源。 */
function matchesPattern(source, pattern) {
  return pattern.test(source);
}

function failSelftest(message) {
  console.error(`PLUGIN_NAV_SELFTEST_FAIL: ${message}`);
  process.exit(1);
}

function runSelftest() {
  // 正负对照：验证 matchesPattern 对「含目标 / 不含目标」的判别力，
  // 防止检测函数退化为「永远真 / 永远假」而让门禁空过。
  const cases = [
    { source: "resolveIconComponent(nav.icon)", pattern: /resolveIconComponent\(nav\.icon\)/, want: true },
    { source: "resolveIconComponent(entry.icon)", pattern: /resolveIconComponent\(nav\.icon\)/, want: false },
    { source: "item.navGroup === group.key", pattern: /item\.navGroup === group\.key/, want: true },
    { source: "entry.hidden", pattern: /entry\.hidden/, want: true },
    { source: "labelIsI18nKey ? t(item.labelKey)", pattern: /labelIsI18nKey \? t\(item\.labelKey\)/, want: true },
    { source: "navLabelIsI18nKey ? t(entry.navLabel)", pattern: /navLabelIsI18nKey \? t\(entry\.navLabel\)/, want: true },
    { source: "navLabelIsI18nKey ? t(pluginRoute.navLabel)", pattern: /navLabelIsI18nKey \? t\(pluginRoute\.navLabel\)/, want: true },
    { source: "function opcDomainPageDecl", pattern: /function opcDomainPageDecl/, want: true },
    { source: "", pattern: /entry\.hidden/, want: false },
  ];
  for (const c of cases) {
    const got = matchesPattern(c.source, c.pattern);
    if (got !== c.want) {
      failSelftest(`正负对照失败: 样本=${JSON.stringify(c.source)} 期望=${c.want} 实际=${got}`);
    }
  }
  console.log("PLUGIN_NAV_SELFTEST_OK");
}

function runCheck() {
  const failures = [];
  for (const c of CHECKS) {
    const abs = join(root, c.file);
    if (!existsSync(abs)) {
      failures.push(`[空洞] ${c.file} 缺失（${c.label}）`);
      continue;
    }
    const source = readFileSync(abs, "utf8");
    if (!matchesPattern(source, c.pattern)) {
      failures.push(`[未命中] ${c.file} — ${c.label}`);
    }
  }
  if (failures.length > 0) {
    console.error("PLUGIN_NAV_CHECK_FAIL");
    for (const f of failures) {
      console.error(`  ${f}`);
    }
    process.exit(1);
  }
  console.log(`PLUGIN_NAV_OK checks=${CHECKS.length}`);
}

if (SELFTEST) {
  runSelftest();
} else {
  runCheck();
}
