#!/usr/bin/env node
// scripts/check-component-neutrality.mjs
// 组件中立性门禁（2026-10-31，审查清单建议 #3 落地，team-lead-15 规格）。
//
// 补的是什么盲区：feature 组件直接 import 其它 feature 的 store / 组件，
//   会把「裁剪某个 feature 就编译失败」的耦合越滚越大（实测 420 条存量跨域引用，
//   其中 210 条是对 layout/common/shared 等共享 UI 基元的正常引用，其余是真跨域）。
//
// 规则（静态 import 图，目标 resolve 后判定；动态 import()/require 不在本门职责）：
//   owner = src/components/<X>/** 的 X（X = shared/layout/common/... 一级目录名）
//   违规 = import 目标 resolve 后落在
//     · src/stores/feature/<B>Store   且 normOwner(B) != normOwner(owner)（B 为剥 Store 后缀的目录名）
//     · src/components/<B>/**         且 normOwner(B) != normOwner(owner) 且 B 未逐字登记在 NEUTRAL_PREFIXES
//   **两支故意不同判据**（2026-11-05 任务 #200）：owner 比较问「是不是同一个模块的两种命名形」⇒ 归一；
//   NEUTRAL 成员资格问「是不是人登记过的中性目录」⇒ 逐字命中登记表。理由见 normOwner 注释的作用域段。
//   @/ 别名与 ../ 相对路径统一 resolve 成 src/… posix 路径再判，避免两套口径。
//
// NEUTRAL_PREFIXES（共享 UI 基元 / 纯类型目录，天然跨 feature，**不许被「优化」删掉**）：
//   layout / common / shared / dynamicUI——2026-10-31 实测：Tooltip 69 处、
//   DropdownMenu 25、AntdList 16、MonacoEditor/ErrorBoundary 等，删除判定会把
//   门禁变成 210+ 假阳性灾难。新增中性目录必须在此**逐字登记并写明理由**（AGENTS.md「组件中立性规范」），
//   不许由命名形态自动推导 —— 否则「把目录改成 kebab 形」就白拿豁免，等于静默扩面。
//
// DOMAIN_STORE_OWNERSHIP（C 档域归属映射表，2026-11 消化「面板读自己功能域 store」假阳性）：
//   与 NEUTRAL_PREFIXES **同构**的登记表 = 唯一权威，精确命中，不许靠命名形态自动扩（#200 判例：
//   改名白拿豁免即红）。键 = 组件目录 owner（normOwner 后），值 = 该域自有 store owner 列表（normOwner 后）。
//   语义：store owner ∈ mapping[组件目录 owner] ⇒ 视同自引不报违规。**只作用于 store 支**（component 支不查，
//   否则与 store 同名的组件目录会白拿豁免）；**单向登记**（stock-analysis→serenity 登记了，不代表
//   serenity→stockAnalysis 自动豁免）；未登记的跨域仍红。判定抽成纯函数 isSameDomain(componentOwner, storeOwner)，
//   直引与未来 #222（barrel re-export 解析）共用同一张表。每条映射带中文理由 + 证据文件（见常量处注释）。
//   ⚠ 这不是「豁免跨域」，是「把本属同域的 store 识别出来」—— 映射对之外的一切跨域引用判据一字不动。
//
// 空洞通过防护（2026-11-05 任务 #199）：src/components 缺失 / 扫描面塌方 ⇒ **退 1**，不再「跳过 = 通过」。
//   --selftest 全是纯函数断言、不碰真实树，拦不住「扫描器看不见任何文件」这种退化（棘轮会静默变绿）。
//
// 白名单 scripts/component-neutrality-allowlist.json：
//   fileExemptions  [{ file, targets: [精确 import 串], reason }] —— 精确到文件+目标
//   targetPrefixExemptions [{ prefix, reason }] —— 全项目生效的目标前缀豁免
//   键刻意**不含行号**（行号随编辑漂移，.i18n-allowlist.json 的腐烂是前车之鉴）。
//
// 棘轮：scripts/component-neutrality-baseline.json 存每条违规 {file, target}，
//   只许减少不许增加——新违规必须当场修（下沉/注入/事件化）或显式登记白名单；
//   修掉存量后跑 --update-baseline 收紧基线（若出现新条目，--update-baseline 拒绝执行）。
//
// 用法：
//   node scripts/check-component-neutrality.mjs                    # 门禁
//   node scripts/check-component-neutrality.mjs --selftest         # 内建正负对照
//   node scripts/check-component-neutrality.mjs --update-baseline  # 收紧棘轮基线
//
// 退出码：0 = 无新增违规（或 selftest 全过）；
//         1 = 有新违规 / 扫描面塌方（空洞通过）/ src/components 缺失 / selftest 失败。

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const SRC_COMPONENTS = join(root, "src", "components");

const args = process.argv.slice(2);
const SELFTEST = args.includes("--selftest");
const UPDATE_BASELINE = args.includes("--update-baseline");

const BASELINE_PATH = join(root, "scripts", "component-neutrality-baseline.json");
const ALLOWLIST_PATH = join(root, "scripts", "component-neutrality-allowlist.json");

// ── 共享 UI 基元目录（目标侧中性；改动必须同步头部注释）──
// 登记表 = **唯一权威**，匹配时逐字命中（精确匹配，不随 normOwner 归一；理由见 normOwner 作用域段）。
const NEUTRAL_PREFIXES = ["layout", "common", "shared", "dynamicUI"];

// ── C 档域归属映射表（与 NEUTRAL_PREFIXES 同构的登记表；改动必须同步头部注释 + selftest）──
// 键 = 组件目录 owner（normOwner 后：lowercase + 去 -/_）；值 = 该域自有 store owner 列表（normOwner 后，
//   store 名已由 adjudicateImport 剥 Store 后缀）。匹配经纯函数 isSameDomain，输入原形名内部归一后精确命中。
// **只认精确登记，不许靠命名形态自动扩**（#200 判例）：未登记的 store / 反向 / component 支一律仍红。
// 每条映射的理由与证据文件逐条写在下面 —— 新增映射必须照此登记 + 写理由，不许静默扩面。
const DOMAIN_STORE_OWNERSHIP = {
  // stock-analysis 域：趋势智选 / 截图诊断 / 模拟组合 / 股票工作流聊天桥都是股票分析功能的自有 store。
  //   证据：SerenityScreeningPanel.tsx:6-12（趋势智选面板读 serenityStore），后端 commands/stock_workflow/serenity.rs；
  //   ScreenshotDiagnosisDashboard.tsx:15、PaperPortfolioDashboard.tsx:14；workflowChatCard.ts:7 注释明言
  //   「生产端 stockWorkflowChatBridge 是股票域文件」（该 store 当前无组件直引，登记为域归属备用，不消账面）。
  stockanalysis: ["serenity", "screenshotdiagnosis", "paperportfolio", "stockworkflowchatbridge",
    // #222 补洞扩登（2026-11-08）：workspaceStore 被 _workspace/ 整个子树消费（shell/TabBar/Switcher/views/
    //   useSimpleMode 及 density/integration 测试）—— 股票工作区壳层自有 store。证据：StockWorkspaceShell.tsx。
    "workspace"],
  // workflow 域：workflowEditor（编辑器态）/ workEngine（引擎执行态）是工作流编辑器的自有 store。
  //   证据：AIPanel/AIPanel.tsx:5-6、WorkflowEditor.tsx:44、DebugPanel.tsx:9、Edges/BaseEdge.tsx:3、Nodes/BaseNode.tsx:4。
  workflow: ["workfloweditor", "workengine"],
  // skill 域：skillExtension 与 skillStore 互引同域 —— skillExtensionStore.ts:21 静态导入 skillStore，
  //   双方仅函数体内引用对方构成循环依赖（该文件 :18-19 注释自证）⇒ 同一功能域。证据：SkillToolbar.tsx:8。
  skill: ["skillextension"],
  // devtools 域：rlTraining 是 RL 训练的唯一 UI 宿主 store。证据：RLTrainingPanel.tsx:6,22（面板读训练态）、
  //   RLCheckpointManager.tsx:3-4、RLTrainingConfig.tsx:5-6、RLTrainingMonitor.tsx:3。
  devtools: ["rltraining"],
  // chat 域：topicGroup（会话内主题分组）/ cognitiveRoute / buddy / cache（按 conversationId 记 prompt 缓存）
  //   都是聊天功能的自有 store。证据：TopicGroupDivider.tsx:4-5（会话内主题分组）、ChatView.tsx:29、
  //   CognitiveRoutePanel.tsx、BuddyMessage.tsx / BuddyWidget.tsx、RightPanelContainer.tsx（cacheStore）。
  chat: ["topicgroup", "cognitiveroute", "buddy", "cache"],
  // settings 域：deviceSync / theme / userProfile / appConfig / voicePreference 的管理 UI 唯一宿主在 settings。
  //   证据：EncryptionSettingsPanel.tsx:3,17、ThemeManager.tsx:5、UserProfileSettings.tsx:11、
  //   SettingsPanel.tsx:5-6（appConfig）、GeneralSettings.tsx:7-8（voicePreference）、RealtimePushPanel.tsx（deviceSync）。
  // #222 补洞扩登（2026-11-08）：settings 目录是**全功能设置面板聚集地**——下列 19 个 store 的管理面板
  //   物理宿主在 settings/（组件名即证据），barrel 改判后逐条显形为「跨域读」，裁其域归属：
  //   provider（ProviderSettings/ProviderList/ProviderDetail/DefaultModelSettings/AdvancedSettings/
  //     BalanceBadge/LocalModelPanel/ModelDownloadPanel）、backup（BackupCenter/SchedulerSettings）、
  //   localTool（LocalToolSettings/ToolManager）、mcp（McpServerSettings）、memory（MemorySettings）、
  //   knowledge（KnowledgeBaseDocuments/PaperOverviewPanel/SourceManager/KnowledgeGraphSearchPanel）、
  //   scheduler（SchedulerSettings）、search（SearchProviderSettings）、readingList（ReadingListSettings）、
  //   platform（GatewayConfigPanel/MessageChannelSettings/PlatformStatusCard）、knowledgeGraph、
  //   source（SourceManager/DirectoryImportWizard）、llmWiki（DirectoryImportWizard）、
  //   dynamicUI（DynamicPagesSettings）、onboarding（AboutPage 重置引导）、currency（GeneralSettings 币种）、
  //   paper（PaperOverviewPanel）、skillExtension（SettingsSidebar 开关）、knowledgeSource（SourceManager）。
  settings: [
    "devicesync", "theme", "userprofile", "appconfig", "voicepreference",
    "provider", "backup", "localtool", "mcp", "memory", "knowledge", "scheduler",
    "search", "readinglist", "platform", "knowledgegraph", "source", "llmwiki",
    "dynamicui", "onboarding", "currency", "paper", "skillextension", "knowledgesource",
  ],
  // layout 域：userProfile 的弹窗宿主 UserProfileModal 在 layout（全局用户资料入口）。证据：UserProfileModal.tsx:14。
  layout: ["userprofile"],
  // link 域：网关链功能页目录（AddGatewayLinkModal/Detail/List/Models/Overview/Policies/Skills）全读
  //   gatewayLinkStore —— 该目录整个就是 gateway-link 功能域，管理面自引。#222 补洞扩登（2026-11-08）。
  link: ["gatewaylink"],
};

// ── 横切 store 登记表（#194 批 D 后续，2026-11-08）：这些 store 是全局关注点，任何组件域
// 直读都合法 —— 豁免按 **store 目标**（不分读方域），与 DOMAIN_STORE_OWNERSHIP 的「域自有」
// 语义互补。登记须带 reason，normOwner 归一后精确命中，不许静默扩（#200）。
// 仅 store 支调用（findViolations），component 支不查 —— 组件跨域走 NEUTRAL 登记，语义不同。
const CROSS_CUTTING_STORES = {
  // appConfig：全局应用配置（主题 / 快捷键 / 面板开关），读方遍布 chat（AwarenessPanel /
  //   SuggestionPanel）、settings（SettingsPanel，已另登记为 settings 自有）等 —— 全局配置读面
  //   横切是语义本相，不是某个域的自有 store。证据：AwarenessPanel.tsx、SuggestionPanel.tsx。
  appconfig: "全局应用配置，读面横切（chat AwarenessPanel / SuggestionPanel 等）",
  // security：数据源安全设置（归属闸门 / 权限面），dynamicUI 渲染器渲染任何卡片前都要查安全归属。
  //   证据：DynamicUIRenderer.tsx（渲染前查 securityStore）。
  security: "数据源安全设置，dynamicUI 渲染器渲染前必查",
  // expert：专家名册（全局清单），选择器 / 属性面板 / 聊天侧徽标都要列专家 —— 名册读面横切。
  //   证据：ExpertSelector.tsx（chat）、AgentPropertyPanel.tsx（workflow）、ExpertBadge.tsx（chat）。
  expert: "专家名册，选择器 / 徽标 / 属性面板跨域读",
  // ── #222 补洞扩登（2026-11-08）── 以下 4 个 store 的读面横切同样是语义本相（barrel 改判后显形）：
  // settings：全局偏好配置（模型参数 / 温度 / 语言 / 主题），任何显示模型选择或偏好读取的面板都要读。
  //   证据：chat ModelSelector.tsx / ChatMinimap.tsx / ChatViewMessages.tsx、layout AppInitializer.tsx。
  settings: "全局偏好配置，读面横切（ModelSelector / AppInitializer 等）",
  // provider：LLM 提供商与模型注册表（清单数据源），任何模型选择 UI 都要列提供商。
  //   证据：chat ModelSelector.tsx、shared ModelSelect.tsx / EmbeddingModelSelect.tsx、
  //   workflow LLMPropertyPanel.tsx（属性面板选模型）。
  provider: "LLM 提供商注册表，模型选择 UI 跨域读",
  // currency：币种格式化工具（formatCny / useFormatCny），任何显示金额的面板都要读 —— 纯展示工具。
  //   证据：devtools CostChart.tsx / TraceDetail.tsx、settings GeneralSettings.tsx、chat AgentStatsPanel.tsx。
  currency: "币种格式化工具，金额显示面板跨域读",
  // onboarding：新手引导状态（引导气泡全局弹出 / 完成度），全局壳与帮助页都要查引导态。
  //   证据：layout AppHeader.tsx / AppInitializer.tsx / Sidebar.tsx、chat TabBar.tsx、help HelpPanel.tsx。
  onboarding: "新手引导状态，全局壳与帮助页跨域读",
  // ── #222 补洞批裁定（2026-11-08，Lane B 报告 AppInitializer 未做项）：以下 2 个 store 的消费
  // 全部是启动序列 getState() 直读（无订阅、无渲染依赖），与 conversation（domain 层不判）、
  // onboarding（已登记横切）、settings（:75）在同一启动三连里并列 —— 显红是登记缺位而非跨域
  // UI 耦合；hook 化要么引入原实现没有的重渲时机（改语义），要么只是让门禁失明的绕过。
  // gateway：网关生命周期（自启开关，AppInitializer.tsx:119 startGateway）—— 网关自启是全局
  //   关注点（"只有开关没有执行方，2026-10-25 接通"，AppInitializer.tsx:115-123 注释自证）。
  gateway: "网关生命周期，全局启动序列 getState 直读（AppInitializer.tsx:119）",
  // skillExtension：技能扩展清单预热（AppInitializer.tsx:142 fetchSkills，启动三连之一）。
  //   ⚠ 键必须写 norm 形态（全小写）：本表 lookup 只归一查询侧（storeOwner），键侧不归一。
  skillextension: "技能扩展清单预热，全局启动序列 getState 直读（AppInitializer.tsx:142）",
};

// ── 双向同域组（#194 存疑 7 裁定，2026-11-08）：组内任两域**互认同域**（store 支双向豁免）。
// 与 DOMAIN_STORE_OWNERSHIP 的单向不同：这里两个方向都合法。仅 store 支。
// chat ↔ proactive：SuggestionPanel（chat）读 proactiveStore 取建议数据；proactive 域组件读
//   chat 会话态取上下文 —— 两个功能域共享一条数据流（proactive 建议源自聊天上下文）。
//   证据：SuggestionPanel.tsx（chat 读 proactive）。
const BIDIRECTIONAL_GROUPS = [
  ["chat", "proactive"],
];

/** 横切 store 判定（纯函数）：storeOwner 登记为全局关注点 ⇒ 任何域直读合法。归一后精确命中。 */
export function isCrossCuttingStore(storeOwner) {
  return CROSS_CUTTING_STORES[normOwner(storeOwner)] !== undefined;
}

/** 双向同域判定（纯函数）：两 owner 同属 BIDIRECTIONAL_GROUPS 某一组 ⇒ 互认。组外一律 false。 */
export function isBidirectionalSameGroup(ownerA, ownerB) {
  return BIDIRECTIONAL_GROUPS.some(
    (g) => g.includes(normOwner(ownerA)) && g.includes(normOwner(ownerB)),
  );
}

// ── @/stores barrel 改判（#222 覆盖洞，2026-11-08）──
// 洞：组件写 `import { useXStore } from "@/stores"`（走 barrel）时 resolveImportTarget 得
// "src/stores"，store 支正则 /^src\/stores\/feature\// 不命中 ⇒ 整条 import 静默绕过扫描。
// 实测组件目录 246 处 barrel 导入。修法：启动时解析 src/stores/index.ts 的 re-export 表
// （值导出符号 → 真实 store 路径），barrel 导入按**导入符号**逐个改判回真实路径再走原判定。
// 判定链（sameOwner / DOMAIN_STORE_OWNERSHIP / 横切 / 双向）与深路径完全共用，不给 barrel 任何额外豁免。
// 只收**值导出**：`export type` 与 `{ type X }` 修饰的符号不入表 —— 类型导入无运行时依赖，不判。
// `export *` 在 src/stores/index.ts 中不存在（2026-11-08 grep 证实零命中）。若未来引入，
// buildStoreBarrelMap 不会展开它 ⇒ 对应符号查表 miss 静默跳过 ⇒ **重新打开覆盖洞**。
// 引入者必须同步扩展 buildStoreBarrelMap 递归解析子模块，并补 selftest 断言。
let storeBarrelMap = new Map();

/** 注入 barrel 映射（main 读盘构建后注入；selftest 注入手造表保持纯函数可测）。 */
export function setStoreBarrelMap(map) {
  storeBarrelMap = map;
}

/** 解析 stores barrel 文本 → Map<导出符号名, "src/stores/…">。仅值导出；`A as B` 记对外名 B。 */
export function buildStoreBarrelMap(content) {
  const map = new Map();
  const EXPORT_RE = /export\s*\{([^}]*)\}\s*from\s*["'](\.[^"']+)["']/g;
  let em;
  while ((em = EXPORT_RE.exec(content)) !== null) {
    const base = "src/stores/" + em[2].replace(/^\.\//, "").replace(/\.js$/, "");
    for (const raw of em[1].split(",")) {
      const item = raw.trim();
      if (!item || item.startsWith("type ")) { continue; }
      const asM = item.match(/^(\S+)\s+as\s+(\S+)$/);
      const exportedName = asM ? asM[2] : item;
      map.set(exportedName, base);
    }
  }
  return map;
}

/**
 * 提取 import 语句的导入符号（as 前原始名 —— 那是模块侧导出名，即 barrel map 的 key）。
 * start = 语句的 `import` 起点索引，end = FROM_RE 本次 match 的结束索引。
 * type-only 整条语句（`import type { X } from …`）返回 []（无运行时依赖）。
 */
export function importedNamesAt(content, start, end) {
  const stmt = content.slice(start, end);
  if (/^import\s+type\b/.test(stmt)) { return []; }
  const braceM = stmt.match(/\{([^}]*)\}/);
  if (!braceM) { return []; } // default / namespace 导入：barrel 无此形态，跳过
  const names = [];
  for (const raw of braceM[1].split(",")) {
    const item = raw.trim();
    if (!item || item.startsWith("type ")) { continue; }
    const asM = item.match(/^(\S+)\s+as\s+(\S+)$/);
    names.push(asM ? asM[1] : item);
  }
  return names;
}

/** 单条判定结果的违规裁决（store 支查映射/横切/双向三支，component 支查 NEUTRAL）——深路径与 barrel 共用。 */
export function isViolationOf(j, selfOwner) {
  const sameOwner = normOwner(j.targetOwner) === normOwner(selfOwner);
  const sameDomain = j.kind === "store"
    && (isSameDomain(selfOwner, j.targetOwner)
      || isCrossCuttingStore(j.targetOwner)
      || isBidirectionalSameGroup(selfOwner, j.targetOwner));
  return j.kind === "store" ? !sameOwner && !sameDomain : !sameOwner && !isNeutralOwner(j.targetOwner);
}

// ── 与存量实测同一套 resolve（@/ 别名 + 相对路径 → src/… posix）──
export function resolveImportTarget(specifier, fromFile) {
  if (specifier.startsWith("@/")) { return "src/" + specifier.slice(2); }
  if (specifier.startsWith(".")) {
    const parts = fromFile.split("/").slice(0, -1);
    for (const seg of specifier.split("/")) {
      if (seg === ".") { continue; }
      else if (seg === "..") { parts.pop(); }
      else { parts.push(seg); }
    }
    return parts.join("/");
  }
  return null; // node_modules / 裸包名
}

const FROM_RE = /\bfrom\s*["']([^"']+)["']/g;

/** owner 判定：src/components/<X>/… → X；src/components/<file> → "(root)"；非 components → null */
export function ownerOf(resolvedPath) {
  const m = resolvedPath.match(/^src\/components\/([^/]+)/);
  if (m) { return m[1]; }
  if (/^src\/components\/[^/]+$/.test(resolvedPath)) { return "(root)"; }
  return null;
}

/** 单条 import 判定。返回 null（不涉规则）或 { kind, targetOwner, target }。 */
export function adjudicateImport(resolvedTarget) {
  const sm = resolvedTarget.match(/^src\/stores\/feature\/([^/]+)/);
  if (sm) {
    const b = sm[1].replace(/Store$/, ""); // styleStore → style：自引不算跨域
    return { kind: "store", targetOwner: b, target: resolvedTarget };
  }
  const cm = resolvedTarget.match(/^src\/components\/([^/]+)/);
  if (cm) {
    return { kind: "component", targetOwner: cm[1], target: resolvedTarget };
  }
  return null;
}

/**
 * owner 名归一 —— 2026-11 任务 #168 批 A 的**判据变更**（不是豁免扩容）。
 *
 * 为什么必须归一：owner 有两套命名形 —— 组件目录用 kebab-case，store 文件用 camelCase。
 *   旧判据「全等字符串」把**同一个 owner 的两种写法**判成跨域 ⇒ 实测 22 条假阳性
 *   （23 个 import 实例）：`src/components/stock-analysis/**` 引 `@/stores/feature/stockAnalysisStore`
 *   21 条 + `multi-agent` 引 `multiAgentStore` 1 条 + 同 target 重复 1 条（棘轮键去重后不计）。
 *
 * 安全性判据（**实测零假阴**，动手前独立复量，非推断）：
 *   并集 = 41 个组件一级目录名 ∪ 78 个 stores/feature 名（剥 `Store` 后缀后参与，其中 73 个真带后缀、
 *   5 个不带：executionPhaseMachine / executionToolCallUtils / quant / stockWorkflowChatBridge /
 *   workflowStoreUtils）= 去重 100 个名字 ⇒ 归一后 97 键，同键组（成员 >1）**仅 3 组**，
 *   且每组都「目录形与 store 形同时命中」= 同一模块的两种写法：
 *     llmwiki:        llm-wiki        / llmWiki
 *     multiagent:     multi-agent     / multiAgent
 *     stockanalysis:  stock-analysis  / stockAnalysis
 *   ⇒ 不存在「两个真不同 owner 被归一成同名」的组 ⇒ 归一不会把跨域合并成自引，零假阴。
 *
 * 命名形态计数（决定归一只需抹平 -/_ 与大小写）：
 *   kebab 目录 6（fine-tune / llm-wiki / multi-agent / stock-analysis / stock-workspace / time-travel）
 *   snake 目录 0 · camel 目录 1（dynamicUI）· stores/feature 78 个名含 -/_ 的 **0 个**（100% camelCase）
 *   ⇒ 不加「剥后缀」之类新语义（剥 `Store` 后缀是既有判据，见 adjudicateImport，本批不动）。
 *
 * 作用域（关键，越界即洗白）：
 *   ✔ 只作用于 **owner 归属推导与比较的两支** —— selfOwner 支与 targetOwner 支（`findViolations` 里的
 *     `normOwner(j.targetOwner) === normOwner(selfOwner)`，两支一起，一字不可拆）。
 *   ✘ **不作用于 NEUTRAL 成员资格**（`isNeutralOwner`）。2026-11-05 任务 #200 把它退回精确匹配，
 *     并**推翻了本文件此前那句「NEUTRAL 不归一会和 owner 支分裂成两套口径」** —— 那句话把两类
 *     语义不同的比较当成了同一件事，是错的。两支故意不同判据：
 *       · owner 比较问「这两个名字是不是**同一个模块**的两种命名形」（目录 kebab vs store camel）
 *         ⇒ 归一是正确判据，#168 批 A 据此消掉 22 条同 owner 假阳性；
 *       · NEUTRAL 问「这个目录名是不是**人登记过**的中性件目录」——AGENTS.md「组件中立性规范」明文
 *         「新增中性目录必须登记进脚本 NEUTRAL_PREFIXES 并写明理由，不许静默扩」。登记是**人的决定**，
 *         命名形态推不出来。若一并归一，把某个 feature 目录改名成 `dynamic-ui` 这种「未登记的 kebab
 *         变体」就**白拿豁免** ⇒ 扩面动作从「改一行登记表 + 写理由 + 被评审」退化成「改目录名」，
 *         白名单与棘轮同时形同虚设。
 *     真实数据上收窄的影响为 **0**（已独立复量：现存 4 个登记名没有任何 kebab/snake 变体出现在
 *     import 图里，判定档读数逐字不变）⇒ selftest ③d 因此可以从正控翻成负控而不改账面。
 *   ✘ 不作用于棘轮键：键仍是 `file + " → " + target`（target = import 字面量原文）。
 *     键里**不含 targetOwner** ⇒ 把归一并入键会让 212 条基线全部失配 ⇒ 那是洗白，不是修判据。
 *   ✘ 不作用于白名单匹配：白名单比的是 `file` 与 import 字面量/前缀，不是目录名（见 main 的过滤段）。
 *
 * ⚠ 本批是判据变更，必须单独成批：**收紧基线（212 → 186）是另一批（批 B）**，
 *   故此处不跑 --update-baseline，212 仍是上限，读数里新增的 22 条「已修复」是**账面而非钉账**。
 */
export function normOwner(name) { return name.toLowerCase().replace(/[-_]/g, ""); }

/**
 * NEUTRAL 成员资格 = **逐字命中登记表**，不随 normOwner 归一（2026-11-05 任务 #200 收窄，leader 裁定）。
 * 为什么与 owner 两支故意不同判据（而不是「口径分裂」）：见 normOwner 注释的作用域段 ——
 * owner 支问「是不是同一个模块的两种写法」，本支问「是不是登记过的中性目录」，后者是人的决定。
 * 钉住它的两条 selftest 断言：③c（登记名原形 dynamicUI 仍豁免，正控）、
 * ③d（未登记的 kebab 变体 dynamic-ui 必须报违规，负控）。
 */
function isNeutralOwner(targetOwner) {
  return NEUTRAL_PREFIXES.includes(targetOwner);
}

/**
 * C 档域归属判定（纯函数，直引与未来 #222 barrel re-export 解析共用）。
 * 问「storeOwner 这个 store 是否登记为 componentOwner 这个组件域的自有 store」⇒ 是则视同自引。
 * 输入收**原形名**（组件目录 kebab / store camel 皆可），内部走 normOwner 归一后精确命中 DOMAIN_STORE_OWNERSHIP。
 * 三条安全边界（与 NEUTRAL 同构，不许静默扩）：
 *   · 单向：只查 mapping[componentOwner]，不查反向 ⇒ serenity→stockAnalysis 不因 stock-analysis→serenity 自动豁免；
 *   · 精确：未登记的 store owner 一律 false ⇒ 改名 / 命名形态推导白拿不到豁免（#200）；
 *   · 仅 store 支调用（见 findViolations）：component 支不查本表，避免与 store 同名的组件目录白拿豁免。
 * normOwner 幂等（lowercase + 去 -/_），故对已归一名再归一无副作用。
 */
export function isSameDomain(componentOwner, storeOwner) {
  const owned = DOMAIN_STORE_OWNERSHIP[normOwner(componentOwner)];
  return owned !== undefined && owned.includes(normOwner(storeOwner));
}

/** 纯函数：给定文件内容 → 违规条目列表（selftest 与正式跑共用）。 */
export function findViolations({ file, content, owner }) {
  const violations = [];
  let m;
  FROM_RE.lastIndex = 0;
  while ((m = FROM_RE.exec(content)) !== null) {
    const spec = m[1];
    const resolved = resolveImportTarget(spec, file);
    if (!resolved) { continue; }
    const selfOwner = owner ?? ownerOf(resolved);
    if (!selfOwner) { continue; }
    // ── @/stores barrel 改判（#222）：按导入符号逐个映射回真实 store 路径再判定 ──
    // 未命中映射的符号（纯类型 / 非 store 工具函数）无 store 语义，跳过；target 存原形 "@/stores"。
    if (resolved === "src/stores" || resolved === "src/stores/index") {
      const stmtStart = content.lastIndexOf("import", m.index);
      for (const name of importedNamesAt(content, stmtStart, FROM_RE.lastIndex)) {
        const realPath = storeBarrelMap.get(name);
        if (!realPath) { continue; }
        const jb = adjudicateImport(realPath);
        if (!jb) { continue; }
        if (isViolationOf(jb, selfOwner)) {
          violations.push({ file, target: spec, resolvedTarget: jb.target, kind: jb.kind, targetOwner: jb.targetOwner });
        }
      }
      continue;
    }
    const j = adjudicateImport(resolved);
    if (!j) { continue; }
    // owner 两支（selfOwner / targetOwner）同走 normOwner 归一；NEUTRAL 支逐字命中登记表（#200）。
    // 两支故意不同判据的理由见 normOwner 注释的作用域段，不要「顺手统一」成三支一起归一或一起全等。
    // targetOwner 与 target 在条目里**存原形**，棘轮键不受归一影响（见 normOwner 注释的作用域）。
    // 判定体已抽 isViolationOf（深路径与 #222 barrel 改判共用，不给 barrel 额外豁免）。
    if (isViolationOf(j, selfOwner)) {
      violations.push({ file, target: spec, resolvedTarget: j.target, kind: j.kind, targetOwner: j.targetOwner });
    }
  }
  return violations;
}

// ── 文件收集 ──
function collectFiles(dir, out) {
  if (!existsSync(dir)) { return out; }
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) { collectFiles(p, out); }
    else if (/\.(ts|tsx)$/.test(name) && !name.endsWith(".d.ts")) { out.push(p); }
  }
  return out;
}

/**
 * 空洞通过（hollow pass）防护 —— 2026-11-05 任务 #199。
 *
 * 拦的是什么：这道门是**棘轮**，判据是「本轮违规集合 ⊆ 基线集合」。扫描器一旦看不见文件
 *   （`src/components` 被改名/搬迁、根路径写错、后缀口径退化、构建产物目录被清空），
 *   违规集合就是空集 ⇒ 空集是任何集合的子集 ⇒ **PASS 退 0**。此后新写的跨域引用永远不会被拦，
 *   门从棘轮静默退化成「永远绿」。CI 里 `--selftest` 先跑的那一步**救不了它**：selftest 全是
 *   纯函数断言、不碰真实树（见 runSelftest 头注释「不碰真实 src」），扫描器瞎掉时它照样全绿。
 *
 * 双条件锚（**都不是魔法数**，两条各自兜住对方退化的那一半）：
 *   a) 绝对下界 `scannedFilesCount >= MIN_SCANNED_FILES`：
 *      兜住「基线缺失/损坏/为空 ⇒ 条件 b 退化成 `>= 0`」这一支（历史上基线文件曾不存在，见 --update-baseline
 *      的 isFirstEstablish 分支，那不是假想情形）。
 *      取值理由：2026-11-05 实测扫描面 = **927** 个 .ts/.tsx（本文件尾部报告即读数量）。
 *      取 **1/3 向下取整到百位 = 300**：允许一次「把组件拆到别处 / 大规模合并目录」的重构（账面腰斩两次
 *      仍不误红），但拦住任何数量级的塌方（0、个位数、只剩一个子目录）。锚是**比例**不是瞬时读数，
 *      所以不随每次提交漂移；漂移的是报告里那个 927。
 *   b) 相对锚 `scannedFilesCount >= 基线里出现的不同 file 数`（仅基线非空时生效）：
 *      这条**随棘轮自更新**，是人写的数字里唯一机械可解释的那个：212 条基线键覆盖 **157** 个不同
 *      `src/components/**` 文件，而每一条键的 file 都必然来自某一轮真实扫描 ⇒「扫描面」的下界
 *      天然就是「基线面」。扫描面掉到基线面以下 ⇒ 有一批**仍在被账面保护的引用所在的文件根本没被读到**
 *      ⇒ 判定不作数。收紧基线时该下界同步下降，不需要有人记得改常量。
 *      为什么用**计数**而不是「基线 file ⊆ 扫描 file」的集合包含：实测 157 个基线 file 里已有 **2 个
 *      在磁盘上不存在**（存量修复时文件被删/搬走，棘轮键残留属正常 decay，靠 --update-baseline 收干），
 *      集合包含会当场误红。计数只问「读到的面积够不够」，不问「每一个历史 file 是否还在」，
 *      正好是要的语义强度。
 *
 * 纯函数，不读盘 ⇒ selftest 可以用注入数字做正负控（真实树上「927 ≥ 300」是永真断言，钉不住退化）。
 */
const MIN_SCANNED_FILES = 300;

export function scanFloorFailures(scannedFilesCount, baselineFilesCount) {
  const problems = [];
  if (scannedFilesCount < MIN_SCANNED_FILES) {
    problems.push(
      `条件 a（绝对下界）：本轮只扫描到 ${scannedFilesCount} 个 .ts/.tsx，< 下界 ${MIN_SCANNED_FILES}` +
        `（2026-11-05 实测扫描面 927 的 1/3）—— 扫描面塌方时「0 条新违规」不代表「没有违规」。`,
    );
  }
  if (baselineFilesCount > 0 && scannedFilesCount < baselineFilesCount) {
    problems.push(
      `条件 b（相对基线面）：本轮扫描到 ${scannedFilesCount} 个不同 file < 棘轮基线里出现的 ` +
        `${baselineFilesCount} 个不同 file —— 有仍在基线账面里的文件没被读到，判定不作数。`,
    );
  }
  return problems;
}

// ── selftest：正负对照（不碰真实 src）──
// ⚠ 本函数**只测判据逻辑**，从不读真实文件树 ⇒ 它拦不住「扫描器看不见任何文件」这类退化，
//   那件事由 main 里的空洞通过防护（scanFloorFailures）负责；本函数用**注入数字**验它自己的判据（⑥）。
function runSelftest() {
  let fail = 0;
  let total = 0;
  const check = (cond, msg) => {
    total++;
    if (!cond) { console.error(`SELFTEST FAIL：${msg}`); fail++; }
  };

  // resolve：@/ 别名
  check(resolveImportTarget("@/stores/feature/xStore", "src/components/a/b.tsx") === "src/stores/feature/xStore",
    "resolve @/ 别名");
  // resolve：相对路径逐级上跳
  check(resolveImportTarget("../../stores/feature/decompositionStore", "src/components/decomposition/x.tsx") === "src/stores/feature/decompositionStore",
    "resolve ../../ 相对路径");
  check(resolveImportTarget("../gateway/tokenFormat", "src/components/chat/x.tsx") === "src/components/gateway/tokenFormat",
    "resolve ../ 相对路径");
  // resolve：node_modules 不判
  check(resolveImportTarget("react", "src/components/a.tsx") === null, "node_modules 不 resolve");

  // 负控：feature 组件 import 别的 feature 的 store ⇒ 违规
  const v1 = findViolations({
    file: "src/components/chat/AgentExecutionPanel.tsx",
    content: 'import { useExecutionStore } from "@/stores/feature/executionStore";\n',
    owner: "chat",
  });
  check(v1.length === 1 && v1[0].kind === "store" && v1[0].targetOwner === "execution",
    "负控：chat 引 executionStore 必须红");

  // 正控：自引自己的 store（剥 Store 后缀）⇒ 不红
  const v2 = findViolations({
    file: "src/components/style/StylePreviewPanel.tsx",
    content: 'import { useStyleStore } from "@/stores/feature/styleStore";\n',
    owner: "style",
  });
  check(v2.length === 0, "正控：style 引 styleStore 是自引，不得假阳性");

  // 正控：中性 UI 基元目录（layout/common/shared/dynamicUI）⇒ 不红
  const v3 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { Tooltip } from "@/components/layout/Tooltip";\nimport { E } from "@/components/shared/ErrorBoundary";\nimport { R } from "@/components/dynamicUI/DynamicUIRenderer";\n',
    owner: "chat",
  });
  check(v3.length === 0, "正控：NEUTRAL_PREFIXES 目录不得假阳性");

  // 负控：跨 feature 组件引用 ⇒ 违规
  const v4 = findViolations({
    file: "src/components/stock-workspace/ContextSidebar.tsx",
    content: 'import { A } from "@/components/stock-analysis/AnnouncementsPanel";\n',
    owner: "stock-workspace",
  });
  check(v4.length === 1 && v4[0].kind === "component" && v4[0].targetOwner === "stock-analysis",
    "负控：stock-workspace 引 stock-analysis 组件必须红");

  // 正控：非 components 文件不判 owner（null ⇒ skip）
  const v5 = findViolations({
    file: "src/pages/X.tsx",
    content: 'import { S } from "@/stores/feature/agentStore";\n',
    owner: null,
  });
  check(v5.length === 0, "正控：components 目录之外的文件不判");

  // ── 归一判据（#168 批 A）正负对照 ──
  // normOwner 自身形态：抹平 -/_ 与大小写，不引入「剥后缀」新语义
  check(normOwner("stock-analysis") === normOwner("stockAnalysis") && normOwner("multi-agent") === normOwner("multiAgent") &&
      normOwner("llm-wiki") === normOwner("llmWiki"), "归一正控：-/_ 与大小写抹平");
  check(normOwner("dynamicUI") === "dynamicui", "归一只是比较口径，不改登记面");
  check(normOwner("stockAnalysisStore") !== normOwner("stockAnalysis"),
    "归一负控：norm 不得顺手剥 Store 后缀（剥后缀是 adjudicateImport 的既有语义）");

  // ① 正控：kebab 目录引同 owner 的 camel store ⇒ 不再假阳性（本批改掉 22 条的那一族）
  const n1 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";\n',
    owner: "stock-analysis",
  });
  check(n1.length === 0, "归一正控①：stock-analysis 引 stockAnalysisStore 是自引，不得再报");
  const n1b = findViolations({
    file: "src/components/multi-agent/MultiAgentDashboard.tsx",
    content: 'import { A } from "@/stores/feature/multiAgentStore";\n',
    owner: "multi-agent",
  });
  check(n1b.length === 0, "归一正控④a：multi-agent 引 multiAgentStore 同形消除");
  const n1c = findViolations({
    file: "src/components/llm-wiki/X.tsx",
    content: 'import { A } from "@/stores/feature/llmWikiStore";\nimport { B } from "@/components/llmWiki/Y";\n',
    owner: "llm-wiki",
  });
  check(n1c.length === 0, "归一正控④b：第三个碰撞组 llm-wiki/llmWiki 同形消除（store 与 component 两支都归一）");

  // ② 负控：归一≠什么都不报。真跨域必须仍红
  // #222 补洞扩登后 provider 已横切（读面跨域），不能再用它当「未登记跨域」反例 ⇒ 换 plan
  const n2 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { usePlanStore } from "@/stores/feature/planStore";\n',
    owner: "stock-analysis",
  });
  check(n2.length === 1 && n2[0].kind === "store" && n2[0].targetOwner === "plan",
    "归一负控②：stock-analysis 引 planStore 仍必须红");
  const n2b = findViolations({
    file: "src/components/multi-agent/X.tsx",
    content: 'import { A } from "@/stores/feature/stockAnalysisStore";\n',
    owner: "multi-agent",
  });
  check(n2b.length === 1, "归一负控④c：multi-agent 引 stockAnalysisStore 仍红（不同 owner 不许被归一合并）");
  const n2c = findViolations({
    file: "src/components/stock-analysis/panels/X.tsx",
    content: 'import { A } from "@/components/stock-workspace/Sidebar";\n',
    owner: "stock-analysis",
  });
  check(n2c.length === 1 && n2c[0].kind === "component",
    "归一负控：两个 kebab 目录（stock-analysis vs stock-workspace）归一后仍不等 ⇒ 仍红");

  // ③ NEUTRAL 支（精确匹配，#200）与 owner 支（归一，#168）两套判据**各自**钉死
  const n3 = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { Tooltip } from "@/components/layout/Tooltip";\nimport { C } from "@/components/common/PageHeader";\n',
    owner: "dynamicUI",
  });
  check(n3.length === 0, "NEUTRAL 正控③a：dynamicUI（camel 目录）引登记名 layout/common ⇒ 豁免");
  const n3b = findViolations({
    file: "src/components/dynamicUI/Renderer.tsx",
    content: 'import { A } from "@/components/chat/Bubble";\n',
    owner: "dynamicUI",
  });
  check(n3b.length === 1, "NEUTRAL 负控③b：dynamicUI 引未登记域（chat）仍红，中性豁免面不许靠命名形态自动扩");
  // ③c 登记面**原形**必须仍被豁免（收窄 NEUTRAL 支后仍如此 —— 这条是「精确匹配没把 4 个登记名判漏」的正控）
  const n3c = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamicUI/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3c.length === 0, "NEUTRAL 正控③c：kebab owner 引登记名原形 dynamicUI 仍豁免（登记表逐字命中即可）");
  // ③d 负控（**#200 判据反转**：#168 批 A 曾把这条钉成正控「dynamic-ui 也算中性」）：
  //   `dynamic-ui` **不在** NEUTRAL_PREFIXES 登记表里 ⇒ 必须报违规。中性目录的扩面是人的登记决定
  //   （AGENTS.md「组件中立性规范」：新增必须登记 + 写明理由，不许静默扩），不许由命名形态推导出来。
  //   真实数据影响为 0：现存 4 个登记名没有任何 kebab/snake 变体出现在 import 图里，
  //   故本条从正控翻成负控时判定档读数逐字不变（2026-11-05 复量：202 / 199 / PASS 基线 212）。
  const n3d = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { R } from "@/components/dynamic-ui/DynamicUIRenderer";\n',
    owner: "stock-analysis",
  });
  check(n3d.length === 1 && n3d[0].targetOwner === "dynamic-ui",
    "NEUTRAL 负控③d：未登记的 kebab 变体 dynamic-ui **必须**报违规 ⇒ 中性扩面只能改 NEUTRAL_PREFIXES 登记表，改名白拿豁免即红");
  // ③e 正控（#200 补，与 ③d 互为犄角）：owner 两支**仍归一**不许被「一起退回全等」。
  //   同一文件里同时命中 store 支（stockAnalysisStore）与 component 支（camel 目录 stockAnalysis）的
  //   同 owner 两种写法（n1 只钉 store 支、n1c 只钉 llm-wiki 组），下一次有人把三支一起改成全等 ⇒ 本条立刻红
  //   （③d 只守 NEUTRAL 那一支，守不了这两支）。
  const n3e = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { A } from "@/stores/feature/stockAnalysisStore";\nimport { B } from "@/components/stockAnalysis/Panel";\n',
    owner: "stock-analysis",
  });
  check(n3e.length === 0,
    "owner 支仍归一③e（正控）：kebab 目录 stock-analysis 引同 owner 的 camel store 支 + camel 组件支两支都豁免 ⇒ 三支一起退回全等即红");

  // ⑥ 空洞通过防护（#199）的判据本身 —— 注入数字做正负控，不碰真实树
  //   （真实树上「927 ≥ 300」是永真断言，扫描器瞎掉时也不会变，钉不住退化）。
  check(scanFloorFailures(927, 157).length === 0,
    "空洞防护正控：真实树实测形状（扫描 927 file / 基线 157 个不同 file）必须放行");
  check(scanFloorFailures(MIN_SCANNED_FILES, 0).length === 0,
    "空洞防护正控：恰好落在下界 + 基线为空（b 退化）⇒ 放行，证明下界不含越界强度");
  check(scanFloorFailures(0, 0).length === 1,
    "空洞防护负控 a：扫描 0 file 且基线为空 ⇒ 仍红（条件 a 兜住 b 退化成 >= 0 的那一支）");
  check(scanFloorFailures(MIN_SCANNED_FILES - 1, 0).length === 1,
    "空洞防护负控 a′：扫描数恰为下界-1 ⇒ 红（边界钉死在「< 下界」，不许松成「只剩一个子目录也算正常」）");
  const floorB = scanFloorFailures(1000, 1001);
  check(floorB.length === 1 && floorB[0].includes("条件 b"),
    "空洞防护负控 b：条件 a 已过（1000 ≥ 300）但基线面 1001 > 扫描面 1000 ⇒ 红，且报的是条件 b ⇒ 两条锚各自独立生效");

  // 未合并组自证：gatewayLink（store 剥后缀名）不许与任一目录名归一成同名。
  // #222 补洞扩登后 link 域已登记 gatewaylink（link→gatewayLinkStore 豁免），
  // 原反例组合失效 ⇒ 换同样未登记 gatewaylink 的 multi-agent，钉住的是归一逻辑不是登记面
  const n4 = findViolations({
    file: "src/components/multi-agent/X.tsx",
    content: 'import { A } from "@/stores/feature/gatewayLinkStore";\n',
    owner: "multi-agent",
  });
  check(n4.length === 1, "归一负控：multi-agent 引 gatewayLinkStore 仍红 ⇒ 归一未把 gatewayLink 与目录名合并");

  // ⑤ 键形态钉住：棘轮键 = file + " → " + import 字面量原文；归一不许渗进 target/targetOwner
  const k1 = findViolations({
    file: "src/components/stock-analysis/BacktestPage.tsx",
    content: 'import { usePlanStore } from "@/stores/feature/planStore";\n',
    owner: "stock-analysis",
  });
  check(k1.length === 1 && k1[0].target === "@/stores/feature/planStore" && k1[0].targetOwner === "plan" &&
      k1[0].resolvedTarget === "src/stores/feature/planStore", "键形态：target/targetOwner 存原形，未归一");
      // 注：原用 providerStore 当反例 —— #222 扩登后 provider 横切 ⇒ 绿，换未登记的 plan 钉同一键形态
  const k2 = findViolations({
    file: "src/components/multi-agent/X.tsx",
    content: 'import { A } from "@/components/stock-analysis/Panel";\n',
    owner: "multi-agent",
  });
  check(k2.length === 1 && k2[0].target === "@/components/stock-analysis/Panel" && k2[0].targetOwner === "stock-analysis",
    "键形态：kebab 形 target/targetOwner 同样存原文 ⇒ 212 条基线键不会因归一失配");

  // ── C 档域归属映射表（DOMAIN_STORE_OWNERSHIP / isSameDomain）正负对照 ──
  // ① 正控：登记映射对（stock-analysis→serenityStore）视同自引，不报违规
  const d1 = findViolations({
    file: "src/components/stock-analysis/SerenityScreeningPanel.tsx",
    content: 'import { useSerenityStore } from "@/stores/feature/serenityStore";\n',
    owner: "stock-analysis",
  });
  check(d1.length === 0, "C 档正控①：stock-analysis 引登记自有 store serenityStore 不得假阳性");
  // ② 负控：未登记的跨域（chat→planStore）仍红 —— 映射不是「豁免一切跨域」。
  //   注：不能用 providerStore 当反例 —— provider 已横切登记（#222 扩登）⇒ 绿；plan 未登记。
  const d2 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { usePlanStore } from "@/stores/feature/planStore";\n',
    owner: "chat",
  });
  check(d2.length === 1 && d2[0].kind === "store" && d2[0].targetOwner === "plan",
    "C 档负控②：chat 引未登记跨域 planStore 仍必须红（映射只识别本域自有 store，不豁免跨域）");
  // ③ 负控：映射不许泛化 —— workflowEditor 登记在 workflow 域，stock-analysis 引它仍红
  const d3 = findViolations({
    file: "src/components/stock-analysis/X.tsx",
    content: 'import { useWorkflowEditorStore } from "@/stores/feature/workflowEditorStore";\n',
    owner: "stock-analysis",
  });
  check(d3.length === 1 && d3[0].targetOwner === "workflowEditor",
    "C 档负控③：stock-analysis 引未登记到本域的 workflowEditorStore 仍红 ⇒ 映射精确命中，不许跨域泛化");
  // ④ 负控：单向登记 —— 反向不成立（serenity 无真实组件目录，用注入 owner 钉死纯函数语义）：
  //   stock-analysis→serenity 登记了，不代表 serenity→stockAnalysis 自动豁免。
  const d4 = findViolations({
    file: "src/components/serenity/X.tsx",
    content: 'import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";\n',
    owner: "serenity",
  });
  check(d4.length === 1 && d4[0].targetOwner === "stockAnalysis",
    "C 档负控④：serenity 引 stockAnalysisStore 仍红 ⇒ 映射是单向登记，反向不自动豁免");
  // ⑤ 正控：isSameDomain 纯函数收原形名内部归一（供 #222 barrel 复用），且单向
  check(isSameDomain("stock-analysis", "serenity") && isSameDomain("stock-analysis", "Serenity") &&
      isSameDomain("workflow", "workflowEditor") && !isSameDomain("serenity", "stockAnalysis"),
    "C 档正控⑤：isSameDomain 归一原形名（kebab 目录 / camel store）精确命中，反向 false");
  // ⑥ 负控：映射只作用 store 支 —— theme 登记为 settings 自有 store，但 settings 引 theme **组件目录**仍红
  //   （component 支不查映射，否则与 store 同名的组件目录白拿豁免）。
  const d6 = findViolations({
    file: "src/components/settings/X.tsx",
    content: 'import { T } from "@/components/theme/ThemePanel";\n',
    owner: "settings",
  });
  check(d6.length === 1 && d6[0].kind === "component" && d6[0].targetOwner === "theme",
    "C 档负控⑥：settings 引 theme 组件目录仍红 ⇒ 域归属映射只作用 store 支，不渗进 component 支");

  // ── #222 barrel 改判正负控（2026-11-08）──
  // 手造映射表（不读盘，保持 selftest 纯函数）：覆盖 type 修饰 / as 改名两种行形。
  setStoreBarrelMap(buildStoreBarrelMap([
    'export { useVoiceStore } from "./feature/voiceStore";',
    'export { useAppConfigStore } from "./feature/appConfigStore";',
    'export { useThemeStore } from "./feature/themeStore";',
    'export { type BuddyInfo, useBuddyStore } from "./feature/buddyStore";',
    'export { useCurrencyStore as useMoneyStore } from "./feature/currencyStore";',
    'export { usePlanStore } from "./feature/planStore";',
  ].join("\n")));
  // ① buildStoreBarrelMap：type 修饰跳过 / as 记对外名 / 值导出全收
  const bmap = storeBarrelMap;
  check(bmap.get("useVoiceStore") === "src/stores/feature/voiceStore" &&
      bmap.get("useMoneyStore") === "src/stores/feature/currencyStore" &&
      bmap.get("BuddyInfo") === undefined && bmap.get("useBuddyStore") === "src/stores/feature/buddyStore" &&
      bmap.size === 6,
    "#222 正控①：barrel 映射 type 修饰跳过 / as 记对外名 useMoneyStore / 值导出全收（size 6）");
  // ② 正控：chat 经 barrel 引未登记 voice store ⇒ 红，target 存原形 "@/stores"，resolvedTarget 为深路径
  const b1 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { useVoiceStore } from "@/stores";\n',
    owner: "chat",
  });
  check(b1.length === 1 && b1[0].kind === "store" && b1[0].targetOwner === "voice" &&
      b1[0].target === "@/stores" && b1[0].resolvedTarget === "src/stores/feature/voiceStore",
    "#222 正控②：chat 经 barrel 引 voiceStore 红且 target 存原形 '@/stores'（棘轮键不受改判影响）");
  // ③ 负控：横切 store 经 barrel 同豁免（判定链与深路径共用，无额外豁免也无额外歧视）
  const b2 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { useAppConfigStore } from "@/stores";\n',
    owner: "chat",
  });
  check(b2.length === 0, "#222 负控③：横切 appconfig 经 barrel 同豁免");
  // ④ 负控：域自有登记对经 barrel 同豁免（settings → theme）
  const b3 = findViolations({
    file: "src/components/settings/X.tsx",
    content: 'import { useThemeStore } from "@/stores";\n',
    owner: "settings",
  });
  check(b3.length === 0, "#222 负控④：settings→theme 登记对经 barrel 同豁免");
  // ⑤ 正控：混合语句 { type X, useY } 只判值符号 useVoiceStore 一条（type 符号无运行时依赖）。
  //   注：buddy 不能用 —— chat 域登记了 buddy 自有 ⇒ 绿；voice 未登记给 chat ⇒ 红。
  const b4 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import {\n  type BuddyInfo,\n  useVoiceStore,\n} from "@/stores";\n',
    owner: "chat",
  });
  check(b4.length === 1 && b4[0].targetOwner === "voice",
    "#222 正控⑤：多行混合语句只判值符号 voice 一条（type 符号跳过、无重复计数）");
  // ⑥ 负控：type-only 整条语句不判（无运行时依赖）
  const b5 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import type { useVoiceStore } from "@/stores";\n',
    owner: "chat",
  });
  check(b5.length === 0, "#222 负控⑥：type-only import 不判");
  // ⑦ 负控：符号不在映射（非 store 工具函数）⇒ 无 store 语义跳过
  const b6 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { formatCny } from "@/stores";\n',
    owner: "chat",
  });
  check(b6.length === 0, "#222 负控⑦：映射外符号（工具函数）跳过");
  // ⑧ 正控：连续两条 import（深路径 + barrel）互不干扰，起点 lastIndexOf 不错位。
  //   两条都引 voice（chat 未登记 ⇒ 各红一条），target 一深一 barrel。
  const b7 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { useVoiceStore } from "@/stores/feature/voiceStore";\n' +
      'import { useVoiceStore } from "@/stores";\n',
    owner: "chat",
  });
  check(b7.length === 2 &&
      b7.some((v) => v.target === "@/stores/feature/voiceStore") &&
      b7.some((v) => v.target === "@/stores"),
    "#222 正控⑧：深路径 + barrel 连续两条各抓一条，语句起点不错位");
  // ⑨ as 改名的导入侧取 as 前原始名（= 模块导出名，即 map key）——plan 未登记给 chat ⇒ 红。
  //   注：不能用 currency 当反例 —— currency 已横切登记 ⇒ 绿；plan 仍是要抓的跨域。
  const b8 = findViolations({
    file: "src/components/chat/X.tsx",
    content: 'import { usePlanStore as plan } from "@/stores";\n',
    owner: "chat",
  });
  check(b8.length === 1 && b8[0].targetOwner === "plan",
    "#222 正控⑨：导入侧 as 前原始名 usePlanStore 命中映射 ⇒ plan 跨域红");

  if (fail > 0) { return false; }
  console.log(
    `SELFTEST OK（共 ${total} 条断言 · 失败 ${fail}）：` +
      "resolve（@/ 与相对路径）/ 自引剥后缀 / 跨 feature 双向负控 / " +
      "owner 两支归一正控 3 组同 owner（stockAnalysis·multiAgent·llmWiki）+ ③e 双支合钉 / " +
      "owner 归一负控（planStore·跨 kebab·multiAgent×stockAnalysis·gatewayLink 未合并）/ " +
      "NEUTRAL 精确匹配（③c 登记名豁免·③d 未登记变体必须红）/ norm 不剥后缀 / " +
      "棘轮键存原形 / 空洞通过防护双锚（a 绝对下界·b 相对基线面）/ " +
      "C 档域归属映射（①登记对豁免·②未登记跨域仍红·③不泛化·④单向·⑤isSameDomain 归一·⑥仅 store 支）+ " +
      "#222 barrel 改判（①映射表 type/as 形态·②原形 target+深路径 resolved·③横切同豁免·④登记对同豁免·" +
      "⑤混合语句只判值符号·⑥type-only 不判·⑦映射外跳过·⑧连续 import 不错位·⑨as 前原始名命中）全部通过"
  );
  return true;
}

// ── main ──
if (SELFTEST) { process.exit(runSelftest() ? 0 : 1); }

// 库模式：对账/普查脚本 import 本文件的导出函数时设 NEUTRALITY_LIB=1，跳过 main ——
// main 顶层会 process.exit（判定红即退 1），import 方还没来得及用导出函数就被杀。
async function runMain() {

if (!existsSync(SRC_COMPONENTS)) {
  console.error(`FAIL: src/components 不存在（应含本仓全部组件目录）—— 本轮对组件中立性**零验证**，` +
    `「没读到」不等于「没有违规」，门禁按红处理。`);
  console.error(`  两种可能，都必须有人当场判：① 目录被改名/搬迁 ⇒ 同步本文件的 SRC_COMPONENTS 并复核扫描数；` +
    `② 构建/裁剪产物 ⇒ 门禁不该在无源码的树里跑（此时 CI 该 skip 整个 step，而不是让它报绿）。`);
  process.exit(1);
}

// ── #222 barrel 改判的前置：读 src/stores/index.ts 构建「符号 → 真实 store 路径」映射 ──
// 缺失即 FAIL（与 SRC_COMPONENTS 同一口径）：barrel 改判依赖该表，读不到 ⇒ 246 处 barrel
// 导入全部漏检 ⇒ 「没读到」不等于「没有违规」。
const STORES_INDEX = join(root, "src/stores/index.ts");
if (!existsSync(STORES_INDEX)) {
  console.error(`FAIL: src/stores/index.ts 不存在 —— #222 barrel 改判失去映射来源，` +
    `组件里 from "@/stores" 的导入将整批漏检。若 barrel 被改名/拆除（#190 分批计划），` +
    `同步本文件的 STORES_INDEX 并复核 barrel 改判断言。`);
  process.exit(1);
}
setStoreBarrelMap(buildStoreBarrelMap(readFileSync(STORES_INDEX, "utf8")));

const allowlist = existsSync(ALLOWLIST_PATH)
  ? JSON.parse(readFileSync(ALLOWLIST_PATH, "utf8"))
  : { fileExemptions: [], targetPrefixExemptions: [] };
const fileExemptMap = new Map();
for (const fe of allowlist.fileExemptions ?? []) {
  fileExemptMap.set(fe.file, new Set(fe.targets ?? []));
}
const prefixExemptions = allowlist.targetPrefixExemptions ?? [];

const files = collectFiles(SRC_COMPONENTS, []).map((p) => relative(root, p).split("\\").join("/"));
const allViolations = [];
for (const rel of files) {
  const owner = ownerOf(rel);
  const content = readFileSync(join(root, rel), "utf8");
  for (const v of findViolations({ file: rel, content, owner })) { allViolations.push(v); }
}

// 白名单过滤
const effective = allViolations.filter((v) => {
  const exemptTargets = fileExemptMap.get(v.file);
  if (exemptTargets?.has(v.target)) { return false; }
  if (prefixExemptions.some((pe) => v.target.startsWith(pe.prefix))) { return false; }
  return true;
});

console.log(`\n=== 组件中立性检查 ===`);
console.log(`扫描 ${files.length} 文件 · 原始跨域引用 ${allViolations.length} · 白名单豁免后 ${effective.length}`);

// 读取棘轮基线（下面的空洞通过防护的条件 b 要用它的 file 面积，故先于比对读）
let baseline = { entries: [] };
if (existsSync(BASELINE_PATH)) {
  try { baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8")); } catch { /* 损坏按空处理 —— 由下面的条件 a 兜住 */ }
}

// ── 空洞通过防护（#199）：必须早于棘轮比对 / PASS 输出 / --update-baseline 写入 ──
// 扫描面塌方时 effective 会是空集，空集 ⊆ 任何基线 ⇒ 后面所有分支都会「正确地」报绿；
// 更糟的是 --update-baseline 会照抄这个空集，把 212 条棘轮一次洗成 0 条。所以在这里就叫停。
const scannedFileCount = new Set(files).size;
const baselineFileCount = new Set((baseline.entries ?? []).map((e) => e.file)).size;
const floorProblems = scanFloorFailures(scannedFileCount, baselineFileCount);
if (floorProblems.length > 0) {
  console.error(`\nFAIL: 扫描面塌方 —— 「0 条新违规」不等于「没有违规」，本轮判定不作数：`);
  for (const p of floorProblems) { console.error(`  · ${p}`); }
  console.error(`  先恢复被扫描的源码面 / 修 collectFiles 口径；**禁止**用 --update-baseline 把空集写进基线洗账面。`);
  process.exit(1);
}
// 棘轮比对
const baseKeys = new Set((baseline.entries ?? []).map((e) => e.file + " → " + e.target));
const curKeys = new Set(effective.map((v) => v.file + " → " + v.target));
const newViolations = effective.filter((v) => !baseKeys.has(v.file + " → " + v.target));
const fixedCount = [...baseKeys].filter((k) => !curKeys.has(k)).length;

if (UPDATE_BASELINE) {
  // 首次建立（基线文件不存在）不受「新违规」保护 —— 没有基线可比，棘轮从当前状态起算；
  // 后续收紧才受保护：存在基线外新违规时拒绝，防止用 update 洗白新增耦合。
  const isFirstEstablish = !existsSync(BASELINE_PATH);
  if (!isFirstEstablish && newViolations.length > 0) {
    console.error(`\nFAIL: --update-baseline 拒绝执行：当前存在 ${newViolations.length} 条基线外新违规，先修或显式登记白名单：`);
    for (const v of newViolations.slice(0, 20)) { console.error(`  + ${v.file} → ${v.target}`); }
    process.exit(1);
  }
  const fsSync = await import("node:fs");
  // 写入前按 file+target 去重：同一文件多行 import 同一 target 会产生重复条目
  const seen = new Set();
  const deduped = effective.filter((v) => {
    const k = v.file + " → " + v.target;
    if (seen.has(k)) { return false; }
    seen.add(k);
    return true;
  });
  fsSync.writeFileSync(
    BASELINE_PATH,
    JSON.stringify({
      note: "棘轮基线：键 = file + target（不含行号，防漂移）。只许减少不许增加；新增违规必须当场修或登记白名单。NEUTRAL_PREFIXES 见 check-component-neutrality.mjs 头部。",
      entries: deduped.map((v) => ({ file: v.file, target: v.target, kind: v.kind, targetOwner: v.targetOwner })),
    }, null, 2) + "\n",
    "utf8",
  );
  console.log(`\n基线已更新：${deduped.length} 条唯一键（本轮已修复 ${fixedCount > 0 ? `较上版减少 ${fixedCount}` : "首次建立"}）。`);
  process.exit(0);
}

if (newViolations.length > 0) {
  console.error(`\nFAIL: ${newViolations.length} 条基线外新违规（修掉、下沉或登记白名单；不许改基线洗白）：`);
  for (const v of newViolations.slice(0, 30)) {
    console.error(`  + ${v.file} → ${v.target}（${v.kind}:${v.targetOwner}）`);
  }
  if (newViolations.length > 30) { console.error(`  … 其余 ${newViolations.length - 30} 条省略`); }
  console.error(`\n若确属误报：新中性目录 ⇒ 在 NEUTRAL_PREFIXES 登记；面板读本域自有 store ⇒ 在 DOMAIN_STORE_OWNERSHIP 登记（键=组件域、值=store 名，均 norm 后）并写明理由 + 证据文件。两者都在 scripts/check-component-neutrality.mjs，不要静默扩豁免。`);
  process.exit(1);
}

console.log(`PASS: 无新增违规（基线 ${baseKeys.size} 条，本轮已修复 ${fixedCount} 条可择机 --update-baseline 收紧）。`);
process.exit(0);
}

if (process.env.NEUTRALITY_LIB !== "1") {
  runMain().catch((e) => { console.error(e); process.exit(1); });
}
