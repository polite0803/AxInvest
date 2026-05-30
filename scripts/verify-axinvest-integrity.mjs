// 验证 AxInvest 分支特有代码在上游合并后是否完整保留
// 用于 post-merge hook 和 pre-push CI
import { existsSync, readdirSync, readFileSync } from "fs";
import { join } from "path";

const ROOT = join(import.meta.dirname, "..");
let errors = 0;

function fail(msg) {
  console.error(`  ❌ ${msg}`);
  errors++;
}

function pass(msg) {
  console.log(`  ✅ ${msg}`);
}

function check(name, fn) {
  console.log(`\n[${name}]`);
  fn();
}

// ── 1. 侧边栏导航 ────────────────────────────────────────────
check("Sidebar 导航入口", () => {
  const f = readFileSync(join(ROOT, "src/components/layout/Sidebar.tsx"), "utf8");

  if (!f.includes('key: "stock-analysis"')) {
    fail("builtinNavItems 缺少 stock-analysis 条目");
  } else {
    pass("builtinNavItems 有 stock-analysis");
  }

  if (!f.includes('"stock-analysis": "/stock-analysis"')) {
    fail("pageKeyToPath 缺少 stock-analysis 映射");
  } else {
    pass("pageKeyToPath 有 stock-analysis 映射");
  }

  if (f.includes("LineChart")) {
    pass("LineChart 已 import");
  } else {
    fail("LineChart 未 import");
  }

  if (f.includes('|| n.key === "stock-analysis"')) {
    pass("section filter 包含 stock-analysis");
  } else {
    fail("section filter 缺少 stock-analysis");
  }
});

// ── 2. 设置侧栏 ──────────────────────────────────────────────
check("SettingsSidebar 设置入口", () => {
  const f = readFileSync(
    join(ROOT, "src/components/settings/SettingsSidebar.tsx"),
    "utf8",
  );

  if (f.includes("stockAnalysis: <TrendingUp")) {
    pass("MENU_ICONS 包含 stockAnalysis");
  } else {
    fail("MENU_ICONS 缺少 stockAnalysis");
  }

  if (f.includes('"stockAnalysis"')) {
    pass("TAB_GROUPS 包含 stockAnalysis");
  } else {
    fail("TAB_GROUPS 缺少 stockAnalysis");
  }

  if (f.includes('"workflow"')) {
    pass("TAB_GROUPS 包含 workflow");
  } else {
    // workflow 由上游管理，可能被移除（冗余入口），不是 AxInvest 资产
    console.log("  ⚠️ TAB_GROUPS 缺少 workflow（上游设计，非 AxInvest 问题）");
  }

  if (f.includes('"knowledgeSettings"')) {
    pass("TAB_GROUPS 包含 knowledgeSettings");
  } else {
    // knowledgeSettings 由上游管理，可能被移除
    console.log("  ⚠️ TAB_GROUPS 缺少 knowledgeSettings（上游设计，非 AxInvest 问题）");
  }
});

// ── 3. 设置页懒加载 ──────────────────────────────────────────
check("SettingsPage 懒加载与路由", () => {
  const f = readFileSync(join(ROOT, "src/pages/SettingsPage.tsx"), "utf8");

  if (f.includes("LazyStockAnalysisSettings")) {
    pass("LazyStockAnalysisSettings import 存在");
  } else {
    fail("LazyStockAnalysisSettings import 丢失");
  }

  if (f.includes("LazyWorkflowEditor") || f.includes("LazyWorkflowSettings")) {
    pass("workflow 懒加载存在");
  } else {
    fail("workflow 懒加载丢失");
  }

  if (f.includes("stockAnalysis:")) {
    pass("SECTION_COMPONENTS 包含 stockAnalysis");
  } else {
    fail("SECTION_COMPONENTS 缺少 stockAnalysis");
  }

  if (f.includes("workflow:")) {
    pass("SECTION_COMPONENTS 包含 workflow");
  } else {
    fail("SECTION_COMPONENTS 缺少 workflow");
  }
});

// ── 4. TypeScript 类型定义 ──────────────────────────────────
check("TypeScript 类型定义", () => {
  const f = readFileSync(join(ROOT, "src/types/index.ts"), "utf8");

  if (f.includes('"stock-analysis"')) {
    pass("BuiltinPageKey 包含 stock-analysis");
  } else {
    fail("BuiltinPageKey 缺少 stock-analysis");
  }

  if (f.includes('"workflow"')) {
    pass("BuiltinPageKey 包含 workflow");
  } else {
    fail("BuiltinPageKey 缺少 workflow");
  }

  if (f.includes('"stockAnalysis"')) {
    pass("SettingsSection 包含 stockAnalysis");
  } else {
    fail("SettingsSection 缺少 stockAnalysis");
  }

  if (f.includes('"knowledgeSettings"')) {
    pass("SettingsSection 包含 knowledgeSettings");
  } else {
    fail("SettingsSection 缺少 knowledgeSettings");
  }
});

// ── 5. tauri.conf.json 品牌 ──────────────────────────────────
check("Tauri 品牌配置", () => {
  const f = readFileSync(join(ROOT, "src-tauri/tauri.conf.json"), "utf8");
  const j = JSON.parse(f);

  if ((j.productName ?? j.app?.productName) === "AxInvest") {
    pass(`productName = "${j.productName ?? j.app?.productName}"`);
  } else {
    fail(`productName = "${j.productName ?? j.app?.productName}" (应为 "AxInvest")`);
  }

  if ((j.identifier ?? j.app?.identifier) === "top.axinvest.desktop") {
    pass(`identifier = "${j.identifier ?? j.app?.identifier}"`);
  } else {
    fail(`identifier = "${j.identifier ?? j.app?.identifier}" (应为 "top.axinvest.desktop")`);
  }

  if (j.app?.windows?.[0]?.title?.includes("AxInvest")) {
    pass("窗口标题包含 AxInvest");
  } else {
    // 窗口标题可能由上游统一管理，不作为阻断项
    console.log("  ⚠️ 窗口标题不包含 AxInvest（可忽略）");
  }
});

// ── 6. Rust 命令注册 ─────────────────────────────────────────
check("Rust 股票分析命令", () => {
  // 模块声明在 commands/mod.rs 中
  const modPath = join(ROOT, "src-tauri/src/commands/mod.rs");
  if (!existsSync(modPath)) {
    fail("commands/mod.rs 不存在");
    return;
  }
  const fMod = readFileSync(modPath, "utf8");

  const required = [
    "stock_analysis",
    "stock_analysis_setup",
    "stock_workflow",
  ];

  for (const mod of required) {
    if (fMod.includes(`mod ${mod}`) || fMod.includes(`pub mod ${mod}`)) {
      pass(`pub mod ${mod} 已声明`);
    } else {
      fail(`commands/mod.rs 缺少: pub mod ${mod}`);
    }
  }

  // 确认关键命令在 lib.rs 的 generate_handler![] 中注册
  const fLib = readFileSync(join(ROOT, "src-tauri/src/lib.rs"), "utf8");

  const keyCmds = [
    "search_stock",
    "create_stock_cron",
    "run_stock_workflow",
    "ensure_stock_analysis_experts_seeded",
  ];

  for (const cmd of keyCmds) {
    if (fLib.includes(cmd)) {
      pass(`命令 ${cmd} 已注册`);
    } else {
      fail(`命令 ${cmd} 未注册`);
    }
  }
});

// ── 7. i18n 键完整性 ─────────────────────────────────────────
check("i18n 语言文件", () => {
  const localesDir = join(ROOT, "src/i18n/locales");
  const files = readdirSync(localesDir).filter((f) => f.endsWith(".json"));

  for (const file of files) {
    try {
      const content = readFileSync(join(localesDir, file), "utf8");
      const j = JSON.parse(content);

      if (j.stockAnalysis && typeof j.stockAnalysis === "object") {
        pass(`${file}: stockAnalysis 键存在`);
      } else {
        fail(`${file}: stockAnalysis 键缺失或非对象`);
      }

      if (j.nav?.stockAnalysis) {
        pass(`${file}: nav.stockAnalysis 存在`);
      } else {
        fail(`${file}: nav.stockAnalysis 缺失`);
      }

      // 关键子键检查
      const critical = ["title", "settings", "klineChart"];
      for (const k of critical) {
        if (!j.stockAnalysis?.[k]) {
          fail(`${file}: stockAnalysis.${k} 缺失`);
        }
      }
    } catch (e) {
      fail(`${file}: JSON 解析失败 — ${e.message}`);
    }
  }
});

// ── 8. 专家文件 ──────────────────────────────────────────────
check("股票分析专家文件", () => {
  const expertsDir = join(ROOT, "src-tauri/agency_experts/stock-analysis");
  if (!existsSync(expertsDir)) {
    fail("agency_experts/stock-analysis 目录不存在");
    return;
  }

  const files = readdirSync(expertsDir).filter((f) => f.endsWith(".md"));

  const required = [
    "market-analyst.md",
    "sentiment-analyst.md",
    "news-analyst.md",
    "fundamentals-analyst.md",
    "policy-analyst.md",
    "hot-money-tracker.md",
    "lockup-watcher.md",
    "research-analyst.md",
    "sector-analyst.md",
    "bull-researcher.md",
    "bear-researcher.md",
    "aggressive-debator.md",
    "conservative-debator.md",
    "neutral-debator.md",
    "research-manager.md",
    "portfolio-manager.md",
    "trader.md",
  ];

  for (const req of required) {
    if (files.includes(req)) {
      pass(req);
    } else {
      fail(`缺失: ${req}`);
    }
  }
});

// ── 9. AxInvest 脚本 ─────────────────────────────────────────
check("AxInvest 专属脚本", () => {
  const scripts = [
    "scripts/post-merge-stock.mjs",
    "scripts/check-i18n-key-exists.mjs",
    "scripts/verify-axinvest-integrity.mjs",
  ];

  for (const s of scripts) {
    if (existsSync(join(ROOT, s))) {
      pass(s);
    } else {
      fail(`缺失: ${s}`);
    }
  }
});

// ── 10. 关键组件文件 ─────────────────────────────────────────
check("关键组件文件", () => {
  const files = [
    "src/components/settings/StockAnalysisSettings.tsx",
    "src/components/stock-analysis/DecisionBanner.tsx",
    "src/components/stock-analysis/KLineChart.tsx",
    "src/components/stock-analysis/TradePanel.tsx",
    "src/components/stock-analysis/RiskMatrix.tsx",
    "src/components/stock-analysis/StockAnalysisPage.tsx",
    "src/components/stock-analysis/StockQuoteCard.tsx",
    "src/stores/feature/stockAnalysisStore.ts",
    "src/types/stock-analysis.ts",
  ];

  for (const f of files) {
    if (existsSync(join(ROOT, f))) {
      pass(f);
    } else {
      fail(`文件丢失: ${f}`);
    }
  }
});

// ── 结果 ─────────────────────────────────────────────────────
console.log(`\n${"=".repeat(50)}`);
if (errors === 0) {
  console.log("✅ AxInvest 完整性检查全部通过！");
  process.exit(0);
} else {
  console.error(`❌ ${errors} 项检查失败！修复后再提交。`);
  process.exit(1);
}
