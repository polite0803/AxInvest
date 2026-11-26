#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 把一个插件 worker 打包成「可投放的插件目录」：`<out>/plugin.json` + `<out>/bin/<worker>`。
 *
 * 为什么要这个脚本（PLAN §74 未做 1）：宿主的 `PluginWorkerDecl.program` 只接受
 * **相对插件安装目录**的路径，而 `cargo build` 的产物在 target 目录 ⇒ 中间缺一步
 * 「把产物摆进插件目录」。本脚本就是那一步，产物落在 git 忽略的 `dist/plugins/<插件>/`。
 *
 * 用法（在 axagent-plugins 根下执行；主仓侧入口是 `npm run plugin:bundle`）：
 *   node scripts/build-plugin-bundle.mjs                 # 构建（debug）+ 打包 + 打印路径（缺省 stock）
 *   node scripts/build-plugin-bundle.mjs --plugin opc     # 打包 opc 插件（#36 骨架）
 *   node scripts/build-plugin-bundle.mjs --release       # 构建 release
 *   node scripts/build-plugin-bundle.mjs --target <triple> # 指定 Cargo target 构建
 *   node scripts/build-plugin-bundle.mjs --plugin-version <版本> # 显式发布版本（声明式插件必需）
 *   node scripts/build-plugin-bundle.mjs --check         # 打包后真起一次 worker 做握手 + 停机回执
 *   node scripts/build-plugin-bundle.mjs --out <dir>     # 自定义输出目录
 *   node scripts/build-plugin-bundle.mjs --build-root <dir> # 只切换 cargo 工作区（CI 临时构建根）
 *
 * 参数化口径（任务 #36 第 1 批）：**一个插件 = `PLUGIN_SPECS` 里一条**，条目的字段就是
 * 本脚本对插件的全部假设（cargo 包名、前端源目录、`ui.json` 源、清单构造器）。新增插件只加
 * 一条，不改脚本主体。⚠ **`--plugin stock` 的产物（路径 / 清单字段与键序 / 产物名 / sha256 /
 * 控制台行）必须与参数化之前逐字一致** —— 下游 `stage-plugin-bundles.mjs` 与既有插件 e2e
 * 都按那份形状断言，静默失配不会有任何一处报错。
 *
 * `--check` 不造假：它 spawn 的是**打包出来的那个文件**，按 `axagent-plugin-proto`
 * 的长度前缀 JSON 帧发 `describe` / `shutdown`，并核对 integrity 的 sha256 与盘上一致。
 */
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

// 本脚本权威位置在 axagent-plugins/scripts/（随 SDK 仓走，#281 判据③缺口③）：SDK 目录拷出后
// 没有 axagent 主仓根，workspaceRoot 必须从脚本自身位置锚定，不得假设主仓布局。
const workspaceRoot = path.resolve(import.meta.dirname, "..");
const argv = process.argv.slice(2);
const release = argv.includes("--release");
const check = argv.includes("--check");
const outFlag = argv.indexOf("--out");
const pluginFlag = argv.indexOf("--plugin");
const targetFlag = argv.indexOf("--target");
const pluginVersionFlag = argv.indexOf("--plugin-version");
const buildRootFlag = argv.indexOf("--build-root");
const pluginName = pluginFlag >= 0 && argv[pluginFlag + 1] ? argv[pluginFlag + 1] : "stock";
const targetTriple = targetFlag >= 0 && argv[targetFlag + 1] ? argv[targetFlag + 1] : null;
// `--build-root` 只切换 cargo 的构建工作区，不切换前端/域包/清单的源码根。
// 缺省构建根 = 该插件对应的独立 Git 仓（唯一权威源）；`--build-root` 保留给
// 需要换 target 目录或跨机交叉编译的调用方，缺省路径不再回退主仓副本。
const cargoCwd = buildRootFlag >= 0 && argv[buildRootFlag + 1]
  ? path.resolve(argv[buildRootFlag + 1])
  : requirePluginSourceRoot(pluginName);
const requestedPluginVersion = pluginVersionFlag >= 0 && argv[pluginVersionFlag + 1]
  ? argv[pluginVersionFlag + 1]
  : null;

/**
 * 插件规格表（键 = `--plugin` 取值）。
 *
 * - `packageName`：cargo 包名，同时决定 `cargo build -p` 与产物文件名（`<packageName>[.exe]`）；
 * - `webDir`：前端贡献源目录（相对**插件源码根** —— 独立仓根，或打包器自带副本时的本仓根）；
 *   `null` = 无前端贡献 ⇒ 不拷资产、且清单不得声明 `pages[]`（`--check` 会拦）；
 * - `uiSchema`：`ui.json` 源（同上，相对插件源码根）；`null` = 无详情槽声明；
 * - `manifest(ctx)`：清单构造器，`ctx = { version, binaryName, sha256 }`。**键序即产物键序**
 *   （`JSON.stringify` 保序），改键序等于改产物，下游比对会当漂移。
 */
/**
 * OPC 域包页面的侧栏元数据（唯一声明面）。
 *
 * ⚠ 这里的 navGroup / labelKey / icon / navOrder 是宿主侧栏投影的权威来源；
 * 前端 Sidebar / CommandPalette 只做渲染，不得再按插件名写死分组或图标（禁区 12）。
 * labelKey 复用既有 `nav.*`（11 语言已齐），不新增文案。
 *
 * 映射来源：git 7b43b5f92^:src/pages/opc/routes.tsx（14 个行业包的编译期声明被删前的原形），
 * 加上 opc-accounting（会计，独立清单工厂）与 opc 公共包（探针页，hidden）。
 *
 * ⚠ 本表必须定义在 `PLUGIN_SPECS` 之前：打包主流程是**模块顶层顺序执行**（§356 取 spec →
 * §594 调 `spec.manifest(...)` → manifest 工厂闭包调 `opcDomainPageDecl` 引用本表）。
 * 函数声明有提升、const 没有 —— 本表曾放在文件尾部的声明区（PLUGIN_SPECS 之后 5700+ 行），
 * 顶层 594 行先于 5827 行执行 ⇒ `Cannot access 'OPC_DOMAIN_PAGE_NAV' before initialization`
 * （Release run 45721938652 的 macOS ARM 构建即崩于此；stock 不走 OPC 域包路径，
 * 本地 `--plugin stock` 验证一直没暴露）。
 */
const OPC_DOMAIN_PAGE_NAV = {
  "opc-finance-invest":     { navGroup: "packs", labelKey: "nav.financeAnalysis",      icon: "TrendingUp",    navOrder: 0 },
  "opc-accounting":         { navGroup: "packs", labelKey: "nav.financeAccounting",    icon: "Calculator",    navOrder: 1 },
  "opc-sales-growth":       { navGroup: "packs", labelKey: "nav.automationSales",      icon: "TrendingUp",    navOrder: 2 },
  "opc-project-management": { navGroup: "packs", labelKey: "nav.automationProjects",   icon: "Layers",        navOrder: 3 },
  "opc-consulting":         { navGroup: "packs", labelKey: "nav.automationConsulting", icon: "Users",         navOrder: 4 },
  "opc-ecommerce":          { navGroup: "packs", labelKey: "nav.automationEcommerce",  icon: "ShoppingBag",   navOrder: 5 },
  "opc-software-dev":       { navGroup: "packs", labelKey: "nav.devopsSoftware",       icon: "Code",          navOrder: 6 },
  "opc-security":           { navGroup: "packs", labelKey: "nav.devopsSecurity",       icon: "ShieldCheck",   navOrder: 7 },
  "opc-geospatial":         { navGroup: "packs", labelKey: "nav.dataGeospatial",       icon: "MapPin",        navOrder: 8 },
  "opc-ai-research":        { navGroup: "packs", labelKey: "nav.dataAiResearch",       icon: "Brain",         navOrder: 9 },
  "opc-content-media":      { navGroup: "packs", labelKey: "nav.contentMedia",         icon: "Video",         navOrder: 10 },
  "opc-design":             { navGroup: "packs", labelKey: "nav.contentDesign",        icon: "Palette",       navOrder: 11 },
  "opc-education":          { navGroup: "packs", labelKey: "nav.contentEducation",     icon: "GraduationCap", navOrder: 12 },
  "opc-game-dev":           { navGroup: "packs", labelKey: "nav.aiMediaGame",          icon: "Gamepad2",      navOrder: 13 },
};

const PLUGIN_SPECS = {
  "opc-accounting": {
    packageName: "axagent-plugin-opc-accounting",
    webDir: path.join("plugin-opc-accounting", "web"),
    uiSchema: null,
    manifest: opcAccountingManifest,
  },
  "opc-security": {
    packageName: "axagent-plugin-opc-security",
    webDir: path.join("plugin-opc-security", "web"),
    uiSchema: null,
    commands: [
      "opc_security_opc_search_wiki",
      "opc_security_web_search",
      "opc_security_file_write",
      "opc_security_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-security", "安全合规", "security"),
  },
  // 统一插件规范试点：本域从声明式翻为 worker。命令名在此登记一次，清单工厂按
  // `PLUGIN_SPECS[name].commands` 派生 command 条目；契约 checker 逐项对账这份列表。
  "opc-game-dev": {
    packageName: "axagent-plugin-opc-game-dev",
    webDir: path.join("plugin-opc-game-dev", "web"),
    uiSchema: null,
    commands: [
      "opc_game_dev_web_search",
      "opc_game_dev_file_write",
      "opc_game_dev_file_read",
      "opc_game_dev_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-game-dev", "游戏开发", "game_dev"),
  },
  "opc-ai-research": {
    packageName: "axagent-plugin-opc-ai-research",
    webDir: path.join("plugin-opc-ai-research", "web"),
    uiSchema: null,
    commands: [
      "opc_ai_research_opc_list_projects",
      "opc_ai_research_opc_search_wiki",
      "opc_ai_research_web_search",
      "opc_ai_research_file_read",
      "opc_ai_research_bash",
      "opc_ai_research_file_write",
      "opc_ai_research_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-ai-research", "AI 研究", "ai_research"),
  },
  "opc-consulting": {
    packageName: "axagent-plugin-opc-consulting",
    webDir: path.join("plugin-opc-consulting", "web"),
    uiSchema: null,
    commands: [
      "opc_consulting_opc_search_wiki",
      "opc_consulting_web_search",
      "opc_consulting_opc_get_dashboard",
      "opc_consulting_opc_create_content_asset",
      "opc_consulting_file_write",
      "opc_consulting_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-consulting", "咨询", "consulting"),
  },
  "opc-design": {
    packageName: "axagent-plugin-opc-design",
    webDir: path.join("plugin-opc-design", "web"),
    uiSchema: null,
    commands: [
      "opc_design_web_search",
      "opc_design_file_write",
      "opc_design_opc_create_content_asset",
      "opc_design_opc_create_landing_page",
      "opc_design_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-design", "设计", "design"),
  },
  "opc-ecommerce": {
    packageName: "axagent-plugin-opc-ecommerce",
    webDir: path.join("plugin-opc-ecommerce", "web"),
    uiSchema: null,
    commands: [
      "opc_ecommerce_web_search",
      "opc_ecommerce_opc_search_wiki",
      "opc_ecommerce_opc_list_customers",
      "opc_ecommerce_opc_create_content_asset",
      "opc_ecommerce_opc_create_landing_page",
      "opc_ecommerce_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-ecommerce", "电商", "ecommerce"),
  },
  "opc-education": {
    packageName: "axagent-plugin-opc-education",
    webDir: path.join("plugin-opc-education", "web"),
    uiSchema: null,
    commands: [
      "opc_education_web_search",
      "opc_education_opc_create_content_asset",
      "opc_education_opc_create_landing_page",
      "opc_education_file_write",
      "opc_education_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-education", "教育", "education"),
  },
  "opc-finance-invest": {
    packageName: "axagent-plugin-opc-finance-invest",
    webDir: path.join("plugin-opc-finance-invest", "web"),
    uiSchema: null,
    commands: [
      "opc_finance_invest_opc_get_dashboard",
      "opc_finance_invest_opc_list_kpis",
      "opc_finance_invest_opc_search_wiki",
      "opc_finance_invest_opc_list_projects",
      "opc_finance_invest_opc_get_financial_report",
      "opc_finance_invest_opc_record_kpi",
      "opc_finance_invest_batch_validate_entities",
      "opc_finance_invest_opc_list_customers",
      "opc_finance_invest_web_search",
      "opc_finance_invest_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-finance-invest", "金融投资", "finance_invest"),
  },
  "opc-geospatial": {
    packageName: "axagent-plugin-opc-geospatial",
    webDir: path.join("plugin-opc-geospatial", "web"),
    uiSchema: null,
    commands: [
      "opc_geospatial_opc_search_wiki",
      "opc_geospatial_web_search",
      "opc_geospatial_opc_create_content_asset",
      "opc_geospatial_opc_create_project",
      "opc_geospatial_file_write",
      "opc_geospatial_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-geospatial", "地理信息", "geospatial"),
  },
  "opc-project-management": {
    packageName: "axagent-plugin-opc-project-management",
    webDir: path.join("plugin-opc-project-management", "web"),
    uiSchema: null,
    commands: [
      "opc_project_management_opc_create_project",
      "opc_project_management_opc_add_milestone",
      "opc_project_management_opc_list_projects",
      "opc_project_management_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-project-management", "项目管理", "project_management"),
  },
  "opc-software-dev": {
    packageName: "axagent-plugin-opc-software-dev",
    webDir: path.join("plugin-opc-software-dev", "web"),
    uiSchema: null,
    commands: [
      "opc_software_dev_opc_list_projects",
      "opc_software_dev_opc_create_project",
      "opc_software_dev_file_write",
      "opc_software_dev_web_search",
      "opc_software_dev_opc_add_milestone",
      "opc_software_dev_batch_validate_entities",
      "opc_software_dev_run_automation_rules",
    ],
    manifest: opcWorkerDomainManifest("opc-software-dev", "软件开发", "software_dev"),
  },
  "opc-sales-growth": {
    packageName: "axagent-plugin-opc-sales-growth",
    webDir: path.join("plugin-opc-sales-growth", "web"),
    uiSchema: null,
    manifest: opcSalesGrowthManifest,
  },
  "opc-content-media": {
    packageName: "axagent-plugin-opc-content-media",
    webDir: path.join("plugin-opc-content-media", "web"),
    uiSchema: null,
    manifest: opcContentMediaManifest,
  },
  // Wiki 业务功能包（P6.3 第一片）：只承接一条命令，经宿主 `wiki.store` 写接缝同步笔记。
  wiki: {
    packageName: "axagent-plugin-wiki",
    // 首批不含前端页面资产 ⇒ `webDir` 为 null（清单不得声明 `pages[]`）。
    webDir: null,
    uiSchema: null,
    manifest: wikiManifest,
  },
  stock: {
    packageName: "axagent-plugin-stock",
    // 前端贡献的源目录：`web/` 下的文件按**相对路径原样**进产物，`pages[].entry` 即这些相对路径。
    // 目前只有一页通道探针（B8-0）；真实股票 UI 随 B8-1..B8-5 才落进这里。
    webDir: path.join("plugin-stock", "web"),
    // UI 贡献声明（任务 #149 第 3 步）：`ui.json` 与 `plugin.json` **同级**落在安装根，
    // 任务源的 `detailSchema` 只能引用它里面已声明的 key —— 宿主 `build_manifest_task_sources`
    // 安装期硬校验，「声明了却不在 ui.json 里」等于装不上。故它是清单的一部分，必须由本脚本
    // 一起产出（前端资产目录 `web/` 只收 html/js/css，扩展名白名单会拒掉 .json）。
    uiSchema: path.join("plugin-stock", "ui.json"),
    manifest: stockManifest,
  },
  opc: {
    packageName: "axagent-plugin-opc",
    // 前端贡献源目录（#301 判据④闭合）：`web/` 下的文件按**相对路径原样**进产物，
    // 目前只有一页通道探针（pages/index.html，照 stock B8-0 先例）。OPC 的 16 个真实页面
    // 仍住主壳 src/pages/opc/**（路由声明已交域模块 routes.tsx），迁入本产物属后续批。
    webDir: path.join("plugin-opc", "web"),
    // OPC 无 ui.json 详情槽声明（没有任务源引用 OPC 的 detailSchema）⇒ 保持 null，
    // 不为「以后可能用」预留空文件。
    uiSchema: null,
    manifest: opcManifest,
  },
};

// 清单命令的智能体面授权档位（`requiredPermission`，见 `PluginCommandManifest`）：
// **未声明 = 只读档** ⇒ 写命令在智能体面按 `Safe` 暴露（`plugin_command_safety(None)`
// 的测试锁定），用户确认弹窗形同虚设。写命令名单以宿主
// 宿主 `crates/plugins/src/worker.rs` 的
// `required_permission_for(("opc.store", …)) => Write` 为准 —— **两处必须同批改**
// （那边加一条写 op，这里就要加一条命令）。
// ⚠ 删除类（delete_blog_post / delete_content_asset / delete_customer / delete_invoice）
// 的宿主等价命令是 `Dangerous`，但清单标 `danger-full-access` 会被
// `PluginManager::check_command_gate` 拒（该档要求插件声明 `execute`）；为不扩张插件
// 权限面，本表一律 `workspace-write`（= Caution）。升 Dangerous 需先裁定 execute 面
// （插件整体获得该声明），属安全面变更，另立评审。
const OPC_WRITE_COMMANDS = new Set([
  "opc_add_milestone",
  "opc_analyze_capability_gaps",
  "opc_cancel_publish_schedule",
  "opc_close_capability_gap",
  "opc_complete_milestone",
  "opc_create_blog_post",
  "opc_create_content_asset",
  "opc_create_customer",
  "opc_create_invoice",
  "opc_create_landing_page",
  "opc_create_project",
  "opc_create_publish_schedule",
  "opc_delete_blog_post",
  "opc_delete_content_asset",
  "opc_delete_customer",
  "opc_delete_invoice",
  "opc_delete_landing_page",
  "opc_delete_project",
  "opc_delete_subscription",
  // #264 批 7 刀 B2：动态工作流执行（写 op —— 会起引擎跑工作流）。
  "opc_execute_dynamic_workflow",
  // #264 批 7 刀 B2：导入人才库（写 op —— 批量落人才模板）。
  "opc_import_talent_library",
  "opc_mark_contact_read",
  "opc_process_due_schedules",
  "opc_proactive_evaluate_leads",
  "opc_publish_blog_post",
  "opc_publish_landing_page",
  // #264 批 7 刀 B2：记录 KPI（写 op）。
  "opc_record_kpi",
  "opc_save_subscription",
  // #264 批 7 刀 B2：同步员工到舰队（写 op —— 建 / 改舰队与成员）。
  "opc_sync_fleet",
  "opc_transition_invoice",
  "opc_update_content_asset",
  "opc_update_customer",
  // #264 批 7 刀 B2：更新发票（写 op）。
  "opc_update_invoice",
  "opc_update_project",
  // #264 批 7 刀 B2：更新发布计划（写 op）。
  "opc_update_publish_schedule",
  // #264 批 7 刀 B3：触发域包自动化规则（写 op —— 规则动作会改库）。
  "opc_run_automation_rules",
  // #264 批 7 刀 B3：订阅扫描落库 / 幂等建改扫描调度任务（写 op —— 都改库）。
  "opc_run_subscription_scan",
  "opc_ensure_demand_scan_job",
  // 刀 B3 收尾：自动学习（写 op —— 落 RL 经验并触发反思/进化）。
  "opc_auto_learn_workflow",
  // #264 刀 C 批 3：学习面六条写命令（反思 / 进化 / 自我改进 / 学习闭环 /
  // 手动 RL 优化 / 手动记录经验）；`opc_get_rl_stats` 是读命令 ⇒ 不进本表。
  "opc_reflect_on_workflow",
  "opc_evolve_workflow",
  "opc_run_self_improvement",
  "opc_trigger_capability_pack_learning",
  "opc_trigger_rl_optimization",
  "opc_record_rl_experience",
  // #264 批 4a：工作项六条写命令（建 / 认领 / 提交评审 / 升级阻塞 / 解除阻塞 /
  // 自改进循环）；`opc_kanban_board` 是读命令 ⇒ 不进本表。
  "opc_create_work_item",
  "opc_work_item_start",
  "opc_work_item_review",
  "opc_escalate_work_item",
  "opc_work_item_unblock",
  "run_self_improving_opc_work_item",
  // #264 批 4b：域包与内容资产三条写命令（导出落盘 / 导入解包注册 / 启停写库）；
  // `opc_get_content_asset` / `opc_market_list` 是读命令 ⇒ 不进本表。
  "opc_export_capability_pack",
  "opc_import_capability_pack",
  "opc_set_capability_pack_enabled",
]);

const spec = PLUGIN_SPECS[pluginName];
if (!spec) {
  fail(`未知插件 --plugin ${pluginName}（可选：${Object.keys(PLUGIN_SPECS).join(" | ")}）`);
}

/** 独立仓根（可用 `AXAGENT_PLUGIN_REPOS_ROOT` 覆盖）；主仓同级目录是默认落点。 */
function pluginReposRoot() {
  if (process.env.AXAGENT_PLUGIN_REPOS_ROOT) { return path.resolve(process.env.AXAGENT_PLUGIN_REPOS_ROOT); }
  // `workspaceRoot` 是 `axagent-plugins/`，独立仓与**主仓**同级 ⇒ 上跳两层。
  return path.resolve(workspaceRoot, "..", "..", "AxInvest-plugin-repos");
}

/**
 * 插件源码根：**独立仓是唯一权威**，主仓 `crates/plugin-*` 副本不参与打包。
 * 两种落点：
 *   ① 主仓 `axagent-plugins/scripts/` ⇒ `<reposRoot>/<plugin>`；
 *   ② 独立仓自带副本 `scripts/` ⇒ 本仓根（仓内已有 `plugin-<plugin>/`）。
 * 都不成立时返回 null，由调用方 fail-closed —— 绝不回退主仓副本。
 */
function resolvePluginSourceRoot(name) {
  const candidate = path.join(pluginReposRoot(), name);
  if (fs.existsSync(path.join(candidate, ".git"))) { return candidate; }
  if (fs.existsSync(path.join(workspaceRoot, `plugin-${name}`))) { return workspaceRoot; }
  return null;
}

function requirePluginSourceRoot(name) {
  const root = resolvePluginSourceRoot(name);
  if (!root) {
    fail(
      `插件源码根不可用：找不到独立仓 ${path.join(pluginReposRoot(), name)}`
        + "（独立仓是唯一权威，不退回主仓副本；可用 AXAGENT_PLUGIN_REPOS_ROOT 指定根）",
    );
  }
  return root;
}

const outRoot = path.resolve(
  outFlag >= 0 && argv[outFlag + 1] ? argv[outFlag + 1] : path.join(workspaceRoot, "dist", "plugins", pluginName),
);
// 与宿主 `PANEL_ASSET_ALLOWED_EXTS`（宿主 crates/plugins/src/manager.rs）同口径：
// 清单校验会拒掉白名单外的扩展名，产物自检必须在**打包期**就报出来，而不是等安装时才失败。
const PAGE_ASSET_ALLOWED_EXTS = new Set(["html", "htm", "js", "css"]);
const PAGE_ENTRY = "pages/index.html";
const webRoot = spec.webDir ? path.join(requirePluginSourceRoot(pluginName), spec.webDir) : null;
const uiSchemaSource = spec.uiSchema ? path.join(requirePluginSourceRoot(pluginName), spec.uiSchema) : null;
const registryPath = path.join(workspaceRoot, "scripts", "opc-domain-pack-plugins.json");
const ownershipRegistry = JSON.parse(fs.readFileSync(registryPath, "utf8"));
const domainPackSourceByPlugin = Object.fromEntries(
  ownershipRegistry.packs
    .filter((p) => p.pluginId !== "plugin-opc")
    .map((p) => {
      const owner = p.pluginId.replace(/^plugin-/, "");
      // 全局命令归属自检要读**每个**域插件的 logical manifest；源码权威都在独立仓，
      // 缺任一仓即 fail-closed（不退回主仓副本，也不静默跳过 —— 那会漏掉跨插件重名）。
      return [owner, path.join(requirePluginSourceRoot(owner), p.pluginId, "domain_packs")];
    }),
);
const domainPackSource = domainPackSourceByPlugin[pluginName] ?? null;
const ownedDomainPackIds = new Set(
  (ownershipRegistry.policy?.verticalSlices ?? [])
    .filter((slice) => slice.pluginId !== "plugin-opc")
    .map((slice) => slice.domainPackId),
);
const expectedDomainPackIds = pluginName === "opc"
  ? ownershipRegistry.packs.map((p) => p.id).filter((id) => !ownedDomainPackIds.has(id))
  : ownershipRegistry.packs
    .filter((p) => p.pluginId === `plugin-${pluginName}` && ownedDomainPackIds.has(p.id))
    .map((p) => p.id);
const DOMAIN_PACK_DIR = "domain_packs";
// 所有 OPC 域（声明式与 worker）都带静态展示区，页面自检不再按 manifestStrategy 分流；
// 是否额外要求 bundle.js，由本插件 PLUGIN_SPECS.commands 是否非空决定。
const opcDomainPackIdForPage = ownershipRegistry.policy?.verticalSlices?.find(
  (slice) => slice.pluginId === `plugin-${pluginName}`,
)?.domainPackId ?? null;

/** 各插件 logical manifest 的合法 owner：由底册 verticalSlices 唯一权威。 */
function domainPackOwnerFor(pluginName) {
  const slice = ownershipRegistry.policy?.verticalSlices?.find(
    (s) => s.pluginId === `plugin-${pluginName}`,
  );
  return slice?.transportOwner ?? "plugin-opc-common";
}

function fail(message) {
  console.error(`[plugin-bundle] ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", windowsHide: true, ...options });
  if (result.error) { fail(`启动 ${command} 失败：${result.error.message}`); }
  if (result.status !== 0) { fail(`${command} ${args.join(" ")} 退出码 ${result.status}`); }
  return result;
}

// 1) 构建 worker（宿主与打包用同一份源码，不另存产物）；声明式插件没有 worker。
let sha256 = null;
let binaryName = null;
let workerVersion = "1.0.0";
if (spec.packageName) {
  if (!fs.existsSync(path.join(cargoCwd, "Cargo.toml"))) {
    fail(`cargo 构建根缺少 Cargo.toml（--build-root）：${cargoCwd}`);
  }
  run(
    "cargo",
    [
      "build",
      "-p",
      spec.packageName,
      ...(release ? ["--release"] : []),
      ...(targetTriple ? ["--target", targetTriple] : []),
    ],
    { cwd: cargoCwd },
  );

  // 2) 从 cargo 自己的视角取 target 目录与版本号 —— 不猜 `target/` 的位置
  //    （本机把 CARGO_TARGET_DIR 指到了 `~/.cargo/target-axagent`）。
  const metadataRaw = spawnSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    cwd: cargoCwd,
    encoding: "utf8",
    windowsHide: true,
  });
  if (metadataRaw.status !== 0) {
    fail(`cargo metadata 退出码 ${metadataRaw.status}：${metadataRaw.stderr.trim()}`);
  }
  const metadata = JSON.parse(metadataRaw.stdout);
  const workerPackage = metadata.packages.find((p) => p.name === spec.packageName);
  if (!workerPackage) { fail(`cargo metadata 里找不到 ${spec.packageName}`); }
  if (requestedPluginVersion && requestedPluginVersion !== workerPackage.version) {
    fail(
      `--plugin-version=${requestedPluginVersion} 与 ${spec.packageName} 的 Cargo 版本 ${workerPackage.version} 不一致`,
    );
  }
  workerVersion = workerPackage.version;

  const targetDir = metadata.target_directory;
  const profileDir = release ? "release" : "debug";
  binaryName = process.platform === "win32" ? `${spec.packageName}.exe` : spec.packageName;
  const builtBinary = path.join(targetDir, ...(targetTriple ? [targetTriple] : []), profileDir, binaryName);
  if (!fs.existsSync(builtBinary)) { fail(`构建产物不存在：${builtBinary}`); }

  // 3) 摆放插件目录：worker 路径**必须**相对安装目录（宿主 resolve_worker_program 会拒绝
  //    绝对路径与 `..` 逃逸），所以这里用 `bin/<name>` 而不是复制到根。
  const binDir = path.join(outRoot, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const stagedBinary = path.join(binDir, binaryName);
  fs.copyFileSync(builtBinary, stagedBinary);
  if (release) {
    // 包体闸门：带 debug_info 的 worker 一个就有 6-7M，17 个把安装包从 80M 抬到 156M。
    // `file` 在部分 Windows 环境不存在，此时只提示不拦包（strip 已由 Cargo profile 保证）。
    const probe = spawnSync("file", [stagedBinary], { encoding: "utf8", windowsHide: true });
    if (probe.error || probe.status !== 0) {
      console.warn(
        `[bundle] 无法用 file 校验 ${binaryName} 的调试符号（跳过）：${probe.error ?? probe.stderr?.trim()}`,
      );
    } else if (/debug_info/.test(probe.stdout)) {
      fail(`${binaryName} 仍带 debug_info；请检查发起构建的 workspace 根的 [profile.release] strip 设置`);
    }
    const sizeMiB = (fs.statSync(stagedBinary).size / 1024 / 1024).toFixed(1);
    console.log(`[bundle] worker ${binaryName}: ${sizeMiB} MiB（release / stripped）`);
  }
  sha256 = createHash("sha256").update(fs.readFileSync(stagedBinary)).digest("hex");
} else if (requestedPluginVersion) {
  workerVersion = requestedPluginVersion;
}

// 3.5) 前端资产：源目录整树拷进安装根。
//      先按源目录的**顶层子目录**清一遍同名目录 —— 否则源里删掉的页面会作为孤儿留在产物里，
//      宿主按目录哈希钉锚也看不出「这份资产没有任何声明指向它」。
//      `webDir = null` 的插件（#36 骨架期的 opc）没有前端贡献 ⇒ 整段跳过，不建空目录。
let webFileCount = 0;
if (webRoot) {
  if (!fs.existsSync(webRoot)) { fail(`前端源目录不存在：${webRoot}`); }
  for (const top of fs.readdirSync(webRoot, { withFileTypes: true })) {
    if (top.isDirectory()) { fs.rmSync(path.join(outRoot, top.name), { recursive: true, force: true }); }
  }
  const webFiles = listFilesRecursive(webRoot);
  if (webFiles.length === 0) { fail(`前端源目录为空：${webRoot}`); }
  for (const file of webFiles) {
    const rel = path.relative(webRoot, file).split(path.sep).join("/");
    const ext = path.extname(rel).slice(1).toLowerCase();
    if (!PAGE_ASSET_ALLOWED_EXTS.has(ext)) {
      fail(`前端资产扩展名不在白名单（${[...PAGE_ASSET_ALLOWED_EXTS].join(", ")}）：${rel}`);
    }
    const target = path.join(outRoot, ...rel.split("/"));
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.copyFileSync(file, target);
  }
  if (!fs.existsSync(path.join(outRoot, ...PAGE_ENTRY.split("/")))) {
    fail(`清单要声明的页面资产不在源目录里：${path.join(webRoot, ...PAGE_ENTRY.split("/"))}`);
  }
  webFileCount = webFiles.length;
}

// 3.6) UI 声明：原样拷到安装根（不做任何改写）。key 集合在这里先解析一次 ——
//      `--check` 与清单里的 `detailSchema` 都要它，解析失败必须**在打包期**报错，
//      而不是等宿主安装时才说「ui.json 不合法」。无 `ui.json` 的插件整段跳过。
let uiSchemaKeyCount = 0;
if (uiSchemaSource) {
  if (!fs.existsSync(uiSchemaSource)) { fail(`UI 声明文件不存在：${uiSchemaSource}`); }
  const uiSchemaRaw = fs.readFileSync(uiSchemaSource, "utf8");
  let uiSchemaKeys;
  try {
    uiSchemaKeys = new Set((JSON.parse(uiSchemaRaw).schemas ?? []).map((s) => s.key));
  } catch (error) {
    fail(`UI 声明不是合法 JSON（${uiSchemaSource}）：${error.message}`);
  }
  fs.copyFileSync(uiSchemaSource, path.join(outRoot, "ui.json"));
  uiSchemaKeyCount = uiSchemaKeys.size;
}

// 3.7) OPC 域包 logical manifest：一域一插件边界随产物走。
//      当前 transport 仍由 plugin-opc-common 承接，所以这些文件不是独立 worker 清单；
//      但它们必须随包分发，供安装/运行期按 domainPackId 做 ACL 与启停判定。
let domainPackCount = 0;
if (domainPackSource) {
  if (!fs.existsSync(domainPackSource)) { fail(`域包清单目录不存在：${domainPackSource}`); }
  const target = path.join(outRoot, DOMAIN_PACK_DIR);
  fs.rmSync(target, { recursive: true, force: true });
  fs.mkdirSync(target, { recursive: true });
  const files = fs.readdirSync(domainPackSource).filter((f) => f.endsWith(".json")).sort();
  if (expectedDomainPackIds.length > 0 && files.length !== expectedDomainPackIds.length) {
    fail(`域包清单份数应为 ${expectedDomainPackIds.length}，实得 ${files.length}：${domainPackSource}`);
  }
  for (const file of files) {
    const raw = fs.readFileSync(path.join(domainPackSource, file), "utf8");
    try {
      JSON.parse(raw);
    } catch (error) {
      fail(`域包清单不是合法 JSON（${file}）：${error.message}`);
    }
    fs.writeFileSync(path.join(target, file), raw.endsWith("\n") ? raw : `${raw}\n`);
  }
  domainPackCount = files.length;
}

// 4) 清单：形状归 `PLUGIN_SPECS[*].manifest`，此处只递随产物变化的三个量。
const manifest = spec.manifest({ version: workerVersion, binaryName, sha256 });
const manifestPath = path.join(outRoot, "plugin.json");
fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`[plugin-bundle] 产物目录：${outRoot}`);
if (spec.packageName) { console.log(`[plugin-bundle] worker   ：${path.join(outRoot, "bin", binaryName)}`); }
if (webRoot) { console.log(`[plugin-bundle] 前端资产 ：${webFileCount} 个文件（页面入口 ${PAGE_ENTRY}）`); }
if (uiSchemaSource) { console.log(`[plugin-bundle] UI 声明  ：ui.json（${uiSchemaKeyCount} 个 schema key）`); }
if (domainPackSource) { console.log(`[plugin-bundle] 域包清单 ：${domainPackCount} 份（${DOMAIN_PACK_DIR}/）`); }
console.log(`[plugin-bundle] 清单     ：${manifestPath}`);
if (sha256) { console.log(`[plugin-bundle] sha256   ：${sha256}`); }

if (check) {
  checkPageVerifierIfPresent();
  await checkStagedBundle(manifest, outRoot);
}

/**
 * 真实页面 bundle 的静态验收入口。
 *
 * 只查「文件可加载 + manifest 有 bundle 页」不够：说明页也能通过。凡是带
 * `src-web/__verify__/verify-bundle.mjs` 的页面产物，都必须真跑到它通过；这样
 * worker 命令页退回占位文案时，CI 会红而不是静默绿。
 */
function checkPageVerifierIfPresent() {
  const verifier = path.join(
    requirePluginSourceRoot(pluginName),
    spec.webDir ?? "",
    "..",
    "src-web",
    "__verify__",
    "verify-bundle.mjs",
  );
  if (!spec.webDir || !fs.existsSync(verifier)) {
    return;
  }
  const result = spawnSync(process.execPath, [verifier], {
    cwd: path.dirname(path.dirname(verifier)),
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0) {
    fail(`页面 bundle 验收失败：${path.relative(workspaceRoot, verifier)}（exit ${result.status}）`);
  }
}

/** 起一次成品 worker：describe 必须回合法声明，shutdown 必须回执并以 0 退出。 */
async function checkStagedBundle(declared, root) {
  // 页面自检只在**声明了前端源**的插件上跑：无前端贡献的插件连 pages[] 都不许有，
  // 否则宿主会去解析一条不存在的资产 —— 那种清单装得上但页面永远打不开。
  if (webRoot) {
    checkDeclaredPages(declared, root);
  } else if ((declared.pages ?? []).length > 0) {
    fail("--check：本插件未声明前端源（webDir=null），清单不得声明 pages[]");
  }
  if (opcDomainPackIdForPage) {
    checkOpcDomainPage(root, opcDomainPackIdForPage, Array.isArray(spec.commands) && spec.commands.length > 0);
  }
  checkDeclaredTaskSources(declared, root);
  if (domainPackSource) { checkDeclaredDomainPacks(root); }

  if (!declared.worker) {
    console.log("[plugin-bundle] --check ✅ 声明式插件无 worker，跳过帧握手");
    return;
  }
  if (!spec.packageName) {
    fail("--check：清单声明了 worker，但插件规格没有可构建的包名");
  }
  const program = path.join(root, declared.worker.program);
  if (!fs.existsSync(program)) { fail(`--check：清单声明的 worker 不存在：${program}`); }
  const onDisk = createHash("sha256").update(fs.readFileSync(program)).digest("hex");
  if (!declared.integrity || onDisk !== declared.integrity.hash) {
    fail(`--check：integrity 不匹配（清单 ${declared.integrity?.hash ?? "缺失"} / 盘上 ${onDisk}）`);
  }

  const child = spawn(program, [], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
  // 任何提前 `fail()` 都要带走子进程：`fail` 是 `process.exit(1)`，不会回收它。
  process.on("exit", () => child.kill());
  const reader = frameReader(child.stdout);

  const describeId = "bundle-check-describe";
  writeFrame(child.stdin, { op: "describe", args: { proto_version: 1 }, request_id: describeId });
  const describe = await reader.next();
  if (describe.kind !== "success") { fail(`--check：describe 失败：${JSON.stringify(describe)}`); }
  if (describe.request_id !== describeId) { fail(`--check：describe 未回填 request_id：${JSON.stringify(describe)}`); }
  if (describe.value?.proto_version !== 1) {
    fail(`--check：声明协议版本不符：${JSON.stringify(describe.value)}`);
  }

  writeFrame(child.stdin, { op: "shutdown", args: {} });
  const shutdown = await reader.next();
  if (shutdown.kind !== "success") { fail(`--check：shutdown 回执失败：${JSON.stringify(shutdown)}`); }

  // `shutdown` 在协议里只是**回执**，真正的停机信号是 stdin EOF（worker 的 `serve` 循环
  // 靠对端关管道结束）。少这一步，自检会永远挂在 `child.on("exit")` 上 —— 2026-11-04 实测
  // （worker 收到回执后仍在等下一帧，脚本却在等它退出，双方互等）。
  child.stdin.end();
  let timer;
  const exitCode = await Promise.race([
    new Promise((resolve) => child.on("exit", resolve)),
    new Promise((resolve) => {
      timer = setTimeout(() => resolve(null), 10000);
    }),
  ]);
  clearTimeout(timer);
  if (exitCode === null) { fail("--check：stdin EOF 后 10 s 内未退出"); }
  if (exitCode !== 0) { fail(`--check：worker 退出码 ${exitCode}（应为 0）`); }

  console.log("[plugin-bundle] --check ✅ describe 握手 + shutdown 回执 + 退出码 0 + sha256 一致");
}

/**
 * 任务源声明的自证（B8-1b）：与宿主 `build_manifest_task_sources` 同判据，
 * 目的是让「装不上才发现」提前到打包期。宿主侧是权威，这里只是提前一遍 ——
 * 判据漂了会以 e2e 安装测试的形式暴露，不靠本函数兜底。
 */
function checkDeclaredTaskSources(declared, root) {
  const sources = declared.taskSources ?? [];
  const declaredCommands = new Set((declared.commands ?? []).map((c) => c.name));
  // `detailSchema` 的归属：只能是**本包 `ui.json`** 的 key（#149 第 3 步）。
  // 与宿主 `build_manifest_task_sources` 同判据：拼别人的 key / 写全行 id 都不许，
  // 宿主那边还会再按库里现存 schema 复核一次。
  const uiSchemaPath = path.join(root, "ui.json");
  let uiKeys = new Set();
  if (fs.existsSync(uiSchemaPath)) {
    try {
      uiKeys = new Set((JSON.parse(fs.readFileSync(uiSchemaPath, "utf8")).schemas ?? []).map((s) => s.key));
    } catch (error) {
      fail(`--check：ui.json 不是合法 JSON：${error.message}`);
    }
  }
  const seenIds = new Set();
  for (const source of sources) {
    if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(source.id ?? "")) {
      fail(`--check：任务源 id 形状非法（小写字母数字与 -）：${JSON.stringify(source.id)}`);
    }
    if (seenIds.has(source.id)) { fail(`--check：任务源 id 在本包内重复：${source.id}`); }
    seenIds.add(source.id);
    if (!source.title || !source.title.trim()) { fail(`--check：任务源 ${source.id} 没有 title`); }
    if (!declaredCommands.has(source.command)) {
      fail(`--check：任务源 ${source.id} 的 command「${source.command}」不在 commands[] 里`);
    }
    if (source.detailSchema !== undefined && source.detailSchema !== null && `${source.detailSchema}`.trim() !== "") {
      if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(source.detailSchema)) {
        fail(`--check：任务源 ${source.id} 的 detailSchema 必须是 key（不是行 id）：${source.detailSchema}`);
      }
      if (!uiKeys.has(source.detailSchema)) {
        fail(`--check：任务源 ${source.id} 的 detailSchema「${source.detailSchema}」不在 ui.json 里`);
      }
    }
  }
  console.log(
    `[plugin-bundle] --check ✅ ${sources.length} 个任务源声明自洽（command ∈ commands[]，detailSchema ∈ ui.json）`,
  );
}

/**
 * 产物自检的前端一侧：`pages[]` 必须真能指向安装根内的既有资产。
 *
 * 校验口径照宿主 `PluginManager::resolve_page_asset` 的载入期防线，但**不替代**它 ——
 * 宿主那边还有 registry 哈希锚与 canonicalize（符号链接逃逸）。这里只保证
 * 「打包脚本写出的清单与盘上文件一致」，把那类缺陷挡在装机之前。
 */
function checkDeclaredPages(declared, root) {
  const pages = declared.pages ?? [];
  if (pages.length === 0) { fail("--check：清单没有 pages[]，前端 bundle 通道无从验证"); }
  let bundlePages = 0;
  for (const page of pages) {
    // `schema` 页的 entry 是动态 UI Schema 的 id，没有文件资产可查（宿主同样跳过形状校验）。
    if (page.loading !== "bundle") { continue; }
    bundlePages += 1;
    if (!page.entry) { fail(`--check：页面 ${page.id} 没有 entry`); }
    if (path.isAbsolute(page.entry) || page.entry.startsWith("/") || page.entry.startsWith("\\")) {
      fail(`--check：页面 ${page.id} 的 entry 必须是相对安装目录的路径：${page.entry}`);
    }
    if (page.entry.split(/[\\/]/).includes("..")) {
      fail(`--check：页面 ${page.id} 的 entry 不得逃出安装目录：${page.entry}`);
    }
    const ext = path.extname(page.entry).slice(1).toLowerCase();
    if (!PAGE_ASSET_ALLOWED_EXTS.has(ext)) {
      fail(`--check：页面 ${page.id} 的 entry 扩展名不在白名单：${page.entry}`);
    }
    const absolute = path.resolve(root, page.entry);
    const relative = path.relative(root, absolute);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      fail(`--check：页面 ${page.id} 的资产落在安装根之外：${absolute}`);
    }
    if (!fs.existsSync(absolute) || !fs.statSync(absolute).isFile()) {
      fail(`--check：清单声明的页面资产不存在：${absolute}`);
    }
  }
  if (bundlePages === 0) { fail("--check：pages[] 里没有任何 bundle 页，通道未被验证"); }
  console.log(`[plugin-bundle] --check ✅ ${bundlePages} 个 bundle 页的 entry 均存在且在白名单内`);
}

/**
 * OPC 域包页面的安装产物自检：worker 域必查 bundle.js；
 * 静态数据区只查 `index.html` 确实声明加载 domain-data.js 的域 ——
 * accounting / sales-growth / content-media 三个老 worker 域从未有静态区，
 * 与 `generate-declarative-opc-pages.mjs` 的 pageAssets 口径保持一致。
 */
function checkOpcDomainPage(root, expectedId, withWorkerBundle) {
  const pageDir = path.join(root, "pages");
  const pagePath = path.join(pageDir, "index.html");
  const dataPath = path.join(pageDir, "domain-data.js");
  if (!fs.existsSync(pagePath)) {
    fail(`--check：域包页面产物缺失：${pageDir}`);
  }
  const html = fs.readFileSync(pagePath, "utf8");
  const withStaticData = html.includes('src="./domain-data.js"');
  if (withStaticData && !fs.existsSync(dataPath)) {
    fail(`--check：域包页面声明了 domain-data.js 但产物缺失：${dataPath}`);
  }
  if (withWorkerBundle) {
    const bundlePath = path.join(pageDir, "bundle.js");
    if (!fs.existsSync(bundlePath)) { fail("--check：worker 域页面缺少 bundle.js"); }
    if (!html.includes('src="./bundle.js"')) {
      fail("--check：worker 域页面没有加载 bundle.js");
    }
  }
  if (!withStaticData) {
    console.log(`[plugin-bundle] --check ✅ 域包页面无静态数据区（${expectedId}：仅 bundle 交互）`);
    return;
  }
  for (const selector of ["#capabilities", "#kpis", "#workflow-steps", "#data-sources", "#acceptance"]) {
    if (!html.includes(selector)) { fail(`--check：域包页面缺少渲染区 ${selector}`); }
  }
  const raw = fs.readFileSync(dataPath, "utf8");
  const match = /^globalThis\.AX_OPC_DOMAIN_DATA\s*=\s*([\s\S]*);\s*$/.exec(raw);
  if (!match) { fail("--check：domain-data.js 不是纯数据赋值"); }
  let parsed;
  try {
    // 生成物经 dprint 格式化后会保留对象/数组尾随逗号；它是合法 JS 字面量但不是合法 JSON。
    // 用 Function 只求值这段静态数据，避免把生成器输出格式与宿主自检口径绑死。
    parsed = new Function(`"use strict"; return (${match[1]});`)();
  } catch (error) {
    fail(`--check：domain-data.js 不是合法静态数据：${error.message}`);
  }
  if (parsed.id !== expectedId) {
    fail(`--check：domain-data.js id=${parsed.id}，期望 ${expectedId}`);
  }
  if (!Array.isArray(parsed.capabilities) || parsed.capabilities.length === 0) {
    fail("--check：域包至少需要展示一条 capability");
  }
  console.log(
    `[plugin-bundle] --check ✅ 域包页面数据完整（${parsed.id}：${parsed.capabilities.length} 能力 / ${parsed.kpis.length} KPI / ${parsed.workflowSteps.length} 步骤）`,
  );
}

/** 域包清单自检：本插件预期份数（公共 13 / accounting 1），owner 与命令不跨域重复。 */
function checkDeclaredDomainPacks(root) {
  // 所有插件的 logical manifest 全局只读投影：命令 owner 不得在两个产物里同时声明。
  const globalCommandOwners = new Map();
  for (const source of Object.values(domainPackSourceByPlugin)) {
    for (const file of fs.readdirSync(source).filter((f) => f.endsWith(".json")).sort()) {
      const global = JSON.parse(fs.readFileSync(path.join(source, file), "utf8"));
      for (const command of global.commands ?? []) {
        if (globalCommandOwners.has(command)) {
          fail(`--check：命令 ${command} 被 ${globalCommandOwners.get(command)} 与 ${global.id} 同时声明`);
        }
        globalCommandOwners.set(command, global.id);
      }
    }
  }
  const dir = path.join(root, DOMAIN_PACK_DIR);
  if (!fs.existsSync(dir)) { fail(`--check：域包清单目录不存在：${dir}`); }
  const files = fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort();
  if (expectedDomainPackIds.length > 0 && files.length !== expectedDomainPackIds.length) {
    fail(`--check：域包清单应为 ${expectedDomainPackIds.length} 份，实得 ${files.length}`);
  }
  const expectedOwner = domainPackOwnerFor(pluginName);
  const seenIds = new Set();
  const seenPluginIds = new Set();
  for (const file of files) {
    let manifest;
    try {
      manifest = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
    } catch (error) {
      fail(`--check：域包清单不是合法 JSON（${file}）：${error.message}`);
    }
    if (!manifest.id || !manifest.pluginId || manifest.transportOwner !== expectedOwner) {
      fail(`--check：域包清单 ${file} 缺 id/pluginId 或 transportOwner 不是 ${expectedOwner}`);
    }
    if (seenIds.has(manifest.id)) { fail(`--check：域包 id 重复：${manifest.id}`); }
    if (seenPluginIds.has(manifest.pluginId)) { fail(`--check：域包 pluginId 重复：${manifest.pluginId}`); }
    seenIds.add(manifest.id);
    seenPluginIds.add(manifest.pluginId);
  }
  console.log(`[plugin-bundle] --check ✅ ${files.length} 个域包逻辑清单齐全，pluginId/owner/commands 无跨域重复`);
}

/** 递归列出目录下的所有文件（保持不排序；调用方只按相对路径逐个处理）。 */
function listFilesRecursive(dir) {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) { files.push(...listFilesRecursive(full)); }
    else if (entry.isFile()) { files.push(full); }
  }
  return files;
}

function writeFrame(stream, value) {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  const header = Buffer.alloc(4);
  header.writeUInt32BE(payload.length, 0);
  stream.write(Buffer.concat([header, payload]));
}

/** 长度前缀 JSON 帧读取器：返回 `next()`，每次给出一帧反序列化后的对象。 */
function frameReader(stream) {
  let buffered = Buffer.alloc(0);
  /** 等待中的读取者：`{ resolve, reject, timer }`。 */
  const waiting = [];
  const pending = [];
  stream.on("data", (chunk) => {
    buffered = Buffer.concat([buffered, chunk]);
    for (;;) {
      if (buffered.length < 4) { break; }
      const length = buffered.readUInt32BE(0);
      if (buffered.length < 4 + length) { break; }
      const payload = buffered.subarray(4, 4 + length);
      buffered = buffered.subarray(4 + length);
      const parsed = JSON.parse(payload.toString("utf8"));
      const waiter = waiting.shift();
      if (waiter) {
        clearTimeout(waiter.timer);
        waiter.resolve(parsed);
      } else {
        pending.push(parsed);
      }
    }
  });
  stream.on("error", (error) => {
    for (const waiter of waiting.splice(0)) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
  });
  return {
    next() {
      if (pending.length > 0) { return Promise.resolve(pending.shift()); }
      return new Promise((resolve, reject) => {
        const waiter = {
          resolve,
          reject,
          timer: setTimeout(() => {
            const index = waiting.indexOf(waiter);
            if (index >= 0) { waiting.splice(index, 1); }
            reject(new Error("等回帧超时（5 s）"));
          }, 5000),
        };
        waiting.push(waiter);
      });
    },
  };
}

/**
 * stock 清单：**字段、键序、取值来源与参数化之前逐字一致**（任务 #36 的硬要求）。
 * 唯一改写是把 `workerPackage.version` 换成 `ctx.version`（同一个值，经 `cargo metadata` 取）。
 */
function stockManifest({ version, binaryName, sha256 }) {
  return {
    name: "stock",
    version,
    description:
      "股票域功能包：行情工具 get_stock_quote + 107 条命名命令（行情/意图解析/证据权重/在跑执行/分析记录/行业成分股/市场主线写入/逐票因子/管道历史与详情/进化计划/DES 仿真/三族 cron 配置 CRUD/自选监视列表/批量行情/批量财务/组合风险/交易对比/板块联动扫描/监控优先级/财务快照/交易日历/组合持仓与压力测试/组合限制/投资组合导入/决策回测查询/NeoDATA 令牌/决策回测触发/验证记录/验证报告/情绪深度/因子清单/基本面报告/基本面 Markdown/最近分析摘要/参数调整建议/热门候选发现/失效荐股缓存/自改进分析循环/批量分析摘要/权重时间线/一致性分数历史/节点反馈通用版/月度投资报告/宏观数据快照/漏检回灌清单/回测反馈分析/VLM 截图解析/回测记录读取/量化策略列表/指标对比/Rhai 策略注册/vendor 状态/任务取消/漏检补分析批次三件/自选股与分组/股票与反思定时任务/自选股扫描/行情监控/组合持仓与交易/进化审批/分析读写与台账/监控与数据源/进化与荐股留痕/权重与参数台账/估值与工具）+ 1 个前端页面与 2 个任务中心任务源",
    // worker 由宿主 spawn ⇒ 必须声明 `subprocess_execution`，否则 enable 时
    // `check_subprocess_permission` 直接拒绝（沙箱是从 manifest.permissions 推出来的，
    // 不是 `PluginWorkerConfig::new` 的 permissive 默认值）。
    //
    // ⚠ `write` 是 E 批（#240）加的，**两个面都要它**，缺一条即「装得上、跑必失败」：
    // ① 插件出站：`worker.rs::required_permission_for(("stock.analysis", "batch_upsert_mainlines"))`
    //    返回 `Write`，而 `has_permission` 是**精确 contains**（不套 `implies_*`，
    //    `subprocess_execution` 顶不上）⇒ 命令在接缝调用前落 `PLUGIN_PERMISSION_DENIED`；
    // ② 命令入站：该命令 `requiredPermission: "workspace-write"`，`check_command_gate`
    //    要求插件声明 `write | file_system_write`，否则整条命令被拒。
    //
    // ⚠ `execute` 是 #328 加的：`stock_delete_{pool_scan,trend_screening}_cron` 两条的宿主
    // 等价命令是 `Dangerous`，清单对齐成 `danger-full-access`，而 `check_command_gate` 的
    // DangerFullAccess 分支**只认 `execute`**（`subprocess_execution` 明确不算）⇒ 不声明它，
    // 这两条会在启动子进程前被拒。副作用仅限 gate 的档位判定：沙箱推导
    // （`build_sandbox_from_permissions`）只看 `subprocess_execution` / `network_access`。
    // 本条**部分推翻** PLAN §141 未做③ / §142.3 的「清单不加 danger-full-access」终局裁定，
    // 评审依据见 PLAN §186；`stock_delete_recommendation_cron` 不在升档面（宿主原档 Caution）。
    permissions: ["subprocess_execution", "write", "execute"],
    mcp_servers: [],
    skills: [],
    agents: [],
    // 默认启用：前端 `StockSearchBar` 的 NL 意图解析已改走本插件的命名命令，
    // 宿主同名命令（B4-j 后删除）不再兜底 ⇒ 内置包不默认启用就等于该功能静默失效。
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    // 前端页面声明：`entry` 相对安装目录（扩展名走宿主资产白名单），宿主
    // `PluginManager::resolve_page_asset` 把它解析成安装根内的绝对路径；
    // 前端只传 `(pluginId, pageId)`，不得自己拼路径。
    // 导航元数据（navGroup/labelKey/icon/navOrder）是宿主侧栏唯一声明面：宿主只投影不写死。
    pages: [{
      id: "stock-home",
      title: "股票",
      entry: PAGE_ENTRY,
      loading: "bundle",
      navGroup: "invest",
      labelKey: "nav.stockAnalysis",
      icon: "ChartBar",
      navOrder: 0,
    }],
    // `command` 用 `worker://<名>` 占位：worker 通道不读它（按 `name` 发帧），但若意外
    // 退回 spawn 通道，这个不存在的程序名会当场失败而不是静默跑别的东西。
    commands: [
      {
        // 批 2b：入参键由 `stock_code` 改 camelCase `stockCode`（禁区 13 既存偏离收口）。
        // 接缝面的 snake_case 翻译点在插件 `market_read_route`，不在清单；宿主 LLM 工具面
        // （`stock_analysis_bridge.rs` 的 `stock_get_quote`）仍声明 `stock_code`，两条面各有调用方。
        name: "stock_get_quote",
        description: "获取 A 股个股实时行情（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_quote",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        // 批 2b：K 线走 `market.quote` 已实现的 `get_klines` op。
        // 不开放 `asOfDate` —— 宿主 `get_stock_kline` 有该入参，但接缝没有，多写一个键就是
        // 承诺一条取不到数的通道；`adjType` 与宿主 `adj` 同档（auto 与缺省都落未复权）。
        name: "stock_get_kline",
        description: "获取 A 股 K 线序列（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_kline",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "6 位股票代码，如 600519" },
            period: { type: "string", description: "周期，如 daily / weekly / 60min" },
            limit: { type: "integer", minimum: 1, description: "取多少根，宿主缺省 100" },
            adjType: { type: "string", description: "复权档：none / forward / backward；auto 与缺省=未复权" },
          },
          required: ["stockCode", "period", "limit"],
        },
      },
      {
        name: "parse_analysis_intent",
        description: "解析自然语言分析意图（纯函数，跑 axagent-stock-core 的权威实现）",
        command: "worker://parse_analysis_intent",
        inputSchema: {
          type: "object",
          properties: { input: { type: "string", description: "如「调研茅台短线」" } },
          required: ["input"],
        },
      },
      {
        // B9-1：宿主同名 Tauri 命令在双声明期仍保留；前端已切到本条，
        // 删除宿主实现与计数收口走 B9-2（避免同批做「改路由 + 重生成注册表」两件不可回滚的事）。
        name: "compute_evidence_weights",
        description: "按市场环境/投资周期/分析师输入计算证据质量权重与共识（纯函数，跑 analysis-engine 权威实现）",
        command: "worker://compute_evidence_weights",
        inputSchema: {
          type: "object",
          properties: {
            marketRegime: {
              type: "object",
              properties: {
                regime: { type: "string", description: "bull | bear | sideways | volatile" },
                confidence: { type: "number", description: "0-1" },
                volatility: { type: "string", description: "high | low | normal" },
                description: { type: "string" },
                volatilityPct: { type: "number" },
                consecutiveUp: { type: "integer" },
                consecutiveDown: { type: "integer" },
              },
              required: ["regime", "confidence", "volatility", "description"],
            },
            timeHorizon: { type: "string", description: "ultra_short | short | mid | long" },
            analysts: {
              type: "array",
              items: {
                type: "object",
                properties: {
                  analystId: { type: "string" },
                  reportText: { type: "string" },
                  stance: { type: "string" },
                  bullScore: { type: "number" },
                  bearScore: { type: "number" },
                  positionPct: { type: "number" },
                  untrusted: { type: "boolean" },
                },
                required: ["analystId"],
              },
            },
            historicalWeights: { type: "object", additionalProperties: { type: "number" } },
          },
          required: ["marketRegime", "timeHorizon", "analysts"],
        },
      },
      {
        // B8-1b 建通道、#150 换取数口：行数据全部来自宿主 `workflow.run` 的 `list_recent`
        // （在跑 + 已结束），插件只做状态词表翻译（引擎六态 → 中心七态）与形状收敛。
        name: "list_recent_workflow_runs",
        description: "列出本插件近期工作流执行（在跑与已结束，经宿主 workflow.run 接缝，返回任务中心行数组）",
        command: "worker://list_recent_workflow_runs",
        inputSchema: {
          type: "object",
          properties: {
            limit: { type: "integer", description: "取数上限，宿主夹在 1..=200，缺省 50" },
            workflowId: { type: "string", description: "只看该 workflow 的执行；缺省 = 全部" },
          },
        },
      },
      {
        // #149 第 1 步建通道、第 3 步才进清单：行数据来自宿主 `analysis.record` 的 `list`
        // （宿主发起的单股分析记录也在内），插件只做状态词表翻译与形状收敛。
        // `name` 必须与 worker 的 `COMMAND_ANALYSIS_RECORDS` 逐字一致（`list_stock_analysis_records`）。
        name: "list_stock_analysis_records",
        description: "列出单股分析记录（经宿主 analysis.record 接缝，只交根记录，返回任务中心行数组）",
        command: "worker://list_stock_analysis_records",
        inputSchema: {
          type: "object",
          properties: {
            limit: { type: "integer", description: "取数上限，宿主夹在 1..=200，缺省 50" },
            offset: { type: "integer", description: "分页偏移，缺省 0" },
          },
        },
      },
      // ── E 批（#240）：三条数据面命令，取数经宿主 `stock.analysis` 接缝 ──
      // 三个工具（`get_industry_constituents` / `market_mainline_batch_upsert` /
      // `get_stock_factor`）只发命令，不再直连 `analysis-engine`（跨包口归零）。
      // `name` 必须与 worker 的三个 `COMMAND_*` 常量逐字一致。
      {
        // 只读：纯查询，宿主 `required_permission_for` 不给它登记权限（值仍是 `None`）。
        name: "stock_get_industry_constituents",
        description: "按行业名查询 A 股成分股分组（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_get_industry_constituents",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            industries: {
              type: "array",
              items: {
                oneOf: [
                  { type: "string" },
                  {
                    type: "object",
                    properties: {
                      name: { type: "string" },
                      leaderCode: { type: "string" },
                    },
                    required: ["name"],
                  },
                ],
              },
            },
          },
          required: ["industries"],
        },
      },
      {
        // 唯一写命令：`requiredPermission` 必须是 `workspace-write`（否则按未声明=只读档，
        // 智能体面把它当 `Safe` 暴露、确认弹窗形同虚设）；`permissions` 里也必须有 `write`，
        // 否则 `check_command_gate` 直接拒（见上方 permissions 注释的两面）。
        name: "stock_market_mainline_batch_upsert",
        description: "批量写入市场主线（经宿主 stock.analysis 写面；回 inserted / updated / archived）",
        command: "worker://stock_market_mainline_batch_upsert",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: true,
          properties: {
            mainline_date: { type: "string", description: "主线日期 YYYY-MM-DD（必填，也收 mainlineDate）" },
            mainlines: {
              type: "array",
              items: { type: "object", additionalProperties: true },
            },
            archive_missing: { type: "boolean", description: "归档当日未提及主线（也收 archiveMissing）" },
            source_workflow_execution_id: { type: "string" },
          },
          required: ["mainline_date", "mainlines"],
        },
      },
      {
        // 只读：逐票 1 月涨跌幅，宿主不登记权限（`None`）。
        name: "stock_get_stock_factor",
        description: "逐票查询 1 月涨跌幅（经宿主 stock.analysis 接缝，只读；缺席票单独报）",
        command: "worker://stock_get_stock_factor",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stock_codes: {
              description:
                '股票代码清单（宽容形态）：字符串数组、{"stockCode"|"code"} 对象数组、或含 stocks 分组的信封',
            },
            as_of_date: { type: "string", description: "可选截止日（YYYY-MM-DD）" },
          },
          required: ["stock_codes"],
        },
      },
      {
        // 只读（#304 C3）：宿主 `market_mainline_list_recent` 退役后的取数面，前端
        // `MarketMainlinePanel` 消费；宿主 provider 缺省 7 天、上界 90 天。
        name: "stock_market_mainline_list_recent",
        description: "列出最近 N 天市场主线（经宿主 stock.analysis 接缝，只读；缺省 7 天）",
        command: "worker://stock_market_mainline_list_recent",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            days: { type: "integer", description: "回看天数（缺省 7，宿主上界 90）" },
          },
        },
      },
      // ── T2 试点批（#248）：四条宿主命令的取数面（只读 / 纯计算），经 `stock.analysis` 接缝 ──
      // 与 E 批三条同一透传纪律：字段形状的权威在主壳 commands 层（pipeline 手拼行 /
      // dao 进化模型 / `commands/wf_des.rs` 的 WfDes DTO），插件侧改写即第二套形状。
      // 四条都只读，宿主 `required_permission_for` 不登记权限（同 E 批只读三条）。
      {
        name: "stock_get_pipeline_history",
        description: "列出管道执行历史（经宿主 stock.analysis 接缝，只读；返回手拼行数组）",
        command: "worker://stock_get_pipeline_history",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "取数上限，宿主缺省取 20" },
          },
        },
      },
      {
        name: "stock_get_pipeline_run_detail",
        description: "查询单次管道执行详情（经宿主 stock.analysis 接缝，只读；缺席 run 由宿主报错）",
        command: "worker://stock_get_pipeline_run_detail",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            runId: { type: "string", description: "管道执行 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["runId"],
        },
      },
      {
        // 该命令无参：入参非空对象会被宿主 provider 的 Validation 拒收。
        name: "stock_list_pending_evolution_plans",
        description: "列出待审批的反思进化计划（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_pending_evolution_plans",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #328：宿主 `get_latest_analysis_for_stock` 的接缝面。出参为 `null` 表示「无记录」，
        // 不是空对象 —— 消费端按 null 判「没有」。
        name: "stock_get_latest_analysis_for_stock",
        description: "查询个股最近一次完整分析的决策摘要（经宿主 stock.analysis 接缝，只读；无记录返回 null）",
        command: "worker://stock_get_latest_analysis_for_stock",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
            asOfDate: { type: "string", description: "时间旅行截止日（YYYY-MM-DD）；省略则取最新" },
          },
          required: ["stockCode"],
        },
      },
      {
        // #334：宿主 `list_mover_reflow_candidates` 的接缝面 —— 漏检回灌清单（只读、零入参）。
        name: "stock_list_mover_reflow_candidates",
        description: "读取漏检回灌清单与转正就绪度（经宿主接缝，只读，无参数）",
        command: "worker://stock_list_mover_reflow_candidates",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #335：宿主 `analyze_backtest_feedback` 的接缝面 —— 回测反馈分析（纯计算，宿主原档
        // Caution ⇒ 等阶 workspace-write，照 run_self_improving_stock_analysis 先例）。
        name: "stock_analyze_backtest_feedback",
        description: "分析回测参与记录，生成每位分析师的表现分析、趋势判断与 Prompt 调整建议（纯计算，经宿主接缝）",
        command: "worker://stock_analyze_backtest_feedback",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            participations: {
              type: "array",
              description: "回测参与记录（每位分析师每条决策一行）",
              items: {
                type: "object",
                additionalProperties: false,
                properties: {
                  analystId: { type: "string", description: "分析师 ID" },
                  timeHorizon: { type: "string", description: "时间维度" },
                  date: { type: "string", description: "分析日期（YYYY-MM-DD）" },
                  stance: {
                    type: "string",
                    description: '立场方向（"bullish" | "bearish" | "neutral"）',
                  },
                  confidence: { type: "number", description: "置信度（0-1）" },
                  wasCorrect: { type: "boolean", description: "实际走势是否正确" },
                },
                required: [
                  "analystId",
                  "timeHorizon",
                  "date",
                  "stance",
                  "confidence",
                  "wasCorrect",
                ],
              },
            },
          },
          required: ["participations"],
        },
      },
      {
        // #335：宿主 `parse_vlm_portfolio_screenshot` 的接缝面 —— VLM 截图解析（纯解析，只读）。
        name: "stock_parse_vlm_portfolio_screenshot",
        description: "解析 VLM 输出的持仓截图识别文本，返回结构化持仓数据（纯解析，经宿主接缝，只读）",
        command: "worker://stock_parse_vlm_portfolio_screenshot",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            rawVlmOutput: {
              type: "string",
              description: "VLM 模型的原始文本输出（含持仓 JSON）",
            },
          },
          required: ["rawVlmOutput"],
        },
      },
      {
        // #337：宿主 `set_vendor_status` 的接缝面 —— 手动设置 vendor 健康状态（写语义，
        // 宿主原档 Safe 但改运行态 ⇒ 等阶 workspace-write）。
        name: "stock_set_vendor_status",
        description:
          "手动设置数据源（vendor）健康状态 healthy/degraded/disabled（经宿主 stock.analysis 接缝，写运行态）",
        command: "worker://stock_set_vendor_status",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            vendor: { type: "string", description: "数据源标识" },
            status: {
              type: "string",
              description: "目标状态",
              enum: ["healthy", "degraded", "disabled"],
            },
          },
          required: ["vendor", "status"],
        },
      },
      {
        // 主行情源设置 —— 原宿主命令 `stock_set_market_data_primary_vendor` 的业务体
        // 收进股票插件；写 settings + 即时重排 quote/klines 链首（等阶 workspace-write）。
        name: "stock_set_market_data_primary_vendor",
        description: "设置主行情源并即时重排 quote/klines 链首（经宿主 stock.analysis 接缝；传 vendor=null 清除覆盖）",
        command: "worker://stock_set_market_data_primary_vendor",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            vendor: {
              type: ["string", "null"],
              description: "主行情源标识；null / 空串清除覆盖，恢复默认优先级",
            },
            applyFromSettings: {
              type: "boolean",
              description: "只读 settings 并应用（装配/重放已保存的主源），不写库",
            },
          },
          required: [],
        },
      },
      {
        // #337：宿主 `cancel_stock_analysis` 的接缝面 —— 取消进行中的分析任务（写语义）。
        name: "stock_cancel_stock_analysis",
        description:
          "取消一个进行中的股票分析任务（翻转取消令牌；任务不存在或已完成时报错）（经宿主 stock.analysis 接缝，写运行态）",
        command: "worker://stock_cancel_stock_analysis",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analysisId: { type: "string", description: "分析任务 ID" },
          },
          required: ["analysisId"],
        },
      },
      {
        // #347：宿主 `list_time_travel_batches` 的接缝面。
        name: "stock_list_time_travel_batches",
        description: "时间旅行批次历史（摘要列表，经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_time_travel_batches",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "缺省 50，上限 200" },
            statusFilter: { type: "string" },
            asOf: { type: "string", description: "as-of 代际过滤" },
          },
        },
      },
      {
        // #348：宿主 `reco_ic_stats` 的接缝面。
        name: "stock_reco_ic_stats",
        description: "荐股逐档 rank IC 与预测半衰期统计（宿主 stock.analysis 接缝）",
        command: "worker://stock_reco_ic_stats",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: { generation: { type: "string" } } },
      },
      {
        // #348：宿主 `get_reco_signal_history` 的接缝面。
        name: "stock_get_reco_signal_history",
        description: "最近一次荐股的策略信号明细（宿主 stock.analysis 接缝）",
        command: "worker://stock_get_reco_signal_history",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: { strategyId: { type: "string" } }, required: ["strategyId"] },
      },
      {
        // #348：宿主 `get_cached_recommendation` 的接缝面。
        name: "stock_get_cached_recommendation",
        description: "读取缓存荐股结果（宿主 stock.analysis 接缝）",
        command: "worker://stock_get_cached_recommendation",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { period: { type: "string" }, asOfDate: { type: "string" } },
          required: ["period"],
        },
      },
      {
        // #348：宿主 `save_eastmoney_proxy` 的接缝面。
        name: "stock_save_eastmoney_proxy",
        description: "保存东财代理（热更新运行态并持久化到工作流模板变量）",
        command: "worker://stock_save_eastmoney_proxy",
        requiredPermission: "workspace-write",
        inputSchema: { type: "object", properties: { proxyUrl: { type: "string" } }, required: ["proxyUrl"] },
      },
      {
        // #348：宿主 `list_reco_history` 的接缝面。
        name: "stock_list_reco_history",
        description: "荐股历史列表（宿主 stock.analysis 接缝）",
        command: "worker://stock_list_reco_history",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: {
            styleFilter: { type: "string" },
            excludeStyles: { type: "string" },
            limit: { type: "number" },
            offset: { type: "number" },
            generation: { type: "string" },
          },
        },
      },
      {
        // #348：宿主 `get_reco_detail` 的接缝面。
        name: "stock_get_reco_detail",
        description: "荐股历史明细（宿主 stock.analysis 接缝）",
        command: "worker://stock_get_reco_detail",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: {
            generatedAt: { type: "string" },
            styleFilter: { type: "string" },
            excludeStyles: { type: "string" },
          },
          required: ["generatedAt"],
        },
      },
      {
        // #348：宿主 `record_decision_trust` 的接缝面。
        name: "stock_record_decision_trust",
        description: "记录决策信任（写 stock_analyses 决策 JSON）",
        command: "worker://stock_record_decision_trust",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: { analysisId: { type: "string" }, trustModel: { type: "string" } },
          required: ["analysisId", "trustModel"],
        },
      },
      {
        // #348：宿主 `reflection_stats` 的接缝面。
        name: "stock_reflection_stats",
        description: "反思命中率统计（宿主 stock.analysis 接缝）",
        command: "worker://stock_reflection_stats",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #349：异步任务 seam（发起宿主长任务，回 { taskId }，经 stock_poll_task 轮询）。
        name: "stock_start_task",
        description: "发起股票域长任务（回测/扫描/荐股/反思/批次复盘等，宿主 spawn_tracked 台账）",
        command: "worker://stock_start_task",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          required: ["task"],
          properties: { task: { type: "string" }, input: { type: "object" } },
        },
      },
      {
        // #349：异步任务 seam（轮询）。
        name: "stock_poll_task",
        description: "轮询宿主长任务状态（completed/failed/stopped 终态回 result）",
        command: "worker://stock_poll_task",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          required: ["taskId"],
          properties: { taskId: { type: "string" } },
        },
      },
      {
        // #349：异步任务 seam（协作式取消，与宿主 stop_background_task 同源）。
        name: "stock_cancel_task",
        description: "取消宿主长任务（置协作取消旗标，终态由任务体写）",
        command: "worker://stock_cancel_task",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          required: ["taskId"],
          properties: { taskId: { type: "string" } },
        },
      },
      {
        // #349：宿主 `generate_stock_report` 的接缝面（写：HTML 渲染落盘）。
        name: "stock_generate_stock_report",
        description: "生成个股 HTML 报告并落盘（读库 + 行情网络 + 渲染）",
        command: "worker://stock_generate_stock_report",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          required: ["analysisId"],
          properties: { analysisId: { type: "string" } },
        },
      },
      {
        // #349：宿主 `get_time_travel_batch` 的接缝面（历史表权威；reap 不跟随）。
        name: "stock_get_time_travel_batch",
        description: "查询时间旅行批次状态（含回放明细；重启后从历史表还原）",
        command: "worker://stock_get_time_travel_batch",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          required: ["batchId"],
          properties: { batchId: { type: "string" } },
        },
      },
      {
        // #346：宿主 `get_reco_strategy_weights` 的接缝面。
        name: "stock_get_reco_strategy_weights",
        description: "荐股策略权重（投影评分链实际消费的 served_vars）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_reco_strategy_weights",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #346：宿主 `manual_recalc_strategy_weights` 的接缝面。
        name: "stock_manual_recalc_strategy_weights",
        description: "手动重算策略权重并落库（经宿主 stock.analysis 接缝）",
        command: "worker://stock_manual_recalc_strategy_weights",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string" },
          },
        },
      },
      {
        // #346：宿主 `recalc_reco_loop_weights` 的接缝面。
        name: "stock_recalc_reco_loop_weights",
        description: "重算荐股闭环逐格权重（经宿主 stock.analysis 接缝）",
        command: "worker://stock_recalc_reco_loop_weights",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string" },
          },
        },
      },
      {
        // #346：宿主 `apply_param_suggestions` 的接缝面。
        name: "stock_apply_param_suggestions",
        description: "应用参数调整建议到模板变量（经宿主 stock.analysis 接缝）",
        command: "worker://stock_apply_param_suggestions",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            updates: { type: "array", items: { type: "object" }, description: "建议条目数组" },
          },
        },
      },
      {
        // #346：宿主 `rollback_param_change` 的接缝面。
        name: "stock_rollback_param_change",
        description: "回滚参数变更（写回旧值并换代）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_rollback_param_change",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            changeId: { type: "string" },
          },
        },
      },
      {
        // #346：宿主 `apply_reco_weights` 的接缝面。
        name: "stock_apply_reco_weights",
        description: "应用荐股策略权重调整（经宿主 stock.analysis 接缝）",
        command: "worker://stock_apply_reco_weights",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            weights: { type: "array", items: { type: "object" }, description: "省略 = 全部应用" },
          },
        },
      },
      {
        // #346：宿主 `calibrate_portfolio_mgr_params` 的接缝面。
        name: "stock_calibrate_portfolio_mgr_params",
        description: "WFO 校准组合管理参数（经宿主 stock.analysis 接缝）",
        command: "worker://stock_calibrate_portfolio_mgr_params",
        requiredPermission: "workspace-write",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #346：宿主 `optimize_scoring_weights` 的接缝面。
        name: "stock_optimize_scoring_weights",
        description: "基于回测优化评分权重（经宿主 stock.analysis 接缝）",
        command: "worker://stock_optimize_scoring_weights",
        requiredPermission: "workspace-write",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #346：宿主 `compute_valuation_band` 的接缝面。
        name: "stock_compute_valuation_band",
        description: "估值带（读并回填 financial_snapshots）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_compute_valuation_band",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            years: { type: "integer", description: "回溯年数（缺省 5）" },
          },
        },
      },
      {
        // #346：宿主 `list_stock_tools` 的接缝面。
        name: "stock_list_stock_tools",
        description: "股票分析工具名清单（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_stock_tools",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #346：宿主 `compute_what_if` 的接缝面。
        name: "stock_compute_what_if",
        description: "What-If 确定性公式（Rhai，纯计算）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_compute_what_if",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            params: { type: "object", description: "WhatIfRequest（camelCase 全字段）" },
          },
        },
      },
      {
        // #345：宿主 `parse_trades_csv` 的接缝面。
        name: "stock_parse_trades_csv",
        description: "解析 CSV 交易记录文件（只预览不写库）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_parse_trades_csv",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            filePath: { type: "string", description: "CSV 文件绝对路径" },
          },
        },
      },
      {
        // #345：宿主 `evolve_node_command` 的接缝面。
        name: "stock_evolve_node_command",
        description: "触发节点自我进化（通用版，基于历史反馈优化 Prompt/配置）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_evolve_node_command",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            nodeType: { type: "string" },
            nodeId: { type: "string" },
          },
        },
      },
      {
        // #345：宿主 `save_node_feedback` 的接缝面。
        name: "stock_save_node_feedback",
        description: "保存节点质量反馈（用于自我进化）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_save_node_feedback",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            request: { type: "object", description: "SaveNodeFeedbackRequest（camelCase 全字段）" },
          },
        },
      },
      {
        // #345：宿主 `serenity_validation_summary` 的接缝面。
        name: "stock_serenity_validation_summary",
        description: "智选后验验证摘要（按代际筛样）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_serenity_validation_summary",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            generation: { type: "string", description: "代际截止日；省略 = 实盘代" },
          },
        },
      },
      {
        // #345：宿主 `create_decision_backtest_cron` 的接缝面。
        name: "stock_create_decision_backtest_cron",
        description: "创建决策回测定时任务（写 CronJobStore）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_decision_backtest_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string" },
            tPlusNList: { type: "array", items: { type: "integer" } },
            maxPicks: { type: "integer" },
            periodFilter: { type: "string" },
            enabled: { type: "boolean" },
          },
        },
      },
      {
        // #345：宿主 `get_evolution_drift_dashboard` 的接缝面。
        name: "stock_get_evolution_drift_dashboard",
        description: "进化漂移仪表盘（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_evolution_drift_dashboard",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string" },
          },
        },
      },
      {
        // #345：宿主 `get_evolution_history` 的接缝面。
        name: "stock_get_evolution_history",
        description: "进化闭环留痕历史（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_evolution_history",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "缺省 50" },
          },
        },
      },
      {
        // #345：宿主 `stock_evolution_recalc` 的接缝面。
        name: "stock_stock_evolution_recalc",
        description: "演化漂移重算并落库新权重（经宿主 stock.analysis 接缝）",
        command: "worker://stock_stock_evolution_recalc",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string" },
          },
        },
      },
      {
        // #345：宿主 `get_mover_reflow_merge` 的接缝面。
        name: "stock_get_mover_reflow_merge",
        description: "回灌并入开关（on/off，缺省 off）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_mover_reflow_merge",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #345：宿主 `set_mover_reflow_merge` 的接缝面。
        name: "stock_set_mover_reflow_merge",
        description: "写回灌并入开关（只接受 on/off）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_set_mover_reflow_merge",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            value: { type: "string", description: "on | off" },
          },
        },
      },
      {
        // #345：宿主 `get_mover_asof_summary` 的接缝面。
        name: "stock_get_mover_asof_summary",
        description: "最新一轮荐股链 as-of 回放摘要（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_mover_asof_summary",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #345：宿主 `get_reco_board_scope` 的接缝面。
        name: "stock_get_reco_board_scope",
        description: "扫描板块范围（auto/sh_main/sz_main/chinext/star/all）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_reco_board_scope",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #345：宿主 `set_reco_board_scope` 的接缝面。
        name: "stock_set_reco_board_scope",
        description: "保存扫描板块范围（非法值回落 auto）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_set_reco_board_scope",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            scope: { type: "string" },
          },
        },
      },
      {
        // #345：宿主 `get_reco_pool_batch` 的接缝面。
        name: "stock_get_reco_pool_batch",
        description: "最近一批实盘候选池留痕（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_reco_pool_batch",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_asof_degradation_count` 的接缝面 —— as-of 降级条目总数。
        name: "stock_get_asof_degradation_count",
        description: "as-of 降级条目总数（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_asof_degradation_count",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_asof_degradation_log` 的接缝面 —— as-of 降级日志快照。
        name: "stock_get_asof_degradation_log",
        description: "as-of 降级日志快照（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_asof_degradation_log",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_market_status` 的接缝面 —— 市场状态文案。
        name: "stock_get_market_status",
        description: "市场状态文案（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_market_status",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_vendor_health_all` 的接缝面 —— 所有数据源健康状态。
        name: "stock_get_vendor_health_all",
        description: "所有数据源健康状态（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_vendor_health_all",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_vendor_fallback_log` 的接缝面 —— vendor fallback 日志。
        name: "stock_get_vendor_fallback_log",
        description: "vendor fallback 日志（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_vendor_fallback_log",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_interface_health_history` 的接缝面 —— 核心接口日级健康记录。
        name: "stock_get_interface_health_history",
        description: "核心接口日级健康记录（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_interface_health_history",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            days: { type: "integer", description: "最近 N 个交易日（缺省 7，夹 1..30）" },
          },
        },
      },
      {
        // #344：宿主 `get_valuation_params` 的接缝面 —— 估值参数。
        name: "stock_get_valuation_params",
        description: "估值参数（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_valuation_params",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `save_valuation_params` 的接缝面 —— 保存估值参数。
        name: "stock_save_valuation_params",
        description: "保存估值参数（经宿主 stock.analysis 接缝）",
        command: "worker://stock_save_valuation_params",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            params: { type: "object", description: "ValuationParams（camelCase 全字段）" },
          },
          required: ["params"],
        },
      },
      {
        // #344：宿主 `get_stock_template_version_status` 的接缝面 —— 模板代际对照（代码版本 vs 库内播种）。
        name: "stock_get_stock_template_version_status",
        description: "模板代际对照（代码版本 vs 库内播种）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_stock_template_version_status",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `list_stock_analyses` 的接缝面 —— 历史分析列表。
        name: "stock_list_stock_analyses",
        description: "历史分析列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_stock_analyses",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "缺省上限 1000（超出自动截断）" },
            offset: { type: "integer" },
          },
          required: ["limit", "offset"],
        },
      },
      {
        // #344：宿主 `get_recent_analyses` 的接缝面 —— 最近分析摘要。
        name: "stock_get_recent_analyses",
        description: "最近分析摘要（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_recent_analyses",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "缺省 5" },
          },
        },
      },
      {
        // #344：宿主 `list_reflections` 的接缝面 —— 反思记录列表。
        name: "stock_list_reflections",
        description: "反思记录列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_reflections",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            limit: { type: "integer" },
            originalAnalysisId: { type: "string" },
          },
        },
      },
      {
        // #344：宿主 `get_stock_analysis` 的接缝面 —— 单条分析详情。
        name: "stock_get_stock_analysis",
        description: "单条分析详情（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_stock_analysis",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analysisId: { type: "string" },
          },
          required: ["analysisId"],
        },
      },
      {
        // #344：宿主 `audit_decision_evidence` 的接缝面 —— 决策证据对账审计。
        name: "stock_audit_decision_evidence",
        description: "决策证据对账审计（经宿主 stock.analysis 接缝）",
        command: "worker://stock_audit_decision_evidence",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analysisId: { type: "string" },
          },
          required: ["analysisId"],
        },
      },
      {
        // #344：宿主 `rename_stock_analysis` 的接缝面 —— 重命名分析记录。
        name: "stock_rename_stock_analysis",
        description: "重命名分析记录（经宿主 stock.analysis 接缝）",
        command: "worker://stock_rename_stock_analysis",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analysisId: { type: "string" },
            newName: { type: "string" },
          },
          required: ["analysisId", "newName"],
        },
      },
      {
        // #344：宿主 `list_price_alerts` 的接缝面 —— 价格告警列表。
        name: "stock_list_price_alerts",
        description: "价格告警列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_price_alerts",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `create_price_alert` 的接缝面 —— 创建价格告警（落库并进监控引擎）。
        name: "stock_create_price_alert",
        description: "创建价格告警（落库并进监控引擎）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_price_alert",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            stockName: { type: "string" },
            condition: { type: "string", description: "告警条件（如 price_above / price_below）" },
            targetPrice: { type: "number" },
          },
          required: ["stockCode", "stockName", "condition", "targetPrice"],
        },
      },
      {
        // #344：宿主 `monitor_status` 的接缝面 —— 实时监控运行读数。
        name: "stock_monitor_status",
        description: "实时监控运行读数（经宿主 stock.analysis 接缝）",
        command: "worker://stock_monitor_status",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `get_t0_config` 的接缝面 —— T+0 配置。
        name: "stock_get_t0_config",
        description: "T+0 配置（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_t0_config",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #344：宿主 `set_t0_config` 的接缝面 —— 更新 T+0 配置。
        name: "stock_set_t0_config",
        description: "更新 T+0 配置（经宿主 stock.analysis 接缝）",
        command: "worker://stock_set_t0_config",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            config: { type: "object", description: "TZeroConfig（camelCase 全字段）" },
          },
          required: ["config"],
        },
      },
      {
        // #344：宿主 `get_portfolio_dashboard` 的接缝面 —— 组合监控仪表盘。
        name: "stock_get_portfolio_dashboard",
        description: "组合监控仪表盘（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_portfolio_dashboard",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string", description: "时间旅行截止日；省略 = live 叠加实时持仓" },
          },
        },
      },
      // ── G2 模拟组合九条：宿主 `paper_portfolio_*` Tauri 命令退役后的同名插件命令 ──
      // 字段形状与入参必填口径逐字对齐 analysis-engine 的三个 Input DTO（camelCase）；
      // 业务体走宿主 `stock.analysis` 接缝，插件侧不重写组合/持仓算法。
      {
        name: "paper_portfolio_create",
        description: "创建模拟组合（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_create",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            name: { type: "string", description: "组合名称" },
            sourceEvent: { type: "string", description: "来源事件描述" },
            sourceNewsId: { type: ["string", "null"], description: "关联新闻 id" },
            sourceScreenshotDiagnosisId: { type: ["string", "null"], description: "关联截图诊断 id" },
          },
          required: ["name", "sourceEvent"],
        },
      },
      {
        name: "paper_portfolio_list",
        description: "列出模拟组合（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_list",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            status: { type: ["string", "null"], description: "状态过滤；省略/null = 全部" },
          },
        },
      },
      {
        name: "paper_portfolio_get",
        description: "获取组合详情（含持仓 + 实时盈亏，经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_get",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            portfolioId: { type: "string", description: "组合 id" },
          },
          required: ["portfolioId"],
        },
      },
      {
        name: "paper_portfolio_close",
        description: "关闭模拟组合（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_close",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            portfolioId: { type: "string", description: "组合 id" },
          },
          required: ["portfolioId"],
        },
      },
      {
        name: "paper_portfolio_archive",
        description: "归档模拟组合（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_archive",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            portfolioId: { type: "string", description: "组合 id" },
          },
          required: ["portfolioId"],
        },
      },
      {
        name: "paper_portfolio_add_position",
        description: "添加虚拟持仓（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_add_position",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            portfolioId: { type: "string", description: "组合 id" },
            symbol: { type: "string", description: "标的代码" },
            market: { type: "string", description: "市场 A / US / HK / ETF；缺省 A" },
            entryPrice: { type: "number", description: "开仓价" },
            entryDate: { type: "string", description: "开仓日 YYYY-MM-DD" },
            quantity: { type: "number", description: "数量" },
            note: { type: ["string", "null"], description: "备注" },
          },
          required: ["portfolioId", "symbol", "entryPrice", "entryDate", "quantity"],
        },
      },
      {
        name: "paper_portfolio_close_position",
        description: "平仓单只持仓（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_close_position",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            positionId: { type: "string", description: "持仓 id" },
            exitPrice: { type: "number", description: "平仓价（必须为正）" },
            exitDate: { type: "string", description: "平仓日 YYYY-MM-DD" },
          },
          required: ["positionId", "exitPrice", "exitDate"],
        },
      },
      {
        name: "paper_portfolio_close_all_positions",
        description: "批量平仓所有 open 持仓（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_close_all_positions",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            portfolioId: { type: "string", description: "组合 id" },
            exitPrice: { type: "number", description: "统一平仓价（必须为正）" },
            exitDate: { type: "string", description: "统一平仓日 YYYY-MM-DD" },
          },
          required: ["portfolioId", "exitPrice", "exitDate"],
        },
      },
      {
        name: "paper_portfolio_list_active_details",
        description: "列出所有 active 组合详情（经宿主 stock.analysis 接缝）",
        command: "worker://paper_portfolio_list_active_details",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      // ── G6 截图诊断八条：宿主 `screenshot_diagnosis_*` Tauri 命令退役后的同名插件命令 ──
      // 入参逐字对齐 analysis-engine DTO（camelCase）；业务体走宿主 `stock.analysis` 接缝，
      // 插件侧不复制 VLM / LLM / 风险诊断算法。
      {
        name: "screenshot_diagnosis_create_from_image",
        description: "上传截图自动诊断（OCR + 结构化 + 风险诊断，经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_create_from_image",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            imageBase64: { type: "string", description: "截图 base64（可带 data:image 前缀）" },
            sourceApp: { type: ["string", "null"], description: "截图来源 App" },
            providerId: { type: "string", description: "视觉模型 provider id" },
            modelId: { type: "string", description: "视觉模型 model id" },
          },
          required: ["imageBase64", "providerId", "modelId"],
        },
      },
      {
        name: "screenshot_diagnosis_create",
        description: "直接传入 positions 创建截图诊断（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_create",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            imageHash: { type: ["string", "null"], description: "截图 SHA256（去重）" },
            imagePath: { type: ["string", "null"], description: "截图本地路径" },
            imageThumbnailBase64: { type: ["string", "null"], description: "缩略图 base64" },
            imageWidth: { type: ["integer", "null"], description: "原图宽度" },
            imageHeight: { type: ["integer", "null"], description: "原图高度" },
            sourceApp: { type: ["string", "null"], description: "截图来源 App" },
            ocrText: { type: ["string", "null"], description: "OCR 全文" },
            positions: { type: "array", items: { type: "object" }, description: "结构化持仓数组" },
            totalMarketValue: { type: "number", description: "总市值；0 = 由 positions 求和" },
            diagnosis: { type: ["object", "null"], description: "风险诊断 schema" },
            narrative: { type: "string", description: "中文诊断说明" },
            recommendedActions: { type: "array", items: { type: "string" }, description: "建议动作" },
            sourceWorkflowExecutionId: { type: ["string", "null"], description: "来源工作流执行 id" },
            providerId: { type: ["string", "null"], description: "provider id" },
            modelId: { type: ["string", "null"], description: "model id" },
            status: { type: "string", description: "初始状态，默认 active" },
          },
          required: ["positions"],
        },
      },
      {
        name: "screenshot_diagnosis_get",
        description: "按 ID 获取截图诊断（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_get",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { diagnosisId: { type: "string", description: "诊断 id" } },
          required: ["diagnosisId"],
        },
      },
      {
        name: "screenshot_diagnosis_list_recent",
        description: "列出最近 N 条截图诊断（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_list_recent",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { limit: { type: ["integer", "null"], description: "条数，默认 20" } },
        },
      },
      {
        name: "screenshot_diagnosis_list_by_status",
        description: "按状态过滤截图诊断（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_list_by_status",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { status: { type: "string", description: "active / archived / failed" } },
          required: ["status"],
        },
      },
      {
        name: "screenshot_diagnosis_archive",
        description: "归档截图诊断（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_archive",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { diagnosisId: { type: "string", description: "诊断 id" } },
          required: ["diagnosisId"],
        },
      },
      {
        name: "screenshot_diagnosis_update",
        description: "更新截图诊断字段（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_update",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            diagnosisId: { type: "string", description: "诊断 id" },
            status: { type: ["string", "null"], description: "新状态" },
            narrative: { type: ["string", "null"], description: "新诊断说明" },
            recommendedActions: { type: ["array", "null"], items: { type: "string" }, description: "新建议动作" },
            errorMessage: { type: ["string", "null"], description: "失败原因" },
          },
          required: ["diagnosisId"],
        },
      },
      {
        name: "screenshot_diagnosis_to_paper_portfolio",
        description: "截图诊断一键转模拟观察组合（经宿主 stock.analysis 接缝）",
        command: "worker://screenshot_diagnosis_to_paper_portfolio",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            diagnosisId: { type: "string", description: "诊断 id" },
            name: { type: "string", description: "组合名称" },
            sourceEvent: { type: "string", description: "来源事件描述" },
          },
          required: ["diagnosisId", "name", "sourceEvent"],
        },
      },
      {
        // #344：宿主 `refresh_portfolio_metrics` 的接缝面 —— 刷新组合监控快照（写 portfolio_metrics_daily + correlation_snapshot）。
        name: "stock_refresh_portfolio_metrics",
        description:
          "刷新组合监控快照（写 portfolio_metrics_daily + correlation_snapshot）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_refresh_portfolio_metrics",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string", description: "快照归属日；省略 = 今日" },
          },
        },
      },
      {
        // #344：宿主 `check_position_limits` 的接缝面 —— 开仓限制校验。
        name: "stock_check_position_limits",
        description: "开仓限制校验（经宿主 stock.analysis 接缝）",
        command: "worker://stock_check_position_limits",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            proposedShares: { type: "integer" },
            proposedPrice: { type: "number" },
          },
          required: ["stockCode", "proposedShares", "proposedPrice"],
        },
      },
      {
        // #344：宿主 `approve_evolution_plan` 的接缝面 —— 接受待审批进化计划（执行进化并回写）。
        name: "stock_approve_evolution_plan",
        description: "接受待审批进化计划（执行进化并回写）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_approve_evolution_plan",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            historyId: { type: "string" },
            reviewer: { type: "string" },
          },
          required: ["historyId"],
        },
      },
      {
        // #344：宿主 `list_mover_recall_runs` 的接缝面 —— mover 核查轮次列表。
        name: "stock_list_mover_recall_runs",
        description: "mover 核查轮次列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_mover_recall_runs",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer" },
          },
        },
      },
      {
        // #344：宿主 `get_mover_recall_run_view` 的接缝面 —— 按轮次读回核查结果。
        name: "stock_get_mover_recall_run_view",
        description: "按轮次读回核查结果（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_mover_recall_run_view",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            runId: { type: "string" },
          },
          required: ["runId"],
        },
      },
      {
        // #344：宿主 `get_mover_recall_event_review` 的接缝面 —— 漏检事件补分析报告与逐档反思。
        name: "stock_get_mover_recall_event_review",
        description: "漏检事件补分析报告与逐档反思（经宿主 stock.analysis 接缝）",
        command: "worker://stock_get_mover_recall_event_review",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            anchorDate: { type: "string" },
            period: { type: "string" },
            chain: { type: "string", description: "reco | serenity；旧轮次省略" },
          },
          required: ["stockCode", "anchorDate", "period"],
        },
      },
      {
        // #344：宿主 `list_param_changes` 的接缝面 —— 参数变更台账。
        name: "stock_list_param_changes",
        description: "参数变更台账（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_param_changes",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "缺省 50，上限 200" },
          },
        },
      },
      {
        // #344：宿主 `evaluate_param_changes` 的接缝面 —— 参数变更到期评估（写评估结论）。
        name: "stock_evaluate_param_changes",
        description: "参数变更到期评估（写评估结论）（经宿主 stock.analysis 接缝）",
        command: "worker://stock_evaluate_param_changes",
        requiredPermission: "workspace-write",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #343：宿主 `list_portfolio` 的接缝面 —— 持仓列表。
        name: "stock_list_portfolio",
        description: "持仓列表（含实时行情盈亏，经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_list_portfolio",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #343：宿主 `add_portfolio_holding` 的接缝面 —— 添加持仓。
        name: "stock_add_portfolio_holding",
        description: "添加持仓（写 portfolio_holdings 表）",
        command: "worker://stock_add_portfolio_holding",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            stockName: { type: "string" },
            shares: { type: "number", description: "持有数量（股）" },
            avgCost: { type: "number", description: "成本价" },
          },
          required: ["stockCode", "stockName", "shares", "avgCost"],
        },
      },
      {
        // #343：宿主 `remove_portfolio_holding` 的接缝面 —— 移除持仓。
        name: "stock_remove_portfolio_holding",
        description: "移除持仓（按 id 删除）",
        command: "worker://stock_remove_portfolio_holding",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "持仓记录 ID" },
          },
          required: ["id"],
        },
      },
      {
        // #343：宿主 `get_portfolio_correlations` 的接缝面 —— 组合两两相关性快照。
        name: "stock_get_portfolio_correlations",
        description: "组合两两相关性快照（按 asOfDate 时间旅行）",
        command: "worker://stock_get_portfolio_correlations",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            asOfDate: { type: "string", description: "时间旅行截止日（YYYY-MM-DD）；省略取最新" },
          },
        },
      },
      {
        // #343：宿主 `toggle_trading_enabled` 的接缝面 —— 开关交易功能。
        name: "stock_toggle_trading_enabled",
        description: "开关交易功能（settings 表）",
        command: "worker://stock_toggle_trading_enabled",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            enabled: { type: "boolean", description: "true=开启 false=关闭" },
          },
          required: ["enabled"],
        },
      },
      {
        // #343：宿主 `import_trades` 的接缝面 —— 批量导入交易记录 CSV。
        name: "stock_import_trades",
        description: "批量导入交易记录 CSV（含交易开关闸）",
        command: "worker://stock_import_trades",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            filePath: { type: "string", description: "CSV 文件绝对路径" },
          },
          required: ["filePath"],
        },
      },
      {
        // #343：宿主 `reject_evolution_plan` 的接缝面 —— 拒绝待审批进化计划。
        name: "stock_reject_evolution_plan",
        description: "拒绝待审批进化计划（原因必填）",
        command: "worker://stock_reject_evolution_plan",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            historyId: { type: "string" },
            reason: { type: "string", description: "拒绝原因（截断 500 字）" },
            reviewer: { type: "string" },
          },
          required: ["historyId", "reason"],
        },
      },
      {
        // #343：宿主 `rollback_evolution` 的接缝面 —— 回滚进化版本到上一已接受版本。
        name: "stock_rollback_evolution",
        description: "回滚进化版本到上一已接受版本",
        command: "worker://stock_rollback_evolution",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            historyId: { type: "string", description: "指定当前生效版本；省略取最近 accepted" },
            reviewer: { type: "string" },
          },
        },
      },
      {
        // #343：宿主 `record_trade` 的接缝面 —— 记录一笔交易。
        name: "stock_record_trade",
        description: "记录一笔交易（经 trading.engine 接缝）",
        command: "worker://stock_record_trade",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            stockName: { type: "string" },
            direction: { type: "string", description: '"buy" | "sell"' },
            price: { type: "number" },
            quantity: { type: "integer" },
            tradeDate: { type: "string", description: "YYYY-MM-DD" },
            tradeTime: { type: "string", description: "HH:MM" },
            notes: { type: "string" },
            analysisId: { type: "string" },
          },
          required: ["stockCode", "stockName", "direction", "price", "quantity", "tradeDate", "tradeTime"],
        },
      },
      {
        // #343：宿主 `list_trades` 的接缝面 —— 交易历史。
        name: "stock_list_trades",
        description: "交易历史（经 trading.engine 接缝，只读）",
        command: "worker://stock_list_trades",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            limit: { type: "integer", description: "缺省 50（宿主 clamp 1..=500）" },
          },
        },
      },
      {
        // #343：宿主 `get_trade_positions` 的接缝面 —— 交易日志驱动的持仓汇总。
        name: "stock_get_trade_positions",
        description: "交易日志驱动的持仓汇总（经 trading.engine 接缝，只读）",
        command: "worker://stock_get_trade_positions",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #343：宿主 `validate_trade` 的接缝面 —— 交易校验。
        name: "stock_validate_trade",
        description: "交易校验（提交前预览，经 trading.engine 接缝）",
        command: "worker://stock_validate_trade",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string" },
            direction: { type: "string", description: '"buy" | "sell"' },
            quantity: { type: "integer" },
            price: { type: "number" },
          },
          required: ["stockCode", "direction", "quantity", "price"],
        },
      },
      {
        // #343：宿主 `get_trading_enabled` 的接缝面 —— 读取交易功能开关状态。
        name: "stock_get_trading_enabled",
        description: "读取交易功能开关状态（经 trading.engine 接缝）",
        command: "worker://stock_get_trading_enabled",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #342：宿主 `watch_stock_quotes` 的接缝面 —— 加入实时行情监控（写语义，零新 op）。
        name: "stock_watch_stock_quotes",
        description: "加入实时行情监控（前端订阅 stock-quote-update 推送；经 quote.watcher 接缝）",
        command: "worker://stock_watch_stock_quotes",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCodes: {
              type: "array",
              items: { type: "string" },
              description: "要监控的股票代码列表",
            },
            priority: { type: "string", description: "active（2s）| background（10s），缺省 active" },
            replace: { type: "boolean", description: "true=替换当前监控列表" },
          },
          required: ["stockCodes"],
        },
      },
      {
        // #342：宿主 `unwatch_stock_quotes` 的接缝面 —— 移除实时行情监控（写语义）。
        name: "stock_unwatch_stock_quotes",
        description: "移除实时行情监控（经 quote.watcher 接缝）",
        command: "worker://stock_unwatch_stock_quotes",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCodes: {
              type: "array",
              items: { type: "string" },
              description: "要移除监控的股票代码列表",
            },
          },
          required: ["stockCodes"],
        },
      },
      {
        // #342：宿主 `list_watchlist` 的接缝面 —— 自选股列表（读）。
        name: "stock_list_watchlist",
        description: "自选股列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_watchlist",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #342：宿主 `add_to_watchlist` 的接缝面 —— 添加自选股（写）。
        name: "stock_add_to_watchlist",
        description: "添加自选股（经宿主 stock.analysis 接缝）",
        command: "worker://stock_add_to_watchlist",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
            stockName: { type: "string", description: "股票名称" },
            notes: { type: "string", description: "备注/分组 JSON" },
          },
          required: ["stockCode", "stockName"],
        },
      },
      {
        // #342：宿主 `watchlist_update_group` 的接缝面 —— 更新自选股分组（写）。
        name: "stock_watchlist_update_group",
        description: "更新自选股分组（经宿主 stock.analysis 接缝）",
        command: "worker://stock_watchlist_update_group",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "自选股记录 ID" },
            groupName: { type: "string", description: "目标分组名" },
          },
          required: ["id", "groupName"],
        },
      },
      {
        // #342：宿主 `watchlist_list_groups` 的接缝面 —— 自选股分组列表（读）。
        name: "stock_watchlist_list_groups",
        description: "自选股分组列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_watchlist_list_groups",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #342：宿主 `watchlist_save_groups` 的接缝面 —— 保存自选股分组（写）。
        name: "stock_watchlist_save_groups",
        description: "保存自选股分组（经宿主 stock.analysis 接缝）",
        command: "worker://stock_watchlist_save_groups",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            groups: { type: "array", items: { type: "string" }, description: "全量分组名列表" },
          },
          required: ["groups"],
        },
      },
      {
        // #342：宿主 `create_stock_cron` 的接缝面 —— 创建股票定时分析任务（写）。
        name: "stock_create_stock_cron",
        description: "创建股票定时分析任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_stock_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
            stockName: { type: "string", description: "股票名称" },
            cronExpression: { type: "string", description: "cron 表达式（5 段）" },
          },
          required: ["stockCode", "stockName", "cronExpression"],
        },
      },
      {
        // #342：宿主 `list_stock_crons` 的接缝面 —— 股票定时任务列表（读）。
        name: "stock_list_stock_crons",
        description: "股票定时任务列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_stock_crons",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #342：宿主 `toggle_stock_cron` 的接缝面 —— 开关股票定时任务（写）。
        name: "stock_toggle_stock_cron",
        description: "开关股票定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_stock_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 ID" },
            enabled: { type: "boolean", description: "true=启用 false=暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        // #342：宿主 `create_validate_decisions_cron` 的接缝面 —— 创建决策校验定时任务（写）。
        name: "stock_create_validate_decisions_cron",
        description: "创建决策校验定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_validate_decisions_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式（缺省 0 6 * * *）" },
            minConfidenceThreshold: { type: "integer", description: "最低置信度 0-100（缺省 0）" },
            reflectionDepth: { type: "string", description: "light | deep（缺省 light）" },
            enabled: { type: "boolean", description: "缺省启用" },
          },
        },
      },
      {
        // #342：宿主 `list_validate_decisions_crons` 的接缝面 —— 决策校验定时任务列表（读）。
        name: "stock_list_validate_decisions_crons",
        description: "决策校验定时任务列表（经宿主 stock.analysis 接缝）",
        command: "worker://stock_list_validate_decisions_crons",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #342：宿主 `toggle_validate_decisions_cron` 的接缝面 —— 开关决策校验定时任务（写）。
        name: "stock_toggle_validate_decisions_cron",
        description: "开关决策校验定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_validate_decisions_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 ID" },
            enabled: { type: "boolean", description: "true=启用 false=暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        // #342：宿主 `update_validate_decisions_cron` 的接缝面 —— 更新决策校验定时任务配置（写）。
        name: "stock_update_validate_decisions_cron",
        description: "更新决策校验定时任务配置（经宿主 stock.analysis 接缝）",
        command: "worker://stock_update_validate_decisions_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 ID" },
            cronExpression: { type: "string", description: "cron 表达式（5 段）" },
            minConfidenceThreshold: { type: "integer", description: "最低置信度 0-100" },
            reflectionDepth: { type: "string", description: "light | deep" },
          },
          required: ["id", "cronExpression", "minConfidenceThreshold", "reflectionDepth"],
        },
      },
      {
        // #342：宿主 `create_batch_reflection_cron` 的接缝面 —— 创建批量反思定时任务（写）。
        name: "stock_create_batch_reflection_cron",
        description: "创建批量反思定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_batch_reflection_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式（缺省 0 18 * * *）" },
            period: { type: "string", description: "ultra_short | short | mid | long（缺省全部档位）" },
            dueOnly: { type: "boolean", description: "true=到期才反思" },
            maxCount: { type: "integer", description: "单批最多条数" },
            enabled: { type: "boolean", description: "缺省启用" },
          },
        },
      },
      {
        // #342：宿主 `create_watchlist_scan_cron` 的接缝面 —— 创建自选股扫描定时任务（写）。
        name: "stock_create_watchlist_scan_cron",
        description: "创建自选股扫描定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_create_watchlist_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式（5 段）" },
            enabled: { type: "boolean", description: "缺省启用" },
          },
          required: ["cronExpression"],
        },
      },
      {
        // #338：宿主 `get_mover_recall_analysis_batch` 的接缝面 —— 批次状态（只读）。
        name: "stock_get_mover_recall_analysis_batch",
        description: "查询漏检补分析批次状态（经宿主 stock.analysis 接缝，只读；找不到回 null）",
        command: "worker://stock_get_mover_recall_analysis_batch",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            batchId: { type: "string", description: "批次 ID" },
          },
          required: ["batchId"],
        },
      },
      {
        // #338：宿主 `list_mover_recall_analysis_batches` 的接缝面 —— 批次历史（只读）。
        name: "stock_list_mover_recall_analysis_batches",
        description: "查询漏检补分析批次历史（摘要不含逐票明细；经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_list_mover_recall_analysis_batches",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "最多返回条数（缺省 50，上限 200）" },
            statusFilter: { type: "string", description: "按状态过滤（running/completed/partial/failed/interrupted）" },
          },
        },
      },
      {
        // #338：宿主 `run_mover_recall_analysis` 的接缝面 —— 发起批次（写 op，
        // 宿主原档 Caution ⇒ 等阶 workspace-write）。fire-and-forget：返回 batchId。
        name: "stock_run_mover_recall_analysis",
        description:
          "发起一次妖股漏检 as-of 历史补分析批次（立即返回 batchId，后台执行，经宿主 stock.analysis 接缝，写库）",
        command: "worker://stock_run_mover_recall_analysis",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            batchLimit: { type: "integer", description: "单批最多补分析事件数（1-100）" },
            maxUnexplainedShare: { type: "number", description: "核查轮次 unexplained 占比阈值（0-1）" },
            runId: { type: "string", description: "仅处理指定核查轮次；省略 = 跨全部轮次" },
          },
          required: ["batchLimit", "maxUnexplainedShare"],
        },
      },
      {
        // #336：宿主 `quant_run_get` 的接缝面 —— 回测运行记录读取（只读）。
        name: "stock_quant_run_get",
        description: "按 runId 读取量化回测运行记录（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_quant_run_get",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            runId: { type: "string", description: "回测运行记录 ID" },
          },
          required: ["runId"],
        },
      },
      {
        // #336：宿主 `quant_strategies_list` 的接缝面 —— 量化策略列表（只读、零入参）。
        name: "stock_quant_strategies_list",
        description: "列出所有量化策略（内置 + Rhai 注册）（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_quant_strategies_list",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #336：宿主 `quant_metrics_compare` 的接缝面 —— 多 run 指标对比（只读）。
        name: "stock_quant_metrics_compare",
        description: "对比多条回测运行的绩效指标并给出各指标最佳 run（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_quant_metrics_compare",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            runIds: {
              type: "array",
              description: "要对比的回测运行 ID 列表",
              items: { type: "string" },
            },
          },
          required: ["runIds"],
        },
      },
      {
        // #336：宿主 `quant_strategy_register_rhai` 的接缝面 —— 注册 Rhai 策略（写 op，
        // 宿主原档 Caution ⇒ 等阶 workspace-write）。宿主 `quant_backtest_run` 不迁：
        // 分钟级长跑，命令帧 30s / 接缝等待 15s 双预算都装不下。
        name: "stock_quant_strategy_register_rhai",
        description: "注册或更新一个 Rhai 量化策略（经宿主 stock.analysis 接缝，写库）",
        command: "worker://stock_quant_strategy_register_rhai",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            name: { type: "string", description: "策略名称（upsert 时为唯一键）" },
            version: { type: "string", description: "策略版本" },
            description: { type: "string", description: "策略描述" },
            scriptSource: { type: "string", description: "Rhai 脚本源码" },
            params: {
              type: "object",
              description: "策略参数（键值对，序列化为 JSON 落库）",
              additionalProperties: true,
            },
            walkForwardEnabled: { type: "boolean", description: "是否启用 Walk-Forward 验证" },
            upsert: { type: "boolean", description: "按 name 已存在时更新而非报错" },
          },
          required: [
            "name",
            "version",
            "scriptSource",
            "params",
            "walkForwardEnabled",
            "upsert",
          ],
        },
      },
      {
        // #333：宿主 `generate_monthly_report` 的接缝面 —— 月度投资报告（只读）。
        name: "stock_generate_monthly_report",
        description: "生成指定年月的投资报告（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_generate_monthly_report",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            year: { type: "integer", description: "年份（如 2026）" },
            month: { type: "integer", description: "月份 1–12" },
          },
          required: ["year", "month"],
        },
      },
      {
        // #333：宿主 `macro_data_snapshot` 的接缝面 —— 宏观数据快照（只读、零入参）。
        name: "stock_macro_data_snapshot",
        description: "获取宏观经济数据快照（经宿主接缝，只读，无参数）",
        command: "worker://stock_macro_data_snapshot",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #332：宿主 `get_node_feedbacks` 的接缝面（通用版，与别名族不同命令）。
        name: "stock_get_node_feedbacks",
        description: "获取指定节点的质量反馈历史（可按 nodeType/nodeId 过滤，经宿主接缝，只读）",
        command: "worker://stock_get_node_feedbacks",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            nodeType: { type: "string", description: "节点类型（如 analyst）" },
            nodeId: { type: "string", description: "节点标识" },
            limit: { type: "integer", description: "返回条数上限（缺省 50）" },
            onlyIssues: { type: "boolean", description: "只返回有问题的反馈" },
          },
        },
      },
      {
        // #332：宿主 `get_node_feedback_stats` 的接缝面（通用版）。
        name: "stock_get_node_feedback_stats",
        description: "获取指定节点反馈的统计与进化判定（经宿主接缝，只读）",
        command: "worker://stock_get_node_feedback_stats",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            nodeType: { type: "string", description: "节点类型" },
            nodeId: { type: "string", description: "节点标识" },
          },
          required: ["nodeType", "nodeId"],
        },
      },
      {
        // #331：宿主 `get_evolution_drift_timeline` 的接缝面 —— 权重时间线（只读）。
        name: "stock_get_evolution_drift_timeline",
        description: "查询某条 (策略, 周期) 的权重演化时间线（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_get_evolution_drift_timeline",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            strategyId: { type: "string", description: "策略标识" },
            period: { type: "string", description: "周期（如 mid / long）" },
            limit: { type: "integer", description: "返回条数上限（缺省 60）" },
          },
          required: ["strategyId", "period"],
        },
      },
      {
        // #331：宿主 `get_agreement_score_history` 的接缝面 —— 一致性分数趋势（只读）。
        name: "stock_get_agreement_score_history",
        description: "查询近期决策一致性分数趋势（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_get_agreement_score_history",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "返回条数上限（缺省 50）" },
          },
        },
      },
      {
        // #330：宿主 `get_latest_analyses_for_stocks` 的接缝面。逐票复用单数版 SQL 判据；
        // 出参 = `{code: Summary|null}` 对象（key 恒在，值是 null 表示该股无记录）。
        name: "stock_get_latest_analyses_for_stocks",
        description: "批量查询多只个股最近一次完整分析的决策摘要（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_get_latest_analyses_for_stocks",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCodes: { type: "array", items: { type: "string" }, description: "股票代码列表" },
            asOfDate: { type: "string", description: "时间旅行截止日（YYYY-MM-DD）；省略则取最新" },
          },
          required: ["stockCodes"],
        },
      },
      {
        // #328：宿主 `list_param_suggestions` 的接缝面；两个参数都可省，缺省返回全部最近建议。
        name: "stock_list_param_suggestions",
        description: "列出最近未处理的参数调整建议（经宿主 stock.analysis 接缝，只读）",
        command: "worker://stock_list_param_suggestions",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码；省略则返回所有股票的建议" },
          },
        },
      },
      {
        // #329：宿主 `discover_stock_candidates` 的接缝面 —— **零新 op 的插件侧编排**：
        // 宇宙（热门股前 30 + 行业龙头前 10 + 引擎兜底池）走两现成 `market.quote` op +
        // `recommender::FALLBACK_STOCKS`，行情逐票 `get_quote`，打分走引擎纯函数
        // `StockScreener::score_candidate`（宿主原命令已退役，纯函数是唯一权威）。
        name: "stock_discover_stock_candidates",
        description: "从全市场发现热门候选标的并按量化信号打分（经行情接缝逐票取数 + 引擎纯函数打分，只读）",
        command: "worker://stock_discover_stock_candidates",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #329：宿主 `invalidate_recommendation_cache` 的接缝面。⚠ 执行体必须留宿主：
        // `recommender::invalidate_cache()` 失效的是**当前进程**的进程内缓存，worker 直调
        // 只会清掉插件进程副本，而宿主 `recommend_stocks_multi` 等活消费方用的是宿主副本
        // （§183 表的裁定依据）。宿主原档 Safe ⇒ 等阶 read-only；接缝 op 级要求包有 write
        // （变更宿主进程状态），本包已声明 write ⇒ 行为等阶。
        name: "stock_invalidate_recommendation_cache",
        description: "失效荐股推荐缓存（宿主进程内缓存立即重算，供设置变更后调用，无参数）",
        command: "worker://stock_invalidate_recommendation_cache",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // #329：宿主 `run_self_improving_stock_analysis` 的接缝面。执行体在宿主 agent crate
        // （consumer 层，插件不可链接）；宿主原档 Caution ⇒ 等阶 workspace-write。
        // 出参 partial: true 表示中途出错但带回已完成轮次（宿主原命令同口径）。
        name: "stock_run_self_improving_stock_analysis",
        description: "运行自改进股票分析循环（多轮执行-评估-改进，经宿主接缝；返回最终报告与轮次）",
        command: "worker://stock_run_self_improving_stock_analysis",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            task: { type: "string", description: '分析任务描述（如 "分析 600519.SH"）' },
            maxRounds: { type: "integer", description: "最大改进轮数（缺省 3）" },
          },
          required: ["task"],
        },
      },
      {
        // 清单不复刻 DTO（同 stock_market_mainline_batch_upsert 宽形态先例）：形状权威在
        // `commands/wf_des.rs` 的 WfDesRequest（camelCase，四字段全必填），此处只锁最外层。
        name: "stock_wf_des_integration",
        description: "Walk-Forward 回测 + DES 对比仿真（纯计算，经宿主 stock.analysis 接缝）",
        command: "worker://stock_wf_des_integration",
        inputSchema: {
          type: "object",
          additionalProperties: true,
          properties: {
            klines: { type: "array", description: "历史 K 线（Bar 数组，宿主拒空集）" },
            wfConfig: {
              type: "object",
              description: "Walk-Forward 配置（trainDays/testDays/stepDays/riskFreeAnnual）",
            },
            desConfig: { type: "object", description: "DES 模拟配置" },
            strategyName: { type: "string", description: "策略名（当前仅 ma_cross）" },
          },
          required: ["klines", "wfConfig", "desConfig", "strategyName"],
        },
      },
      // ── 批 1a（#35）：十二股票 cron 配置命令，经 `stock.analysis` 接缝 ──
      // 三条 create 回建好的任务、九条管理面回 `null`；配置归一与响应形状的权威在主壳
      // `commands/recommendation_cron.rs` / `commands/pool_scan.rs` 的无 State 核心。
      // ⚠ 九条写命令（三 create + 三 toggle + 三 delete）必须 `requiredPermission:
      // "workspace-write"`（否则智能体面按 `Safe` 暴露、确认弹窗形同虚设）；三条 list
      // 是读 op，宿主 `required_permission_for` 与清单都不加闸。
      {
        name: "stock_create_recommendation_cron",
        description: "创建荐股定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_recommendation_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            name: { type: "string", description: "任务名" },
            cronExpression: { type: "string", description: "cron 表达式（camelCase）" },
            periods: {
              type: "array",
              items: { type: "string", enum: ["ultra_short", "short", "mid", "long"] },
              description: "投资周期档位，不能为空",
            },
            minConfidence: { type: "integer", description: "最低置信度 0-100" },
            topN: { type: "integer", description: "每次推送只数，必须大于 0" },
          },
          required: ["name", "cronExpression", "periods", "minConfidence", "topN"],
        },
      },
      {
        // 无参：接缝 provider 的 `require_no_params` 会拒非空入参。
        name: "stock_list_recommendation_crons",
        description: "列出荐股定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_recommendation_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_recommendation_cron",
        description: "启停荐股定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_recommendation_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_recommendation_cron",
        description: "删除荐股定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_recommendation_cron",
        // 宿主原命令是 `#[agent_command(safety = Caution)]`（PLAN §142.1），已于 §142 退役；
        // §142.2 判过「档位等阶」：workspace-write 投影 Caution ⇒ 智能体面授权强度不变。
        // **不随另外两条升 danger-full-access**：那两条宿主是 Dangerous（升档＝关旁路），
        // 本条升档会把 Caution 变 Dangerous ⇒ 智能体彻底失去删荐股 cron 的能力（砍功能，非补洞）。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      {
        // 三参可选：缺省 = 原链 / `0 16 * * 1-5` / 启用。
        name: "stock_create_trend_screening_cron",
        description: "创建趋势智选定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_trend_screening_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式，缺省 0 16 * * 1-5" },
            enabled: { type: "boolean", description: "缺省 true" },
            workflowId: {
              type: "string",
              description: "链 id：serenity-screening（原链，缺省）/ serenity-screening-fast（快速链）",
            },
          },
        },
      },
      {
        name: "stock_list_trend_screening_crons",
        description: "列出趋势智选定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_trend_screening_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_trend_screening_cron",
        description: "启停趋势智选定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_trend_screening_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_trend_screening_cron",
        description: "删除趋势智选定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_trend_screening_cron",
        // 宿主等价命令 `delete_trend_screening_cron` 是 `#[agent_command(safety = Dangerous)]`，
        // 按 PLAN §142.3 保留宿主版；只有 danger-full-access 投影成 Dangerous
        // （`plugin_command_safety`）。标 workspace-write 会投影 Caution ⇒ 智能体经插件名
        // 绕过宿主的「禁止」（两个名字不同，`merge_plugin_command_specs` 的宿主优先去重不命中）。
        // 档位等价优先于最小权限；包级 `execute` 的依据见上方 permissions 注释。
        requiredPermission: "danger-full-access",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      {
        // 六参可选：缺省值在宿主无 State 核心内（`PoolScanConfig::default()`）。
        name: "stock_create_pool_scan_cron",
        description: "创建候选池扫描定时任务（经宿主 stock.analysis 接缝写入调度器任务表）",
        command: "worker://stock_create_pool_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            cronExpression: { type: "string", description: "cron 表达式，缺省 0 17 * * *" },
            lookbackDays: { type: "integer", description: "进池窗口天数，0 = 全池" },
            maxStocks: { type: "integer", description: "最多分析只数，0 = 不限" },
            minConfidence: { type: "integer", description: "最低置信度 0-100" },
            enabled: { type: "boolean", description: "缺省 true" },
            maxPerRun: { type: "integer", description: "单次运行硬上限" },
            runDeadlineSeconds: { type: "integer", description: "单次运行截止秒数" },
          },
        },
      },
      {
        name: "stock_list_pool_scan_crons",
        description: "列出候选池扫描定时任务（经宿主 stock.analysis 接缝，只读，无参数）",
        command: "worker://stock_list_pool_scan_crons",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "stock_toggle_pool_scan_cron",
        description: "启停候选池扫描定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_toggle_pool_scan_cron",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "任务 id" },
            enabled: { type: "boolean", description: "true = 启用，false = 暂停" },
          },
          required: ["id", "enabled"],
        },
      },
      {
        name: "stock_delete_pool_scan_cron",
        description: "删除候选池扫描定时任务（经宿主 stock.analysis 接缝）",
        command: "worker://stock_delete_pool_scan_cron",
        // 同 stock_delete_trend_screening_cron：宿主等价命令 `delete_pool_scan_cron` 是
        // `Dangerous` 且按 §142.3 保留 ⇒ 清单必须 danger-full-access，否则插件名成为
        // 「确认即可删」的旁路。包级 `execute` 的依据见上方 permissions 注释。
        requiredPermission: "danger-full-access",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { id: { type: "string", description: "任务 id" } },
          required: ["id"],
        },
      },
      // ── 批 2a（#35）：十一条行情只读命令，一律经宿主 market.quote 接缝 ──
      // 不声明 requiredPermission ⇒ 投影 Safe，与宿主同名命令的 `safety = Safe` 逐条等阶。
      // 入参是 camelCase（禁区 13），插件路由表翻成接缝要求的 snake_case。
      {
        name: "stock_get_research_reports",
        description: "获取研究报告（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_research_reports",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_concept_blocks",
        description: "获取个股所属概念板块（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_concept_blocks",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_announcements",
        description: "获取公司公告（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_announcements",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_earnings_calendar",
        description: "获取财报披露日历（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_earnings_calendar",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_concept_board_members",
        description: "获取概念板块成分股（经宿主 market.quote 接缝，只读；入参是板块代码）",
        command: "worker://stock_get_concept_board_members",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { boardCode: { type: "string", description: "板块代码，如 BK0475" } },
          required: ["boardCode"],
        },
      },
      {
        name: "stock_get_cls_flash",
        description: "获取财联社快讯（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_cls_flash",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_peers",
        description: "获取同行业对比（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_peers",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_option_pcr",
        description: "获取期权 PCR（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_option_pcr",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_lockup_schedule",
        description: "获取限售解禁时间表（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_lockup_schedule",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_dividend_records",
        description: "获取分红记录（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_dividend_records",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_financials",
        description: "获取股票财务数据（经宿主 market.quote 接缝，只读；对比面板使用）",
        command: "worker://stock_get_financials",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      // ── 批 2c（#262）：五条全市场榜单/盘面只读命令，零入参，同走 market.quote 接缝 ──
      // `stock_get_index_quotes` 的接缝 op 是 `get_index_quotes_outcome`（返回 DataOutcome
      // 信封，与宿主命令同形）；清单面命令名与宿主 `get_index_quotes` 对齐。
      {
        name: "stock_get_hot_stocks",
        description: "获取热门股票榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_hot_stocks",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_industry_ranking",
        description: "获取行业排名榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_industry_ranking",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_index_quotes",
        description: "获取指数行情（经宿主 market.quote 接缝，只读；无入参，返回 DataOutcome 信封）",
        command: "worker://stock_get_index_quotes",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_market_dragon_tiger",
        description: "获取全市场龙虎榜（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_market_dragon_tiger",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_get_north_bound_flow",
        description: "获取北向资金（经宿主 market.quote 接缝，只读；无入参）",
        command: "worker://stock_get_north_bound_flow",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      // ── 候选 A（#35 行情资讯直映批）：十条宿主只读命令，全部经宿主 market.quote 接缝 ──
      // 不声明 requiredPermission ⇒ 投影 Safe，与宿主同名命令的 `safety = Safe` 逐条等阶。
      // 宿主 `search_stock` 的 `market` 过滤参不在接缝 op 上 ⇒ 清单不声明该键；
      // 国际/外汇/基准 K 线与宿主命令同不收复权参（接缝 adj_type 缺省）。
      {
        name: "stock_search_stock",
        description: "搜索股票（按关键词匹配代码与名称；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_search_stock",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { keyword: { type: "string", description: "搜索关键词，如「茅台」或 600519" } },
          required: ["keyword"],
        },
      },
      {
        name: "stock_search_news",
        description: "搜索财经新闻（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_search_news",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            keyword: { type: "string", description: "搜索关键词" },
            limit: { type: "integer", minimum: 1, description: "取多少条，缺省 20，上限 100" },
          },
          required: ["keyword"],
        },
      },
      {
        name: "stock_search_concept_boards",
        description: "搜索概念板块（按关键词匹配板块名；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_search_concept_boards",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { keyword: { type: "string", description: "搜索关键词，如「人工智能」" } },
          required: ["keyword"],
        },
      },
      {
        name: "stock_get_social_sentiment",
        description: "获取社交舆情（股吧/雪球讨论热度与情感倾向；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_social_sentiment",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_limit_up_pool",
        description: "获取涨停池（date 缺省取当下；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_limit_up_pool",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { date: { type: "string", description: "交易日 YYYY-MM-DD，缺省=当下" } },
        },
      },
      {
        name: "stock_get_international_stock_quote",
        description: "获取国际个股实时行情（美股/港股等；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_international_stock_quote",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "国际标的代码，如 AAPL / 00700" },
          },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_international_stock_kline",
        description: "获取国际个股 K 线序列（固定未复权，与宿主命令同档；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_international_stock_kline",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "国际标的代码，如 AAPL / 00700" },
            period: { type: "string", description: "周期，如 daily / weekly / 60min" },
            limit: { type: "integer", minimum: 1, description: "取多少根" },
          },
          required: ["stockCode", "period", "limit"],
        },
      },
      {
        name: "stock_get_forex_kline",
        description: "获取外汇 K 线序列（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_forex_kline",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            pair: { type: "string", description: "货币对，如 USD/JPY" },
            period: { type: "string", description: "周期，如 daily / weekly / 60min" },
            limit: { type: "integer", minimum: 1, description: "取多少根" },
          },
          required: ["pair", "period", "limit"],
        },
      },
      {
        name: "stock_get_benchmark_kline",
        description: "获取基准指数 K 线序列（经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_benchmark_kline",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            benchmarkCode: { type: "string", description: "基准指数代码，如 HSI / SPX" },
            period: { type: "string", description: "周期，如 daily / weekly / 60min" },
            limit: { type: "integer", minimum: 1, description: "取多少根" },
          },
          required: ["benchmarkCode", "period", "limit"],
        },
      },
      {
        name: "stock_get_consensus_eps",
        description: "获取一致预期 EPS（分析师盈利预测汇总；经宿主 market.quote 接缝，只读）",
        command: "worker://stock_get_consensus_eps",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: { stockCode: { type: "string", description: "6 位股票代码，如 600519" } },
          required: ["stockCode"],
        },
      },
      // ── F1 首批（#307）：三条「只有智能体在用」的宿主只读命令迁 T2 命令面 ──
      // 三条都**显式声明** `requiredPermission: "read-only"`：投影档位与候选 A 的「未声明 ⇒ Safe」
      // 等价（`plugin_command_safety`：None 与 "read-only" 同映射 Safe），但显式写死让
      // 「只读走 read-only」的批次纪律可审计，不靠缺省值兜底。三条都没有前端调用点
      // （消费方是 LLM 工具面），故清单之外的前端半（src/** 的 invokePluginCommand / dev mock / i18n）本批零改动。
      {
        // 宿主 `batch_get_quotes` 退役后的取数面。入参窗口与宿主逐字对齐：`codes` **无条数上限**
        // （宿主侧只有 `batch_get_klines` 带 50 条闸，本条历史上没有），三个可选档缺省时全部
        // 落 provider 侧默认（单只 8s / 不提前终止）；`totalTimeoutMs` 与宿主同口径 —— 接受但不生效。
        name: "stock_batch_get_quotes",
        description:
          "逐只并发批量获取实时行情（经宿主 market.quote 接缝，只读；出参 success/failures 双 map 按 code 归因）",
        command: "worker://stock_batch_get_quotes",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            codes: {
              type: "array",
              items: { type: "string" },
              description: "股票代码数组，如 600519 / 000001",
            },
            perStockTimeoutMs: { type: "integer", minimum: 1, description: "单只取数超时（毫秒），缺省 8000" },
            totalTimeoutMs: {
              type: "integer",
              minimum: 1,
              description: "整体超时（毫秒）；与宿主同口径：接受但不生效",
            },
            maxFailures: {
              type: "integer",
              minimum: 0,
              description: "失败数超过该阈值即提前终止；缺省 0 = 不提前终止",
            },
          },
          required: ["codes"],
        },
      },
      {
        // 宿主 `batch_get_financials` 退役后的取数面。入参只有一个 `codes`，与宿主命令逐字同形
        // （原命令也没有超时档 —— 单只 15s 固定在 provider 侧）。
        name: "stock_batch_get_financials",
        description:
          "逐只并发批量获取财务报告序列（经宿主 market.quote 接缝，只读；出参 success/failures 双 map 按 code 归因）",
        command: "worker://stock_batch_get_financials",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            codes: {
              type: "array",
              items: { type: "string" },
              description: "股票代码数组，如 600519 / 000001",
            },
          },
          required: ["codes"],
        },
      },
      {
        // 宿主 `list_watched_quotes` 退役后的取数面：走既有 `quote.watcher` 的 `list_watched` op
        // （零入参，解包 `{codes:[…]}` 成裸数组由插件侧完成，出参形状与原宿主命令一致）。
        // 与原命令的唯一差别是**顺序**：接缝侧按升序 sort（裁定 J6），原命令是 HashMap 键序（本就不可复现）。
        name: "stock_list_watched_quotes",
        description: "列出当前实时行情监听集合（经宿主 quote.watcher 接缝，只读；无入参，出参为代码数组）",
        command: "worker://stock_list_watched_quotes",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      // ── #311：四条零新 op 的纯智能体工具（宿主同名命令同批退役）──
      // 档位按**接缝 op 的读写性质**定（与 #307 三条同口径）：三条读 op 显式 read-only，
      // 唯一写命令走 `quote.watcher::watch`（宿主 `required_permission_for` 判 Write）
      // ⇒ 必须 workspace-write，否则智能体面按 Safe 暴露、确认弹窗形同虚设。
      // ⚠ 两处与宿主原命令的档位差已登记 PLAN §176：两条读命令宿主标 Caution（纯读，
      // 降为 read-only）；写命令宿主标 Safe（升为 workspace-write，收紧）。
      {
        name: "stock_get_portfolio_risk",
        description: "获取组合风险指标（持仓经宿主 trading.engine 接缝取，算法跑 analysis-engine 权威实现；无入参）",
        command: "worker://stock_get_portfolio_risk",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_compare_trade_with_analysis",
        description:
          "对比实际交易出场价与交易前分析的预测价位（经宿主 trading.engine 接缝；交易不存在或无前置分析时报错）",
        command: "worker://stock_compare_trade_with_analysis",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            tradeId: { type: "string", description: "交易记录 id（trades 表主键）" },
          },
          required: ["tradeId"],
        },
      },
      {
        name: "stock_scan_sector_coherence",
        description: "批量扫描板块联动情况（本体索引 + 逐票实时行情，按联动强度绝对值降序返回报告数组）",
        command: "worker://stock_scan_sector_coherence",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            conceptIds: {
              type: "array",
              items: { type: "string" },
              description: "概念板块 id 数组，如 concept_ai；空数组返回空结果",
            },
          },
          required: ["conceptIds"],
        },
      },
      {
        // `priority` **不加 enum**：宿主原命令认 background/bg 且把任何未知串静默回落
        // Active，加 enum 等于把这条宽容变成 IPC 层拒收（行为回退）。归一在插件侧
        // `watch_priority_arg`，接缝只收 active/background。
        name: "stock_set_quote_watch_priority",
        description: "设置单只股票的实时行情监控优先级（经宿主 quote.watcher 接缝写入监视集合）",
        command: "worker://stock_set_quote_watch_priority",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "6 位股票代码，如 600519" },
            priority: {
              type: "string",
              description: "监控优先级：background/bg = 后台低频，其余值（含 active）= 活跃高频",
            },
          },
          required: ["stockCode", "priority"],
        },
      },
      // ── F1 余三条（#35 / #321）：宿主同名命令同批退役，走新 op ──
      // `stock_list_financial_snapshots` 读本地快照表原始行，与 `stock_get_financials`
      // （vendor 财报）不等价 ⇒ 独立 op；`stock_refresh_trading_calendar` 的东财报表
      // 2026-09-27 起已下线，与被退役宿主命令等价失败；`stock_sweep_daily_snapshots`
      // 是触发口（执行体 `run_daily_snapshot_sweep` 留宿主，长任务不进 worker 帧）。
      // 档位按接缝 op 的读写性质（#311 同口径）：两条读 op 显式 read-only，写 op
      // 走宿主 `required_permission_for` 判 Write ⇒ 必须 workspace-write。
      {
        name: "stock_list_financial_snapshots",
        description: "列估值快照原始行（本地 financial_snapshots 表某股在区间内的全部快照；与财报接口不等价）",
        command: "worker://stock_list_financial_snapshots",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "6 位股票代码，如 600519" },
            start: { type: "string", description: "起始日期（含，YYYY-MM-DD），缺省不设下界" },
            end: { type: "string", description: "结束日期（含，YYYY-MM-DD），缺省不设上界" },
          },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_refresh_trading_calendar",
        description: "从东方财富 API 刷新交易日历（该报表已下线，调用返回与退役宿主命令相同的错误）",
        command: "worker://stock_refresh_trading_calendar",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_sweep_daily_snapshots",
        description: "触发每日收盘快照采集（网络抓取 + 落 financial_snapshots 表；盘中调用取上一交易日口径）",
        command: "worker://stock_sweep_daily_snapshots",
        requiredPermission: "workspace-write",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      // ── F2 余四条（#35 / #322）：宿主同名 Tauri 命令同批退役，走 trading.engine 新 op ──
      // 档位按接缝 op 的读写性质：两条写 op（更新持仓 / 导入 VLM 持仓）显式
      // workspace-write，两条零入参读 op 显式 read-only（#311 同口径）。
      {
        name: "stock_get_position_limits",
        description: "获取全局仓位限制配置（面板 pos_max_* 生效值，回落 20/10/40）",
        command: "worker://stock_get_position_limits",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_update_portfolio_holding",
        description: "更新单条持仓的数量与成本（按 id 定位；写 portfolio_holdings 表）",
        command: "worker://stock_update_portfolio_holding",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            id: { type: "string", description: "持仓记录 id" },
            shares: { type: "number", description: "新持仓数量（股）" },
            avgCost: { type: "number", description: "新平均成本" },
          },
          required: ["id", "shares", "avgCost"],
        },
      },
      {
        name: "stock_import_portfolio_from_vlm",
        description: "批量导入 VLM 识别的持仓（replaceExisting=true 会先清空旧持仓，不可逆）",
        command: "worker://stock_import_portfolio_from_vlm",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            rawVlmOutput: { type: "string", description: "VLM 识别的原始输出文本" },
            replaceExisting: { type: "boolean", description: "导入前清空旧持仓（缺省不清）" },
          },
          required: ["rawVlmOutput"],
        },
      },
      {
        name: "stock_run_portfolio_stress_test",
        description: "运行组合压力测试（集中度 + β 估计 + 多情景模拟；纯计算，无 DB 副作用）",
        command: "worker://stock_run_portfolio_stress_test",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      // ── F6 别名族（#35 / #322）：宿主 Tauri 命令同批退役；别名形状逐字保留 ──
      // 泛化形邻居（evolve_node_command / save_node_feedback / get_node_feedbacks /
      // get_node_feedback_stats）不退役，宿主继续提供。写 op 两条显式 workspace-write。
      {
        name: "stock_evolve_analyst_command",
        description: "触发分析师节点自我进化（基于历史反馈统计优化 Prompt/配置；别名，硬编码 Analyst 节点）",
        command: "worker://stock_evolve_analyst_command",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analystId: { type: "string", description: "分析师节点 ID，如 a-technical" },
          },
          required: ["analystId"],
        },
      },
      {
        name: "stock_get_analyst_evolution_status_command",
        description: "获取分析师节点当前进化状态（不触发进化；别名）",
        command: "worker://stock_get_analyst_evolution_status_command",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analystId: { type: "string", description: "分析师节点 ID，如 a-technical" },
          },
          required: ["analystId"],
        },
      },
      {
        name: "stock_get_node_evolution_status_command",
        description: "获取任意节点当前进化状态（泛化形：nodeType + nodeId，未知类型兜 other）",
        command: "worker://stock_get_node_evolution_status_command",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            nodeType: {
              type: "string",
              description: "节点类型：analyst/debate/decision/tool/valuation/risk/other",
            },
            nodeId: { type: "string", description: "节点 ID" },
          },
          required: ["nodeType", "nodeId"],
        },
      },
      {
        name: "stock_save_analyst_feedback",
        description: "保存分析师反馈（别名：13 字段，无 node_type —— 宿主按 Analyst 节点补齐）",
        command: "worker://stock_save_analyst_feedback",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analystId: { type: "string", description: "分析师节点 ID" },
            reportId: { type: "string", description: "关联报告 ID" },
            stockCode: { type: "string", description: "6 位股票代码" },
            executionId: { type: "string", description: "关联执行 ID" },
            qualityScore: { type: "integer", description: "质量分（0-100）" },
            grade: { type: "string", description: "评级，如 A/B/C" },
            issueCount: { type: "integer", description: "问题数" },
            warningCount: { type: "integer", description: "警告数" },
            goodCount: { type: "integer", description: "亮点数" },
            checksJson: { type: "string", description: "检查项明细（JSON 字符串）" },
            bullScore: { type: "number", description: "多头评分（可选）" },
            bearScore: { type: "number", description: "空头评分（可选）" },
            confidence: { type: "number", description: "置信度（可选）" },
            scoreConsistent: { type: "boolean", description: "评分自洽" },
            directionConsistent: { type: "boolean", description: "方向自洽" },
          },
          required: [
            "analystId",
            "reportId",
            "stockCode",
            "executionId",
            "qualityScore",
            "grade",
            "issueCount",
            "warningCount",
            "goodCount",
            "checksJson",
            "scoreConsistent",
            "directionConsistent",
          ],
        },
      },
      {
        name: "stock_get_analyst_feedbacks",
        description: "获取分析师反馈历史（别名：单键 analystId 必填，limit/onlyIssues 可选）",
        command: "worker://stock_get_analyst_feedbacks",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analystId: { type: "string", description: "分析师节点 ID" },
            limit: { type: "integer", description: "返回条数上限（缺省 50）" },
            onlyIssues: { type: "boolean", description: "只看有问题的记录（缺省 false）" },
          },
          required: ["analystId"],
        },
      },
      {
        name: "stock_get_analyst_feedback_stats",
        description: "获取分析师反馈统计（含 needs_evolution 进化触发判定；别名）",
        command: "worker://stock_get_analyst_feedback_stats",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            analystId: { type: "string", description: "分析师节点 ID" },
          },
          required: ["analystId"],
        },
      },
      // ── #323 T2 五命令批迁（子批②③④）：宿主同名命令同批退役，走新 op ──
      // 档位按接缝 op 的读写性质（#321/#322 同口径）：写 op 显式 workspace-write
      // （三段持久效应：脚本缓存 / 共享内存 / 模板变量），两条读 op 显式 read-only。
      {
        name: "stock_save_neodata_token",
        description: "保存 NeoData token（写脚本缓存并立即生效，持久化到工作流模板变量）",
        command: "worker://stock_save_neodata_token",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            token: { type: "string", description: "NeoData token，不能为空" },
          },
          required: ["token"],
        },
      },
      {
        name: "stock_query_decision_backtest",
        description: "查询决策回测分析（有 outcome 且有决策动作的分析记录对比数据，按日期降序）",
        command: "worker://stock_query_decision_backtest",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            limit: { type: "integer", description: "取数上限，宿主缺省 100、上界 500" },
          },
        },
      },
      {
        // 配置校验面：宿主原命令 `portfolio_backtest_run` 的实际语义是校验 no-op
        // （PortfolioEngine 构造即弃，回测执行需要前端传 K 线，原命令本就不执行）。
        name: "stock_portfolio_backtest_run",
        description: "运行组合回测——配置校验（解析组合配置并校验合法性，返回 status/strategies/message）",
        command: "worker://stock_portfolio_backtest_run",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            configJson: { type: "string", description: "组合配置 JSON 字符串（含 strategies 数组）" },
          },
          required: ["configJson"],
        },
      },
      // ── #324 T2 F7 回测验证三条迁：宿主同名命令同批退役，走新 op ──
      // 档位按接缝 op 的读写性质（#321/#322/#323 同口径）：写 op 显式 workspace-write
      // （写 decision_validations 表），两条读 op 显式 read-only。
      {
        name: "stock_run_decision_backtest",
        description: "跑决策回测（对历史荐股拉 T+N 窗口 K 线回放，写验证记录并聚合命中率/因子 IC 报告）",
        command: "worker://stock_run_decision_backtest",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            tPlusNList: {
              type: "array",
              items: { type: "integer" },
              description: "T+N 验证窗口（缺省 = 四档期望持有交易日权威推导）",
            },
            excludeSynthetic: { type: "boolean", description: "排除 synthetic 兜底 pick（默认 true）" },
            maxPicks: { type: "integer", description: "最大回测 pick 数（默认 200）" },
            dryRun: { type: "boolean", description: "只出报告不写库（预览用）" },
            periodFilter: { type: "string", description: "仅回测指定周期：short | mid | long，缺省全部" },
            includeReplay: { type: "boolean", description: "把回放代一并验证（默认 false，只算实盘分母）" },
          },
        },
      },
      {
        name: "stock_list_decision_validations",
        description: "列出决策验证记录（分页，默认只列实盘代；generation 传某次回放截止日看那一批）",
        command: "worker://stock_list_decision_validations",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "按股票代码过滤" },
            hitOutcome: { type: "string", description: "按命中结果过滤" },
            limit: { type: "integer", description: "分页大小，宿主缺省 100" },
            offset: { type: "integer", description: "分页偏移，宿主缺省 0" },
            generation: { type: "string", description: "回放代截止日（YYYY-MM-DD）；缺省只列实盘代" },
          },
        },
      },
      {
        name: "stock_compute_validation_report",
        description: "计算验证报告（基于已写入的验证记录聚合命中率/因子 IC，带代际与窗口排除计数）",
        command: "worker://stock_compute_validation_report",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            generation: { type: "string", description: "回放代截止日（YYYY-MM-DD）；缺省算实盘出票一代" },
          },
        },
      },
      {
        name: "stock_analyze_sentiment_depth",
        description: "分析情绪深度（对预序列化的历史快照序列做情绪聚合，纯本地计算）",
        command: "worker://stock_analyze_sentiment_depth",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
            stockName: { type: "string", description: "股票名称" },
            historyJson: { type: "string", description: "历史快照数组（SentimentSnapshot[]）的 JSON 字符串" },
          },
          required: ["stockCode", "stockName", "historyJson"],
        },
      },
      {
        name: "stock_factor_analysis_list",
        description: "获取因子分析结果（全量因子清单：id/名称/类别/权重/启停，纯本地计算）",
        command: "worker://stock_factor_analysis_list",
        requiredPermission: "read-only",
        inputSchema: { type: "object", additionalProperties: false, properties: {} },
      },
      {
        name: "stock_generate_fundamentals_report",
        description: "生成基本面分析报告（PE/PB/ROE/同比/估值带/健康度评分，含可选 Markdown）",
        command: "worker://stock_generate_fundamentals_report",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
            includeMarkdown: { type: "boolean", description: "是否同时返回 Markdown 渲染（缺省 true）" },
          },
          required: ["stockCode"],
        },
      },
      {
        name: "stock_get_fundamentals_report_markdown",
        description: "仅获取基本面报告 Markdown 渲染（轻量，不返回结构）",
        command: "worker://stock_get_fundamentals_report_markdown",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            stockCode: { type: "string", description: "股票代码" },
          },
          required: ["stockCode"],
        },
      },
    ],
    // 任务中心任务源声明（PLAN §97.4）：只给「源标识 + 显示名 + 取数命令」，
    // 不给渲染。`command` 必须是上面 commands[] 里已声明的名字 —— 宿主清单校验会硬拦，
    // 这里 `--check` 再自证一次，免得产物装不上才发现。
    taskSources: [
      { id: "stock-workflow-runs", title: "股票工作流执行", command: "list_recent_workflow_runs", color: "volcano" },
      {
        // 与宿主原 `stockAnalysis.adapter.ts` **同 id**（#149 第 3 步删掉宿主那份后不再重名）：
        // 插件源的注册是「宿主优先、重名忽略」，同 id 才谈得上「插件源顶替宿主源」。
        id: "stock-analysis",
        title: "单股分析",
        command: "list_stock_analysis_records",
        color: "gold",
        // 详情槽指向本插件 `ui.json` 的 key（任务 #149 第 2 步的通道）：
        // 只写 key，行 id `plugin.<pluginId>.<key>` 由宿主拼（插件拼得出来就能引用别人的 schema）。
        detailSchema: "stock-analysis-detail",
      },
    ],
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}

/**
 * opc 清单（#36 第 1 批只有 worker；第 4 批接进首条命名命令）。
 *
 * `defaultEnabled: true` 的理由与 stock 同一条（第 402 行那段注释）：前端 OPC 站点页 /
 * 仪表盘的博客列表已改走本插件的命名命令，宿主同名命令已删 ⇒ 内置包不默认启用就等于
 * 该功能静默失效（`resolvePluginForCommand` 解析不到插件）。
 * `pages` 已写（#301 判据④闭合）：一页通道探针证明 pages[] 装载链通；OPC 16 个真实页面
 * 仍住主壳 src/pages/opc/**，迁入本产物属后续批。
 * 仍**不写** `taskSources` / `uiSchema`：OPC 没有任务源贡献，也没有 ui.json 详情槽。
 */

function withRequiredPermission(commands) {
  return commands.map((command) => ({
    ...command,
    requiredPermission: OPC_WRITE_COMMANDS.has(command.name) ? "workspace-write" : "read-only",
  }));
}

function opcManifest({ version, binaryName, sha256 }) {
  // opc 自身已不拥有任何域包（14 份全部分给各域插件），该目录在 Git 中为空 ⇒
  // 干净 checkout 里根本不存在。无条件 readdirSync 会让 Release 的 beforeBuildCommand
  // 以 ENOENT 失败（2026-11-22 实测 6 个桌面 target 全红）；目录不存在等价于零份域包。
  const opcSourceRoot = resolvePluginSourceRoot("opc");
  const opcDomainPackDir = opcSourceRoot ? path.join(opcSourceRoot, "plugin-opc", "domain_packs") : null;
  const domainPackManifests = (fs.existsSync(opcDomainPackDir) ? fs.readdirSync(opcDomainPackDir) : [])
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => path.posix.join(DOMAIN_PACK_DIR, f));
  return {
    name: "opc",
    version,
    description: "OPC 域功能包（#36）：worker 通道 + 130 条命名命令（读 76 条 · 写 54 条）",
    // 同 stock：worker 由宿主 spawn ⇒ 不声明 `subprocess_execution` 会被 enable 直接拒绝。
    // ⚠ `write` 是第二批加的：宿主分派层 `worker.rs` 的 `required_permission_for`
    // 给 `("opc.store","publish_blog_post" | "delete_blog_post" | "create_blog_post")`
    // 三个写 op 登记了 `Write`，而 `has_permission` 是**精确
    // `contains`**（刻意不套 `implies_*`，免得 `subprocess_execution` 这类宽权限悄悄补齐写面
    // 准入）⇒ 清单缺这一条时登记即拒：命令会在接缝调用前落 `PLUGIN_PERMISSION_DENIED`，
    // 表现为「插件装得上、describe 正常、发布恒失败」。权限与 op 登记必须同批。
    permissions: ["subprocess_execution", "write"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    // 前端页面声明（#301）：语义与 stock 段上方四行注释逐字相同（entry 相对安装目录、
    // 宿主 resolve_page_asset 解析、前端只传 (pluginId, pageId)）。键序同 stock：pages
    // 插在 worker 与 commands 之间。
    // 通道探针页：只验证装载通道，不是真实 UI；隐藏侧栏入口（路由仍注册，可直达调试）。
    pages: [{
      id: "opc-home",
      title: "OPC",
      entry: PAGE_ENTRY,
      loading: "bundle",
      hidden: true,
    }],
    // P6.2：一域一 logical manifest；宿主只读投影，不在此复制 worker/命令实现。
    domainPacks: domainPackManifests,
    // `command` 用 `worker://<名>` 占位（同 stock）：worker 通道不读它，但按名发帧的
    // `name` 必须与 worker 里的 `COMMAND_LIST_BLOG_POSTS` / `COMMAND_PUBLISH_BLOG_POST` 逐字一致。
    commands: withRequiredPermission([
      {
        // 先例 #144 的双声明已在 #36 blog 刀收口：宿主 `commands/opc.rs` 的同名 Tauri 命令
        // 已删，本条是发布博客的唯一通道，回宿主的两态枚举串。
        name: "opc_publish_blog_post",
        description: "发布一篇 OPC 博客（经宿主 opc.store 写面；回 published / not_found 两态）",
        command: "worker://opc_publish_blog_post",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "文章 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        // 删除：入参同 publish（平铺 `id`），出参是三态枚举串逐字透传 —— 命令层与宿主
        // provider 各判一半（形状 / 语义），前端对 not_found / protected 自行翻提示。
        name: "opc_delete_blog_post",
        description: "删除一篇 OPC 博客（经宿主 opc.store 写面；回 removed / not_found / protected 三态）",
        command: "worker://opc_delete_blog_post",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "文章 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      // ── #36 批 2：4 条写命令（全部走 opc.store 写面 ⇒ 宿主 requireWrite 闸）──
      // 出参一律是契约的外部标记枚举（`{"transitioned"…}` / `{"updated"…}` / `"removed"` /
      // `"not_found"`）；「缺行」是正常读数，前端按取值翻提示，不按错误码分派。
      // ── #36 批 3：8 条写命令 + 1 条财务报表查询（前 8 条走 opc.store 写面 ⇒ 宿主
      //    requireWrite 闸；`opc_get_financial_report` 是读 op，与八个 list op 同档无闸）──
      // 入参一律 camelCase（与契约 `*Request` DTO 的 serde 键逐字一致，禁区 13）；
      // 数组元素内部字段类型的错由宿主 serde 拒收，命令层只判「是不是对象数组」。
      // ── #32 批 A：三条写命令（客户更新/客户删除/发票删除，均走 opc.store 写面 ⇒
      //    宿主 requireWrite 闸）。update 出参 = 更新后的列裁剪视图；删除出参 = 无载荷。
      //    可选字段「缺省 = 该列不动」；⚠ 清空在本命令线格式上不可表达（与宿主同名
      //    Tauri 命令行为一致）。opc_delete_invoice 删的是域包发票 —— 宿主同名 Tauri
      //    命令删的是交付域发票，两套体系同名不同义。
      {
        name: "opc_update_customer",
        description: "更新 OPC 客户（经宿主 opc.store 写面；回更新后的列裁剪视图）",
        command: "worker://opc_update_customer",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "客户 id（接缝主键；空值由宿主 provider 拒收）" },
            name: { type: "string", description: "新姓名（缺省 = 该列不动）" },
            email: { type: "string", description: "新邮箱（缺省 = 该列不动）" },
            phone: { type: "string", description: "新电话（缺省 = 该列不动）" },
            company: { type: "string", description: "新公司（缺省 = 该列不动）" },
            customerType: {
              type: "string",
              description: "新客户类型（consumer / business；其余值由宿主落 Unknown）",
            },
            country: { type: "string", description: "新国家（缺省 = 该列不动）" },
            region: { type: "string", description: "新地区（缺省 = 该列不动）" },
            city: { type: "string", description: "新城市（缺省 = 该列不动）" },
            address: { type: "string", description: "新地址（缺省 = 该列不动）" },
            latitude: { type: "number", description: "新纬度（缺省 = 该列不动）" },
            longitude: { type: "number", description: "新经度（缺省 = 该列不动）" },
            notes: { type: "string", description: "新备注（缺省 = 该列不动）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_customer",
        description: "删除 OPC 客户（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_customer",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "客户 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_invoice",
        description: "删除 OPC 发票（域包发票；经宿主 opc.store 写面，成功无载荷）",
        command: "worker://opc_delete_invoice",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发票 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      // ── #32 批 B：四条项目命令（详情读 1 + 写 3，均走 opc.store ⇒ 写命令由
      //    `withRequiredPermission` 按上面 `OPC_WRITE_COMMANDS` 自动挂 workspace-write）。
      //    详情出参 = 摘要列 + 描述 / 备注 / 里程碑明细（一个扁平对象，无内层壳键）。
      {
        name: "opc_get_project",
        description: "获取 OPC 项目详情（经宿主 opc.store 只读接缝；含描述 / 备注 / 里程碑明细）",
        command: "worker://opc_get_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_update_project",
        description: "更新 OPC 项目（经宿主 opc.store 写面；回更新后的详情视图）",
        command: "worker://opc_update_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
            title: { type: "string", description: "新标题（缺省 = 该列不动）" },
            description: { type: "string", description: "新描述（缺省 = 该列不动）" },
            budget: { type: "number", description: "新预算（缺省 = 该列不动）" },
            deadline: { type: "number", description: "新截止时间戳（epoch 秒，缺省 = 不动）" },
            notes: { type: "string", description: "新备注（缺省 = 该列不动）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_project",
        description: "删除 OPC 项目（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_project",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "项目 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_complete_milestone",
        description: "完成 OPC 项目的里程碑（经宿主 opc.store 写面；回完成后的详情视图）",
        command: "worker://opc_complete_milestone",
        inputSchema: {
          type: "object",
          properties: {
            projectId: { type: "string", description: "项目 id" },
            milestoneId: { type: "string", description: "里程碑 id（须在该项目明细里）" },
          },
          required: ["projectId", "milestoneId"],
        },
      },
      {
        name: "opc_create_project",
        description: "创建 OPC 项目（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_project",
        inputSchema: {
          type: "object",
          properties: {
            customerId: { type: "string", description: "关联客户 id（可选）" },
            title: { type: "string", description: "项目标题" },
            description: { type: "string", description: "项目描述（可选）" },
            budget: { type: "number", description: "预算（可选）" },
            currency: { type: "string", description: "货币代码（缺省 CNY）" },
            deadline: { type: "number", description: "截止时间戳（epoch 秒，可选）" },
            notes: { type: "string", description: "备注（可选）" },
          },
          required: ["title"],
        },
      },
      {
        name: "opc_add_milestone",
        description: "给 OPC 项目添加里程碑（经宿主 opc.store 写面；回项目视图，含里程碑计数）",
        command: "worker://opc_add_milestone",
        inputSchema: {
          type: "object",
          properties: {
            projectId: { type: "string", description: "项目 id" },
            title: { type: "string", description: "里程碑标题" },
            description: { type: "string", description: "描述（可选）" },
            dueAt: { type: "number", description: "到期时间戳（epoch 秒，可选）" },
          },
          required: ["projectId", "title"],
        },
      },
      {
        name: "opc_create_landing_page",
        description: "创建 OPC 落地页（经宿主 opc.store 写面；创建后为草稿态，回列裁剪视图）",
        command: "worker://opc_create_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "标题" },
            slug: { type: "string", description: "URL 标识" },
            description: { type: "string", description: "页面描述（可选）" },
            content: { type: "string", description: "正文（Markdown）" },
          },
          required: ["title", "slug", "content"],
        },
      },
      {
        name: "opc_publish_landing_page",
        description: "发布 OPC 落地页（经宿主 opc.store 写面；回发布后的列裁剪视图）",
        command: "worker://opc_publish_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "落地页 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_delete_landing_page",
        description: "删除 OPC 落地页（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_delete_landing_page",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "落地页 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_mark_contact_read",
        description: "把 OPC 联系提交标成已读（经宿主 opc.store 写面；成功无载荷）",
        command: "worker://opc_mark_contact_read",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "联系提交 id（接缝主键；空值由宿主 provider 拒收）" },
          },
          required: ["id"],
        },
      },
      {
        name: "opc_get_financial_report",
        description: "生成 OPC 财务报表并附投资建议（只读 op：按期间统计收入 / 利润 / 可投资金额）",
        command: "worker://opc_get_financial_report",
        inputSchema: {
          type: "object",
          properties: {
            period: { type: "string", description: "期间，如 '2026-07'（月度）或 '2026-Q2'（季度）" },
          },
          required: ["period"],
        },
      },
      {
        name: "opc_create_content_asset",
        description: "创建 OPC 内容资产（经宿主 opc.store 写面；创建后为草稿态，回列裁剪视图）",
        command: "worker://opc_create_content_asset",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "标题" },
            contentType: { type: "string", description: "内容类型（article / video / image）" },
            body: { type: "string", description: "正文（Markdown）" },
            tags: { type: "array", items: { type: "string" }, description: "标签（可选）" },
          },
          required: ["title", "contentType", "body"],
        },
      },
      {
        name: "opc_create_publish_schedule",
        description: "创建 OPC 发布计划（经宿主 opc.store 写面；回创建行的列裁剪视图，含 id）",
        command: "worker://opc_create_publish_schedule",
        inputSchema: {
          type: "object",
          properties: {
            contentRefType: { type: "string", description: "内容类型（blog_post / content_asset）" },
            contentRefId: { type: "string", description: "被发布内容的主键（不是本行 id）" },
            scheduledAt: { type: "number", description: "计划发布时间戳（epoch 秒）" },
          },
          required: ["contentRefType", "contentRefId", "scheduledAt"],
        },
      },
      // ── #36 批 1：8 条只读查询 + 1 条看板（全部走 opc.store 只读 op，无必填字段）──
      // 属性名一律 camelCase（与契约 `*ListQuery` 的 serde 键逐字一致）；不写 `required`：
      // 缺键即「不过滤 / 宿主取缺省」，不是错误。
      {
        name: "opc_list_customers",
        description: "列出 OPC 客户（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_customers",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤" },
            customerType: { type: "string", description: "客户类型过滤" },
            search: { type: "string", description: "名称 / 邮箱等关键词" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_projects",
        description: "列出 OPC 项目（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_projects",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤" },
            customerId: { type: "string", description: "所属客户 id" },
            search: { type: "string", description: "标题关键词" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_list_kpis",
        description: "列出 OPC KPI 记录（经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_kpis",
        inputSchema: {
          type: "object",
          properties: {
            period: { type: "string", description: "统计周期过滤（空 = 不过滤）" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        // 看板：宿主没有聚合 op，由插件进程组合三次列表查询现算六项读数。
        name: "opc_get_dashboard",
        description: "获取 OPC 仪表盘汇总（收入 / 待处理发票 / 活跃项目与客户计数）",
        command: "worker://opc_get_dashboard",
        inputSchema: { type: "object", properties: {} },
      },
      {
        // 看板总览：走宿主 `dashboard_summary` op（整表聚合，与上面「首页现算」不同口径）。
        name: "opc_get_dashboard_summary",
        description: "获取 OPC 看板总览（收入合计 / 发票 / 项目 / 客户计数 + KPI 与收入趋势明细）",
        command: "worker://opc_get_dashboard_summary",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 1：交付域五命令（两读三写）──
      // 与域包发票命令同名不同义：交付账本只列 `lead_id IS NOT NULL` 的来源行。
      // 属性名一律 camelCase（命令面）。两条读命令缺键即「不过滤」故不写 `required`；
      // 三条写命令把必填 id 列进 `required`，缺键在宿主侧就会被拒。
      {
        name: "opc_list_delivery_invoices",
        description: "列出交付发票（won 线索开票账本，可按状态过滤）",
        command: "worker://opc_list_delivery_invoices",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤（draft / sent / paid）" },
          },
        },
      },
      {
        name: "opc_create_invoice_from_lead",
        description: "把 won 线索开成交付发票（宿主幂等：已有发票直接返回）",
        command: "worker://opc_create_invoice_from_lead",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "需求线索 id（必须为 won 状态）" },
            input: {
              type: "object",
              description: "缺省字段从线索元数据自动填充（title / amount / currency / notes）",
            },
          },
          required: ["leadId"],
        },
      },
      {
        name: "opc_update_invoice_status",
        description: "推进交付发票状态机（draft → sent → paid，逆向报错）",
        command: "worker://opc_update_invoice_status",
        inputSchema: {
          type: "object",
          properties: {
            invoiceId: { type: "string", description: "交付发票 id" },
            status: { type: "string", description: "目标状态（draft / sent / paid）" },
          },
          required: ["invoiceId", "status"],
        },
      },
      {
        name: "opc_delete_delivery_invoice",
        description: "删除交付发票（作废用删除代替，不可逆）",
        command: "worker://opc_delete_delivery_invoice",
        inputSchema: {
          type: "object",
          properties: {
            invoiceId: { type: "string", description: "交付发票 id" },
          },
          required: ["invoiceId"],
        },
      },
      {
        name: "opc_get_delivery_summary",
        description: "交付汇总（won 数 / 发票数 / 回款按币种 / 转化率）",
        command: "worker://opc_get_delivery_summary",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 2：需求发现八命令（五写三读）──
      // 属性名 camelCase（命令面）。三条写命令把必填对象/字符串列进 `required`；
      // 两条读命令零入参，`list_leads` 三个过滤项均可缺省（缺键即不过滤）。
      {
        name: "opc_list_platforms",
        description: "列出需求平台配置（表空时自动填充内置默认平台）",
        command: "worker://opc_list_platforms",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_save_platform",
        description: "保存（新增或更新）需求平台配置",
        command: "worker://opc_save_platform",
        inputSchema: {
          type: "object",
          properties: {
            input: { type: "object", description: "平台配置（字段同 DemandPlatform）" },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_delete_platform",
        description: "删除需求平台配置",
        command: "worker://opc_delete_platform",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "平台标识" } },
          required: ["id"],
        },
      },
      {
        name: "opc_list_leads",
        description: "列出需求线索（按商业价值分降序，可按状态过滤）",
        command: "worker://opc_list_leads",
        inputSchema: {
          type: "object",
          properties: {
            limit: { type: "integer", description: "返回条数上限（缺省 100，宿主钳到 500）" },
            minScore: { type: "number", description: "最低商业价值分（缺省不过滤）" },
            status: { type: "string", description: "生命周期状态过滤（new / evaluated / ...）" },
          },
        },
      },
      {
        name: "opc_get_scan_policy",
        description: "读取需求扫描策略（缺键时返回默认策略）",
        command: "worker://opc_get_scan_policy",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_save_scan_policy",
        description: "保存需求扫描策略（宿主侧做范围钳制）",
        command: "worker://opc_save_scan_policy",
        inputSchema: {
          type: "object",
          properties: {
            policy: { type: "object", description: "扫描策略（字段同 ScanPolicy，可缺省字段）" },
          },
          required: ["policy"],
        },
      },
      {
        name: "opc_discover_and_evaluate_leads",
        description: "扫描并评估需求线索（并发扫描 → 评估 → 去重入库）",
        command: "worker://opc_discover_and_evaluate_leads",
        inputSchema: {
          type: "object",
          properties: { query: { type: "string", description: "扫描检索词（不能为空）" } },
          required: ["query"],
        },
      },
      {
        name: "opc_create_lead",
        description: "手动补录一条需求线索（窗口内去重返回既有行）",
        command: "worker://opc_create_lead",
        inputSchema: {
          type: "object",
          properties: {
            input: { type: "object", description: "线索字段（title / description 必填）" },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_update_lead_status",
        description: "更新需求线索生命周期状态（new → evaluated → contacted → won/lost）",
        command: "worker://opc_update_lead_status",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "线索 ID" },
            status: { type: "string", description: "目标状态（迁移合法性由宿主判定）" },
          },
          required: ["leadId", "status"],
        },
      },
      {
        name: "opc_convert_lead_to_workflow",
        description: "把需求线索转化为实现工作流模板（已转化则报错）",
        command: "worker://opc_convert_lead_to_workflow",
        inputSchema: {
          type: "object",
          properties: { leadId: { type: "string", description: "线索 ID" } },
          required: ["leadId"],
        },
      },
      {
        name: "opc_run_lead_workflow",
        description: "启动线索实现工作流的执行（返回执行 ID）",
        command: "worker://opc_run_lead_workflow",
        inputSchema: {
          type: "object",
          properties: { leadId: { type: "string", description: "线索 ID" } },
          required: ["leadId"],
        },
      },
      // ── #264 批 4：领域工作流三只读命令（无前端调用方，保留智能体工具面）──
      // 属性名 camelCase（命令面）；两条可缺省，一条必填 workflowId。
      {
        name: "opc_list_domain_workflows",
        description: "列出领域工作流摘要（可按 domainId 或 tag 过滤）",
        command: "worker://opc_list_domain_workflows",
        inputSchema: {
          type: "object",
          properties: {
            domainId: { type: "string", description: "领域标识（如 academic / finance）" },
            tag: { type: "string", description: "标签过滤" },
          },
        },
      },
      {
        name: "opc_get_domain_workflow",
        description: "获取单个领域工作流的完整定义（缺行返回 null）",
        command: "worker://opc_get_domain_workflow",
        inputSchema: {
          type: "object",
          properties: { workflowId: { type: "string", description: "工作流模板 ID" } },
          required: ["workflowId"],
        },
      },
      {
        name: "opc_list_domains",
        description: "列出领域 ID 与中文名（下拉选择用）",
        command: "worker://opc_list_domains",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 5：交付面六命令（执行 / 列表 / 详情 / 更新 / 重试 / 取消）──
      // 属性名 camelCase（命令面）；deliverables / errors 是任意 JSON 值。
      {
        name: "opc_execute_demand_workflow",
        description: "为需求线索创建交付记录并后台触发工作流（返回交付记录）",
        command: "worker://opc_execute_demand_workflow",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "线索 ID" },
            workflowTemplateId: { type: "string", description: "工作流模板 ID（缺省走线索已转化模板或默认交付模板）" },
          },
          required: ["leadId"],
        },
      },
      {
        name: "opc_list_deliveries",
        description: "列出交付记录（可按 status / leadId 过滤，按创建时间降序）",
        command: "worker://opc_list_deliveries",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤（pending/running/completed/failed/cancelled）" },
            leadId: { type: "string", description: "线索 ID 过滤" },
          },
        },
      },
      {
        name: "opc_get_delivery",
        description: "获取单个交付详情（缺行报 NotFound）",
        command: "worker://opc_get_delivery",
        inputSchema: {
          type: "object",
          properties: { deliveryId: { type: "string", description: "交付记录 ID" } },
          required: ["deliveryId"],
        },
      },
      {
        name: "opc_update_delivery",
        description: "更新交付执行面字段（status 必写；其余可选缺席 = 不动）",
        command: "worker://opc_update_delivery",
        inputSchema: {
          type: "object",
          properties: {
            deliveryId: { type: "string", description: "交付记录 ID" },
            status: { type: "string", description: "目标状态（pending/running/completed/failed/cancelled）" },
            progress: { type: "number", description: "进度 0-1" },
            resultSummary: { type: "string", description: "结果摘要" },
            deliverables: { description: "交付物（任意 JSON 值）" },
            errors: { description: "错误列表（任意 JSON 值）" },
          },
          required: ["deliveryId", "status"],
        },
      },
      {
        name: "opc_retry_delivery",
        description: "重置交付为待执行（清进度 / 结果 / 错误）",
        command: "worker://opc_retry_delivery",
        inputSchema: {
          type: "object",
          properties: { deliveryId: { type: "string", description: "交付记录 ID" } },
          required: ["deliveryId"],
        },
      },
      {
        name: "opc_cancel_delivery",
        description: "取消交付（状态置 cancelled 并落完成时间）",
        command: "worker://opc_cancel_delivery",
        inputSchema: {
          type: "object",
          properties: { deliveryId: { type: "string", description: "交付记录 ID" } },
          required: ["deliveryId"],
        },
      },
      // ── #264 批 6：能力发现面五命令（scan / proactive / analyze 零入参；
      // list 的 status 可选；close 只收 gapId）──
      {
        name: "opc_scan_capabilities",
        description: "扫描系统能力清单（工具/技能/MCP/工作流/智能体，读 op）",
        command: "worker://opc_scan_capabilities",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_proactive_evaluate_leads",
        description: "按配置的领域关键词主动扫描、评估并入库需求线索（写 op）",
        command: "worker://opc_proactive_evaluate_leads",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_list_capability_gaps",
        description: "列出能力缺口记录（读 op；status 可选过滤）",
        command: "worker://opc_list_capability_gaps",
        inputSchema: {
          type: "object",
          properties: { status: { type: "string", description: "缺口状态（如 open/resolved）" } },
        },
      },
      {
        name: "opc_close_capability_gap",
        description: "关闭能力缺口（状态置 resolved 并落关闭时间，写 op）",
        command: "worker://opc_close_capability_gap",
        inputSchema: {
          type: "object",
          properties: { gapId: { type: "string", description: "能力缺口 ID" } },
          required: ["gapId"],
        },
      },
      {
        name: "opc_analyze_capability_gaps",
        description: "主动分析能力缺口：统计高频缺失关键词并自动创建 Top 5 缺口行（写 op）",
        command: "worker://opc_analyze_capability_gaps",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 7 刀 A：需求订阅面三命令 + 域包配置面八命令（宿主同名 11 条 Tauri 命令
      // 已删，智能体工具面经本清单续用 `opc_*` 原名，见 #273 裁定）。
      // ⚠ 两组「同名不同源」不许合并：`opc_list_industries` 读代码内域包注册表，
      //   `opc_list_runtime_industries` 扫运行时目录下的 yaml；描述已逐条写明数据源。
      // 入参键 = 命令面 camelCase（`capabilityPackId` / `actionKey` / `input` / `id`），
      // 插件内部再映射成接缝 snake_case（`domain_pack_id` / `action_key`），两侧不互换。
      {
        name: "opc_list_subscriptions",
        description: "列出全部需求订阅词（读 op，整表按创建时间升序，含启停与最近扫描命中数）",
        command: "worker://opc_list_subscriptions",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_save_subscription",
        description: "新增或更新一条需求订阅（写 op；input.id 缺省 = 新增，关键词唯一冲突由宿主拒）",
        command: "worker://opc_save_subscription",
        inputSchema: {
          type: "object",
          properties: {
            input: {
              type: "object",
              description: "订阅字段；缺省字段按 dao 既有默认值/不动处理",
              properties: {
                id: { type: "string", description: "订阅 ID；留空表示新增" },
                keyword: { type: "string", description: "订阅关键词（全表唯一）" },
                enabled: { type: "boolean", description: "是否启用扫描" },
                intervalHours: { type: "number", description: "扫描间隔（小时，越界由宿主钳位）" },
                minScore: { type: "number", description: "推送门槛：商业价值分（越界由宿主钳位）" },
                platforms: {
                  type: "array",
                  items: { type: "string" },
                  description: "限定平台 ID；空数组 = 跟随全局启用的平台",
                },
              },
            },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_delete_subscription",
        description: "删除一条需求订阅（写 op；空 ID 与查无此行同回未找到）",
        command: "worker://opc_delete_subscription",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "订阅 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_list_industries",
        description: "列出代码内注册表登记的内建域包摘要（读 op，含操作数与工作流数；不扫磁盘）",
        command: "worker://opc_list_industries",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_get_capability_pack_config",
        description: "取一个内建域包的完整配置（读 op，含 prompt 全文；未注册域包回未找到）",
        command: "worker://opc_get_capability_pack_config",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID（注册表键）" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_action_config",
        description: "取内建域包单个操作的完整配置（读 op，含 prompt 与目标工具；不存在回未找到）",
        command: "worker://opc_get_action_config",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID（注册表键）" },
            actionKey: { type: "string", description: "操作 key" },
          },
          required: ["capabilityPackId", "actionKey"],
        },
      },
      {
        name: "opc_list_capability_pack_actions",
        description: "列出一个内建域包的全部操作摘要（读 op，裁掉 prompt / resultPrompt 两个长字段）",
        command: "worker://opc_list_capability_pack_actions",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID（注册表键）" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_list_capability_pack_workflows",
        description: "列出一个内建域包的全部工作流摘要（读 op，含 templateId，不含节点定义）",
        command: "worker://opc_list_capability_pack_workflows",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID（注册表键）" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_list_runtime_industries",
        description: "列出运行时域包目录下已存在的域包（读 op，扫磁盘 yaml；出参带 count）",
        command: "worker://opc_list_runtime_industries",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_get_kpi_definitions",
        description: "取一个域包的 KPI 定义清单（读 op；出参内层键为 snake_case，如 metric_type）",
        command: "worker://opc_get_kpi_definitions",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_has_capability_pack",
        description: "判断运行时域包是否存在（读 op；不存在回 exists=false，不当错误处理）",
        command: "worker://opc_has_capability_pack",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      // ── #264 批 7 刀 B1：域包全读七命令（宿主同名七条 Tauri 命令已删，智能体工具面
      // 经本清单续用 `opc_*` 原名，见 #273 裁定）。七条全 read-only，与 worker 权限表
      // 同批同源（写分支一条都不加）。
      {
        name: "opc_get_workflow_config",
        description: "取域包单个工作流配置（读 op；查无回未找到）",
        command: "worker://opc_get_workflow_config",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID（注册表键）" },
            workflowId: { type: "string", description: "工作流 ID" },
          },
          required: ["capabilityPackId", "workflowId"],
        },
      },
      {
        name: "opc_validate_entity",
        description: "验证域包实体（读 op —— 只校验不落库；出参含 valid 与 errors 数组）",
        command: "worker://opc_validate_entity",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            entityType: { type: "string", description: "实体类型" },
            entityData: { type: "object", description: "实体数据（键名与域包实体定义一致）" },
          },
          required: ["capabilityPackId", "entityType", "entityData"],
        },
      },
      {
        name: "opc_batch_validate_entities",
        description: "批量验证域包实体（读 op；元素内 entityType 为空的条目被宿主跳过，出参逐条带 valid/errors）",
        command: "worker://opc_batch_validate_entities",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            entities: {
              type: "array",
              items: { type: "object" },
              description: "实体列表；每元素含 entityType 与 data 两键",
            },
          },
          required: ["capabilityPackId", "entities"],
        },
      },
      {
        name: "opc_compute_kpis",
        description: "计算域包 KPI（读 op，走 DefaultDataService 读库；days 缺省按 30 处理）",
        command: "worker://opc_compute_kpis",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            days: { type: "number", description: "统计天数（可选整数，缺省 30）" },
          },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_capability_pack_data",
        description:
          "按域包 analysis.yaml 的 data_sources 路由取数据（读 op，db/cache/web/file vendor 降级链；query 缺省按空对象处理）",
        command: "worker://opc_get_capability_pack_data",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            sourceId: { type: "string", description: "数据源 ID（analysis.yaml data_sources[].id）" },
            dataDomain: { type: "string", description: "数据域" },
            query: { type: "object", description: "查询参数（可选对象）" },
          },
          required: ["capabilityPackId", "sourceId", "dataDomain"],
        },
      },
      {
        name: "opc_get_capability_pack_precheck",
        description: "域包数据源质量预检（读 op，quality_precheck 源清单探测）",
        command: "worker://opc_get_capability_pack_precheck",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_capability_pack_health",
        description: "域包数据源健康快照（读 op，vendor 降级可观测）",
        command: "worker://opc_get_capability_pack_health",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      // ── #264 批 7 刀 B2：域包单据面十二命令（六读六写；宿主同名十二条 Tauri 命令已删，
      // 智能体工具面经本清单续用 `opc_*` 原名）。六条写命令同批进 `OPC_WRITE_COMMANDS`
      // （= 宿主 `required_permission_for` 的写 op 名单，两处必须同批改）。
      // `input` 子对象的键是**域模型 DTO 的 snake_case**（与宿主 serde 反序列化一致），
      // 不转成 camelCase —— 与命令面平铺键（camelCase）分属两层。
      {
        name: "opc_get_invoice",
        description: "获取发票（读 op；查无回未找到）",
        command: "worker://opc_get_invoice",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "发票 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_update_invoice",
        description: "更新发票（写 op；input 内只改给定字段，缺省字段不动）",
        command: "worker://opc_update_invoice",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发票 ID" },
            input: {
              type: "object",
              description: "更新字段（snake_case，与域模型 DTO 一致；三个键都可选）",
              properties: {
                line_items: {
                  type: "array",
                  items: { type: "object" },
                  description: "明细行；元素含 description / quantity / unit_price / tax_rate / total",
                },
                notes: { type: "string", description: "备注" },
                due_at: {
                  type: "integer",
                  description: "到期时间戳（秒级 epoch）；传 null 表示清空",
                },
              },
            },
          },
          required: ["id", "input"],
        },
      },
      {
        name: "opc_get_customer",
        description: "获取客户（读 op；查无回未找到）",
        command: "worker://opc_get_customer",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "客户 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_find_customer_by_email",
        description: "按邮箱查找客户（读 op；查无回 null，不是错误）",
        command: "worker://opc_find_customer_by_email",
        inputSchema: {
          type: "object",
          properties: { email: { type: "string", description: "客户邮箱" } },
          required: ["email"],
        },
      },
      {
        name: "opc_record_kpi",
        description: "记录 KPI（写 op；归属域包由调用方从运行上下文提供，不自行编造）",
        command: "worker://opc_record_kpi",
        inputSchema: {
          type: "object",
          properties: {
            input: {
              type: "object",
              description: "KPI 记录字段（snake_case，与域模型 DTO 一致；五个键全必填）",
              properties: {
                domain_pack_id: { type: "string", description: "归属域包 ID（空串 / 空白由宿主拒）" },
                name: { type: "string", description: "指标名" },
                value: { type: "number", description: "指标值" },
                unit: { type: "string", description: "单位" },
                period: { type: "string", description: "统计周期（YYYY-MM）" },
              },
              required: ["domain_pack_id", "name", "value", "unit", "period"],
            },
          },
          required: ["input"],
        },
      },
      {
        name: "opc_list_revenue",
        description: "列出收入记录（读 op；category 可选过滤，limit 缺省由宿主取默认上限）",
        command: "worker://opc_list_revenue",
        inputSchema: {
          type: "object",
          properties: {
            category: { type: "string", description: "收入分类（可选）" },
            limit: { type: "number", description: "返回条数上限（可选整数）" },
          },
        },
      },
      {
        name: "opc_get_investment_advice",
        description: "获取投资建议（读 op；按周期现算，不落库）",
        command: "worker://opc_get_investment_advice",
        inputSchema: {
          type: "object",
          properties: { period: { type: "string", description: "统计周期" } },
          required: ["period"],
        },
      },
      {
        name: "opc_import_talent_library",
        description: "导入人才库（写 op；扫描目录填充人才模板，已存在的模板 id 跳过 —— 幂等）",
        command: "worker://opc_import_talent_library",
        inputSchema: {
          type: "object",
          properties: { path: { type: "string", description: "专家目录路径（不存在 / 不可读由宿主拒）" } },
          required: ["path"],
        },
      },
      {
        name: "opc_sync_fleet",
        description: "同步员工到舰队（写 op，零入参；幂等地把 OPC 员工注册 / 更新为舰队成员）",
        command: "worker://opc_sync_fleet",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_get_publish_schedule",
        description: "获取发布计划详情（读 op；查无回未找到）",
        command: "worker://opc_get_publish_schedule",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "发布计划 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_update_publish_schedule",
        description: "更新发布计划（写 op；input 内只改给定字段，缺省字段不动）",
        command: "worker://opc_update_publish_schedule",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "发布计划 ID" },
            input: {
              type: "object",
              description: "更新字段（snake_case，与域模型 DTO 一致；两个键都可选）",
              properties: {
                scheduled_at: {
                  type: "integer",
                  description: "计划发布时间戳（秒级 epoch）；传 null 表示清空",
                },
                status: { type: "string", description: "计划状态（合法值由宿主状态机判；传 null 表示清空）" },
              },
            },
          },
          required: ["id", "input"],
        },
      },
      {
        name: "opc_execute_dynamic_workflow",
        description: "动态工作流执行（写 op，兼容旧接口；转标准执行通道，三级模板查找在宿主）",
        command: "worker://opc_execute_dynamic_workflow",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            workflowId: { type: "string", description: "工作流 ID（可选；缺省按域包默认工作流）" },
            days: { type: "number", description: "统计天数（可选整数）" },
            userInput: { type: "object", description: "调用方输入（可选任意对象，原样透传）" },
          },
          required: ["capabilityPackId"],
        },
      },
      // ── #264 刀 C 批 2：域包分析执行（读命令；宿主同名 Tauri 命令已删，
      // 原命令名与智能体工具面逐字续用）。读命令 ⇒ 不进 `OPC_WRITE_COMMANDS`。
      {
        name: "opc_execute_analysis",
        description: "执行域包分析（读 op；风控阈值取域包 runtime.yaml 声明，未声明即不启用）",
        command: "worker://opc_execute_analysis",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            days: { type: "integer", description: "统计天数（可选；缺省 30 天）" },
          },
          required: ["capabilityPackId"],
        },
      },
      // ── #264 刀 C 批 3：学习面七命令（六写一读；宿主同名 Tauri 命令已删，原命令名与
      // 智能体工具面逐字续用）。六条写命令同批进 `OPC_WRITE_COMMANDS`
      // （= 宿主 `required_permission_for` 的写 op 名单，两处必须同批改）。
      {
        name: "opc_reflect_on_workflow",
        description: "触发工作流反思（写 op；反思模板由宿主从域包适配器取）",
        command: "worker://opc_reflect_on_workflow",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            workflowId: { type: "string", description: "工作流 ID" },
            workflowResult: { description: "工作流执行结果（任意 JSON）" },
          },
          required: ["capabilityPackId", "workflowId", "workflowResult"],
        },
      },
      {
        name: "opc_evolve_workflow",
        description: "触发工作流进化（写 op；进化约束由宿主从域包适配器取）",
        command: "worker://opc_evolve_workflow",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            workflowId: { type: "string", description: "工作流 ID" },
            reason: { type: "string", description: "进化原因" },
          },
          required: ["capabilityPackId", "workflowId", "reason"],
        },
      },
      {
        name: "opc_run_self_improvement",
        description: "执行自我改进（写 op）",
        command: "worker://opc_run_self_improvement",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            target: { type: "string", description: "改进目标" },
          },
          required: ["capabilityPackId", "target"],
        },
      },
      {
        name: "opc_trigger_capability_pack_learning",
        description: "触发域包学习闭环（写 op；反思 → 进化 → 自我改进 → RL 四段编排在宿主）",
        command: "worker://opc_trigger_capability_pack_learning",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            workflowId: { type: "string", description: "工作流 ID" },
            workflowResult: { description: "工作流执行结果（任意 JSON）" },
          },
          required: ["capabilityPackId", "workflowId", "workflowResult"],
        },
      },
      {
        name: "opc_get_rl_stats",
        description: "获取经验池统计信息（读 op；capabilityPackId 缺省取全局池）",
        command: "worker://opc_get_rl_stats",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID（可选；缺省取全局池）" },
          },
        },
      },
      {
        name: "opc_trigger_rl_optimization",
        description: "手动触发 RL 策略优化（写 op；RL 配置由宿主读取）",
        command: "worker://opc_trigger_rl_optimization",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_record_rl_experience",
        description: "手动记录一条工作流执行经验（写 op）",
        command: "worker://opc_record_rl_experience",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            workflowId: { type: "string", description: "工作流 ID" },
            qualityScore: { type: "number", description: "质量分" },
            workflowResult: { description: "工作流执行结果（任意 JSON）" },
          },
          required: ["capabilityPackId", "workflowId", "qualityScore", "workflowResult"],
        },
      },
      // ── #264 批 4a：工作项七命令（六写一读；宿主同名七条 Tauri 命令处于双声明期，待
      // 前端半迁完再删）。六条写命令
      // 同批进 `OPC_WRITE_COMMANDS`（= 宿主 `required_permission_for` 的写 op 名单）。
      {
        name: "opc_create_work_item",
        description: "创建工作项（写 op；id 由宿主生成 wi-<12 位 hex>）",
        command: "worker://opc_create_work_item",
        inputSchema: {
          type: "object",
          properties: {
            title: { type: "string", description: "工作项标题" },
            ownerRoleId: { type: "string", description: "负责角色 ID（可选）" },
          },
          required: ["title"],
        },
      },
      {
        name: "opc_kanban_board",
        description: "获取看板投影（读 op；按 phase 聚合成列）",
        command: "worker://opc_kanban_board",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_work_item_start",
        description: "认领工作项（写 op；依赖 doomed 时拒）",
        command: "worker://opc_work_item_start",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "工作项 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_work_item_review",
        description: "提交评审（写 op；先跑自改进评估，质量门达标才进 REVIEW）",
        command: "worker://opc_work_item_review",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "工作项 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_escalate_work_item",
        description: "升级阻塞工作项（写 op；置 BLOCKED 并记录原因）",
        command: "worker://opc_escalate_work_item",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "工作项 ID" },
            reason: { type: "string", description: "升级原因" },
          },
          required: ["id", "reason"],
        },
      },
      {
        name: "opc_work_item_unblock",
        description: "解除工作项阻塞（写 op）",
        command: "worker://opc_work_item_unblock",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "工作项 ID" } },
          required: ["id"],
        },
      },
      {
        name: "run_self_improving_opc_work_item",
        description: "自改进 OPC 工作项循环（写 op；规则化评估，毫秒级）",
        command: "worker://run_self_improving_opc_work_item",
        inputSchema: {
          type: "object",
          properties: {
            task: { type: "string", description: "工作项 ID / 任务标识" },
            maxRounds: { type: "integer", description: "最大轮次（缺省 3）" },
          },
          required: ["task"],
        },
      },
      // ── #264 批 4b：域包与内容资产五命令（三写两读；宿主同名五条 Tauri 命令处于
      // 双声明期，待前端半迁完再删）。三条写命令同批进 `OPC_WRITE_COMMANDS`
      // （= 宿主 `required_permission_for` 的写 op 名单，两处必须同批改）。
      {
        name: "opc_export_capability_pack",
        description: "导出域包为 .opcip 归档（写 op；返回落盘路径）",
        command: "worker://opc_export_capability_pack",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "域包 ID" },
            outDir: { type: "string", description: "导出目录" },
          },
          required: ["id", "outDir"],
        },
      },
      {
        name: "opc_import_capability_pack",
        description: "导入 .opcip 域包并注册（写 op；返回能力闭合校验结论）",
        command: "worker://opc_import_capability_pack",
        inputSchema: {
          type: "object",
          properties: {
            archivePath: { type: "string", description: "归档绝对路径" },
          },
          required: ["archivePath"],
        },
      },
      {
        name: "opc_set_capability_pack_enabled",
        description: "启用 / 停用域包（写 op）",
        command: "worker://opc_set_capability_pack_enabled",
        inputSchema: {
          type: "object",
          properties: {
            packId: { type: "string", description: "域包 ID" },
            enabled: { type: "boolean", description: "是否启用" },
          },
          required: ["packId", "enabled"],
        },
      },
      {
        name: "opc_get_content_asset",
        description: "取内容资产全文（读 op；含 body）",
        command: "worker://opc_get_content_asset",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "内容资产 ID" } },
          required: ["id"],
        },
      },
      {
        name: "opc_market_list",
        description: "列出市场域包（读 op；扫内置目录 + 已装状态）",
        command: "worker://opc_market_list",
        inputSchema: { type: "object", properties: {} },
      },
      // ── #264 批 7 刀 B3：域包自动规则 / 仪表盘四命令（三读一写；宿主同名四条 Tauri
      // 命令已删）。`opc_run_automation_rules` 是写命令，同批进 `OPC_WRITE_COMMANDS`
      // （= 宿主 `required_permission_for` 的写 op 名单，两处必须同批改）。
      {
        name: "opc_get_capability_pack_dashboard",
        description: "获取域包仪表盘（读 op；days 缺省由宿主按 30 天取）",
        command: "worker://opc_get_capability_pack_dashboard",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            days: { type: "integer", description: "统计天数（可选）" },
          },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_capability_pack_workflow_steps",
        description: "获取域包工作流步骤（读 op）",
        command: "worker://opc_get_capability_pack_workflow_steps",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_capability_pack_automation_rules",
        description: "获取域包自动化规则（读 op；只回启用的规则）",
        command: "worker://opc_get_capability_pack_automation_rules",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_run_automation_rules",
        description: "运行域包自动化规则（写 op；规则动作会改库）",
        command: "worker://opc_run_automation_rules",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            entityType: { type: "string", description: "实体类型（如 customer）" },
            entityId: { type: "string", description: "实体 ID" },
            status: { type: "string", description: "实体状态（可选）" },
            overdueDays: { type: "integer", description: "逾期天数（可选）" },
            createdDays: { type: "integer", description: "创建天数（可选）" },
            fields: { type: "object", description: "规则上下文字段（可选任意对象，原样透传）" },
          },
          required: ["capabilityPackId", "entityType", "entityId"],
        },
      },
      // ── #264 批 7 刀 B3（后三条）：需求订阅扫描 / 扫描任务装配 / 能力匹配。前两条是写
      // 命令（同批进 `OPC_WRITE_COMMANDS`），能力匹配是读命令。
      {
        name: "opc_run_subscription_scan",
        description: "批量扫描需求订阅（写 op；命中线索入库并推进订阅到期时间）",
        command: "worker://opc_run_subscription_scan",
        inputSchema: {
          type: "object",
          properties: {
            onlyDue: { type: "boolean", description: "只扫到期订阅（可选；缺省 true，false = 强制全量重扫）" },
          },
        },
      },
      {
        name: "opc_ensure_demand_scan_job",
        description: "幂等装配需求扫描定时任务（写 op；已有同任务类型则更新）",
        command: "worker://opc_ensure_demand_scan_job",
        inputSchema: {
          type: "object",
          properties: {
            cronExpression: { type: "string", description: "cron 表达式（可选；缺省按宿主默认）" },
            webhookUrl: { type: "string", description: "推送 webhook 地址（可选）" },
            filePath: { type: "string", description: "推送落盘路径（可选，支持 ~/ 前缀）" },
          },
        },
      },
      {
        name: "opc_match_lead_capabilities",
        description: "为需求线索匹配能力（读 op；检索失败降级为无命中）",
        command: "worker://opc_match_lead_capabilities",
        inputSchema: {
          type: "object",
          properties: {
            leadId: { type: "string", description: "线索 ID" },
            topK: { type: "integer", description: "返回候选数上限（可选；越界由宿主夹取）" },
          },
          required: ["leadId"],
        },
      },
      // 刀 B3 收尾：自动学习（写命令，同批进 `OPC_WRITE_COMMANDS`）。
      // `workflowResult` 是任意 JSON（迁移前宿主参数就是 `serde_json::Value`，含 null）。
      {
        name: "opc_auto_learn_workflow",
        description: "触发 OPC 域包工作流的自动学习管线（写 op；识别域包 → 质量分 → 反思/进化/RL）",
        command: "worker://opc_auto_learn_workflow",
        inputSchema: {
          type: "object",
          properties: {
            templateId: { type: "string", description: "工作流模板 ID" },
            workflowResult: { description: "工作流结果（任意 JSON，形状由宿主判）" },
          },
          required: ["templateId", "workflowResult"],
        },
      },
      // ── #264 刀 C 批 1：域包 manifest / prompt 装配 + 学习面五条命令（全读 ⇒ 五条都
      // **不进** `OPC_WRITE_COMMANDS`；宿主同名五条 Tauri 命令已删，出参信封形状由宿主 provider 钉）。
      {
        name: "opc_get_capability_pack",
        description: "获取域包 manifest 摘要（读 op；出参 { manifest }，version/enabled 为宿主常量）",
        command: "worker://opc_get_capability_pack",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_build_capability_pack_prompt",
        description: "构建域包上下文的对话 prompt（读 op；宿主装配 {{input}} 后回五键信封）",
        command: "worker://opc_build_capability_pack_prompt",
        inputSchema: {
          type: "object",
          properties: {
            capabilityPackId: { type: "string", description: "域包 ID" },
            actionKey: { type: "string", description: "操作 key" },
            userInput: { type: "string", description: "用户输入（可选；缺省或空白不替换模板）" },
          },
          required: ["capabilityPackId", "actionKey"],
        },
      },
      {
        name: "opc_get_learning_metrics",
        description: "查询域包学习指标（读 op；指标由宿主学习引擎现算，无跨调用缓存）",
        command: "worker://opc_get_learning_metrics",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_get_learning_config",
        description: "获取域包学习配置（读 op；缺 learning.yaml 由宿主退回默认配置，不报错）",
        command: "worker://opc_get_learning_config",
        inputSchema: {
          type: "object",
          properties: { capabilityPackId: { type: "string", description: "域包 ID" } },
          required: ["capabilityPackId"],
        },
      },
      {
        name: "opc_list_learning_configs",
        description: "获取全部域包学习配置（读 op，无入参；宿主动态扫描域包目录）",
        command: "worker://opc_list_learning_configs",
        inputSchema: {
          type: "object",
          properties: {},
        },
      },
    ]),
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}

/** accounting 域包独立清单：只声明三条迁出的发票命令。 */
function opcAccountingManifest({ version, binaryName, sha256 }) {
  return {
    name: "opc-accounting",
    version,
    description: "OPC accounting 域包：发票列表 / 创建 / 状态变更",
    permissions: ["subprocess_execution", "write"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    pages: [{
      id: "opc-accounting",
      title: "会计",
      entry: PAGE_ENTRY,
      loading: "bundle",
      navGroup: "packs",
      labelKey: "nav.financeAccounting",
      icon: "Calculator",
      navOrder: 1,
    }],
    domainPacks: [path.posix.join(DOMAIN_PACK_DIR, "accounting.json")],
    commands: [
      {
        name: "opc_list_invoices",
        description: "列出 OPC 发票（accounting 域包，经宿主 opc.store 只读接缝取数）",
        command: "worker://opc_list_invoices",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: {
            status: { type: "string", description: "状态过滤" },
            customerId: { type: "string", description: "客户 id" },
            dateFrom: { type: "number", description: "开票时间下界（epoch 秒）" },
            dateTo: { type: "number", description: "开票时间上界（epoch 秒）" },
            limit: { type: "number", description: "返回条数上限" },
          },
        },
      },
      {
        name: "opc_create_invoice",
        description: "创建 OPC 发票（accounting 域包，经宿主 opc.store 写面）",
        command: "worker://opc_create_invoice",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            customerId: { type: "string", description: "客户 id" },
            lineItems: {
              type: "array",
              description: "行项目；行金额由宿主现算",
              items: {
                type: "object",
                properties: {
                  description: { type: "string" },
                  quantity: { type: "number" },
                  unitPrice: { type: "number" },
                  taxRate: { type: "number" },
                },
              },
            },
            currency: { type: "string" },
            dueAt: { type: "number" },
            notes: { type: "string" },
          },
          required: ["customerId", "lineItems"],
        },
      },
      {
        name: "opc_transition_invoice",
        description: "变更 OPC 发票状态（accounting 域包，两态回执）",
        command: "worker://opc_transition_invoice",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string" },
            status: {
              type: "string",
              enum: ["draft", "sent", "paid", "overdue", "cancelled", "refunded"],
            },
          },
          required: ["id", "status"],
        },
      },
    ],
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}

/**
 * 通用域 worker 清单工厂（统一插件规范的唯一形态）。
 *
 * 只从插件名 / 标题 / 域包 id / 命令名列表派生清单，命令实现留在 Rust worker 内：
 * `PLUGIN_SPECS[name].commands` 是命令面的唯一登记处，契约 checker 会逐项对账。
 * 工具入参 schema 按命令后缀匹配四种通用形状（`web_search` / `file_read` /
 * `file_write` / `run_automation_rules`），不为每个域另写一份 —— 11 域共用同一份
 * 工具实现，schema 也必须同源。
 */
function opcDomainPageDecl(name, title) {
  const nav = OPC_DOMAIN_PAGE_NAV[name];
  const base = { id: name, title, entry: PAGE_ENTRY, loading: "bundle" };
  return nav ? { ...base, ...nav } : base;
}

function opcWorkerDomainManifest(name, title, pack) {
  return ({ version, binaryName, sha256 }) => {
    const commands = (PLUGIN_SPECS[name]?.commands ?? []).map((commandName) => {
      const description = `OPC ${title}域包命令：${commandName}`;
      if (commandName.endsWith("_web_search")) {
        return {
          name: commandName,
          description,
          command: `worker://${commandName}`,
          requiredPermission: "read-only",
          inputSchema: {
            type: "object",
            properties: { query: { type: "string", description: "搜索关键词" } },
            required: ["query"],
          },
        };
      }
      if (commandName.endsWith("_file_read")) {
        return {
          name: commandName,
          description,
          command: `worker://${commandName}`,
          requiredPermission: "read-only",
          inputSchema: {
            type: "object",
            properties: { path: { type: "string", description: "工作区相对路径" } },
            required: ["path"],
          },
        };
      }
      if (commandName.endsWith("_file_write")) {
        return {
          name: commandName,
          description,
          command: `worker://${commandName}`,
          requiredPermission: "workspace-write",
          inputSchema: {
            type: "object",
            properties: {
              path: { type: "string", description: "工作区相对路径" },
              content: { type: "string", description: "写入内容" },
            },
            required: ["path", "content"],
          },
        };
      }
      if (commandName.endsWith("_run_automation_rules")) {
        return {
          name: commandName,
          description,
          command: `worker://${commandName}`,
          requiredPermission: "workspace-write",
          inputSchema: {
            type: "object",
            properties: {
              // `capabilityPackId` 由 worker 按自身身份固定注入，不暴露给调用方越域。
              entityType: { type: "string", description: "实体类型" },
              entityId: { type: "string", description: "实体 id" },
              status: { type: "string", description: "实体状态过滤" },
              overdueDays: { type: "number", description: "逾期天数阈值" },
              createdDays: { type: "number", description: "创建天数阈值" },
              fields: { type: "object", description: "规则匹配字段" },
            },
            required: ["entityType", "entityId"],
          },
        };
      }
      // 其余工具命令（OpcSearchWiki / OpcCreateProject 等）的真实入参 schema 在宿主工具里，
      // 清单只声明「对象」这一形状；worker 不做字段级校验，避免复制宿主 schema 造成双权威。
      // 权限按命令语义保守取值：List/Search/Get/Validate 是读，其余按写处理（含 Bash / FileWrite）。
      // 实体批量校验走 opc.store 的读 op（只校验不落库），不能因命令名不含读取动词而误标写权限。
      const readTool = /_(?:list|search|get|read|find|query|batch_validate)_/.test(commandName);
      return {
        name: commandName,
        description,
        command: `worker://${commandName}`,
        requiredPermission: readTool ? "read-only" : "workspace-write",
        inputSchema: {
          type: "object",
          description: "参数由宿主工具校验；worker 只检查对象形状并逐字转发",
        },
      };
    });
    return {
      name,
      version,
      description: `OPC ${title}域包：域专属 worker（命令经宿主接缝复用工具实现）`,
      // 与既有域 worker 同档：`subprocess_execution` 是宿主启动命令 shell hook 的硬要求，
      // `write` 同时满足 opc.store 写 op 与 tool.set 写工具闸（FileWrite / Bash）。
      permissions: ["subprocess_execution", "write"],
      mcp_servers: [],
      skills: [],
      agents: [],
      defaultEnabled: true,
      worker: { program: path.posix.join("bin", binaryName) },
      pages: [opcDomainPageDecl(name, title)],
      domainPacks: [path.posix.join(DOMAIN_PACK_DIR, `${pack}.json`)],
      commands,
      integrity: { algorithm: "sha256", hash: sha256 },
    };
  };
}

/** 通用声明式域包清单工厂：只贡献页面 + logical manifest，不声明 worker / 命令。 */
function declarativeOpcManifest(name, title, pack) {
  return () => ({
    name,
    version: "1.0.0",
    description: `OPC ${title}域包：声明式插件（共享 Tool / 公共能力仍归公共底座）`,
    permissions: [],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    pages: [opcDomainPageDecl(name, title)],
    domainPacks: [path.posix.join(DOMAIN_PACK_DIR, `${pack}.json`)],
    commands: [],
  });
}

/** security 域包独立清单：声明式插件，无 worker / 无命令，三条能力仍由宿主 Tool 承接。 */
function opcSecurityManifest({ version }) {
  return {
    name: "opc-security",
    version: version ?? "1.0.0",
    description: "OPC security 域包：安全审计声明式插件（共享 Tool 仍归公共底座）",
    permissions: [],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    pages: [{
      id: "opc-security",
      title: "安全合规",
      entry: PAGE_ENTRY,
      loading: "bundle",
      navGroup: "packs",
      labelKey: "nav.devopsSecurity",
      icon: "ShieldCheck",
      navOrder: 7,
    }],
    domainPacks: [path.posix.join(DOMAIN_PACK_DIR, "security.json")],
    commands: [],
  };
}

/** sales_growth 域包独立清单：三条命令迁独立 worker。 */
function opcSalesGrowthManifest({ version, binaryName, sha256 }) {
  return {
    name: "opc-sales-growth",
    version,
    description: "OPC sales_growth 域包：客户创建 / 落地页列表 / 联系人列表",
    permissions: ["subprocess_execution", "write"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    pages: [{
      id: "opc-sales-growth",
      title: "销售增长",
      entry: PAGE_ENTRY,
      loading: "bundle",
      navGroup: "packs",
      labelKey: "nav.automationSales",
      icon: "TrendingUp",
      navOrder: 2,
    }],
    domainPacks: [path.posix.join(DOMAIN_PACK_DIR, "sales_growth.json")],
    commands: [
      {
        name: "opc_create_customer",
        description: "创建客户（sales_growth 域包，经宿主 opc.store 写面）",
        command: "worker://opc_create_customer",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            name: { type: "string" },
            email: { type: "string" },
            phone: { type: "string" },
            company: { type: "string" },
            customerType: { type: "string" },
            country: { type: "string" },
            region: { type: "string" },
            city: { type: "string" },
            address: { type: "string" },
            source: { type: "string" },
            notes: { type: "string" },
            latitude: { type: "number" },
            longitude: { type: "number" },
          },
          required: ["name", "email"],
        },
      },
      {
        name: "opc_list_landing_pages",
        description: "列出落地页（sales_growth 域包，只读）",
        command: "worker://opc_list_landing_pages",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
      {
        name: "opc_list_contacts",
        description: "列出联系人（sales_growth 域包，只读）",
        command: "worker://opc_list_contacts",
        requiredPermission: "read-only",
        inputSchema: { type: "object", properties: {} },
      },
    ],
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}

/** content_media 域包独立清单：内容资产 / 博客 / 发布计划命令迁独立 worker。 */
function opcContentMediaManifest({ version, binaryName, sha256 }) {
  const names = [
    "opc_create_content_asset",
    "opc_create_publish_schedule",
    "opc_list_blog_posts",
    "opc_create_blog_post",
    "opc_list_publish_schedules",
    "opc_list_content_assets",
    "opc_update_content_asset",
    "opc_delete_content_asset",
    "opc_cancel_publish_schedule",
    "opc_process_due_schedules",
  ];
  const writes = new Set([
    "opc_create_content_asset",
    "opc_create_publish_schedule",
    "opc_create_blog_post",
    "opc_update_content_asset",
    "opc_delete_content_asset",
    "opc_cancel_publish_schedule",
    "opc_process_due_schedules",
  ]);
  return {
    name: "opc-content-media",
    version,
    description: "OPC content_media 域包：博客 / 内容资产 / 发布计划",
    permissions: ["subprocess_execution", "write"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    pages: [{
      id: "opc-content-media",
      title: "内容媒体",
      entry: PAGE_ENTRY,
      loading: "bundle",
      navGroup: "packs",
      labelKey: "nav.contentMedia",
      icon: "Video",
      navOrder: 10,
    }],
    domainPacks: [path.posix.join(DOMAIN_PACK_DIR, "content_media.json")],
    commands: names.map((name) => ({
      name,
      description: `content_media 域包命令 ${name}`,
      command: `worker://${name}`,
      requiredPermission: writes.has(name) ? "workspace-write" : "read-only",
      inputSchema: { type: "object", properties: {} },
    })),
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}

/**
 * Wiki 业务功能包清单（P6.3 第一片）。
 *
 * 一条命令：`wiki_sync_note_to_knowledge_base`（写面：笔记 → 知识库文档）。
 * 首批**不含前端页面资产** ⇒ 不声明 `pages[]`（打包器会据此拒绝 web 资产）。
 *
 * ⚠ 包级 `permissions` 含 `execute`：删除类命令（`wiki_delete_note` /
 * `wiki_template_delete`）的宿主等价命令是 `#[agent_command(safety = Dangerous)]`
 * ⇒ 智能体工具面在派发层**无条件硬拒**（`command_decision`，含 `full_access` 会话）。
 * 清单必须把这两条对齐成 `danger-full-access`（投影同为 Dangerous），否则插件名
 * 成为「确认即可删」的旁路（判据见 PLAN §142.3 与 §328 两条 Dangerous 的对齐先例）。
 * `check_command_gate` 的 `DangerFullAccess` 分支**只认 `execute`**
 * （`subprocess_execution` 明确不算）⇒ 不声明它，这两条会在启动子进程前被拒。
 * 副作用仅限 gate 的档位判定：沙箱推导（`build_sandbox_from_permissions`）只看
 * `subprocess_execution` / `network_access`。
 */
function wikiManifest({ version, binaryName, sha256 }) {
  return {
    name: "wiki",
    version,
    description: "Wiki 业务功能包：笔记同步到知识库（经宿主 wiki.store 写接缝）",
    permissions: ["subprocess_execution", "write", "execute"],
    mcp_servers: [],
    skills: [],
    agents: [],
    defaultEnabled: true,
    worker: { program: path.posix.join("bin", binaryName) },
    commands: [
      {
        name: "wiki_list_notes",
        description: "列出一个 Wiki vault 下的笔记（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_list_notes",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { vaultId: { type: "string", description: "宿主侧 vault id" } },
          required: ["vaultId"],
        },
      },
      {
        name: "wiki_get_note_links",
        description: "读取一条 Wiki 笔记的出链（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_note_links",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { noteId: { type: "string", description: "宿主侧笔记 id" } },
          required: ["noteId"],
        },
      },
      {
        name: "wiki_get_note_backlinks",
        description: "读取指向一条 Wiki 笔记的反链（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_note_backlinks",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { noteId: { type: "string", description: "宿主侧笔记 id" } },
          required: ["noteId"],
        },
      },
      {
        name: "wiki_get_note",
        description: "按 id 读取一条 Wiki 笔记（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_note",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "宿主侧笔记 id" } },
          required: ["id"],
        },
      },
      {
        name: "wiki_get_note_by_path",
        description: "按 vault + 路径读取一条 Wiki 笔记（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_note_by_path",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: {
            vaultId: { type: "string", description: "宿主侧 vault id" },
            filePath: { type: "string", description: "vault 内相对路径" },
          },
          required: ["vaultId", "filePath"],
        },
      },
      {
        name: "wiki_notes_search",
        description: "搜索一个 Wiki vault 下的笔记（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_notes_search",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: {
            vaultId: { type: "string", description: "宿主侧 vault id" },
            query: { type: "string", description: "检索词" },
            topK: { type: "integer", description: "可选返回条数上限" },
          },
          required: ["vaultId", "query"],
        },
      },
      {
        name: "wiki_import_obsidian_vault",
        description: "导入 Obsidian Vault 到 Wiki（经宿主 wiki.store 写面）",
        command: "worker://wiki_import_obsidian_vault",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            wikiId: { type: "string", description: "宿主侧 wiki id" },
            vaultPath: { type: "string", description: "Obsidian vault 目录路径" },
          },
          required: ["wikiId", "vaultPath"],
        },
      },
      {
        name: "wiki_import_knowledge_md",
        description: "导入 KNOWLEDGE.md 到 Wiki（经宿主 wiki.store 写面）",
        command: "worker://wiki_import_knowledge_md",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            wikiId: { type: "string", description: "宿主侧 wiki id" },
            filePath: { type: "string", description: "可选 KNOWLEDGE.md 路径" },
          },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_export_markdown",
        description: "导出 Wiki 为 Markdown 目录（经宿主 wiki.store 写面）",
        command: "worker://wiki_export_markdown",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            wikiId: { type: "string", description: "宿主侧 wiki id" },
            outputPath: { type: "string", description: "输出目录路径" },
          },
          required: ["wikiId", "outputPath"],
        },
      },
      {
        name: "wiki_export_html",
        description: "导出 Wiki 为 HTML 目录（经宿主 wiki.store 写面）",
        command: "worker://wiki_export_html",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            wikiId: { type: "string", description: "宿主侧 wiki id" },
            outputPath: { type: "string", description: "输出目录路径" },
          },
          required: ["wikiId", "outputPath"],
        },
      },
      {
        name: "wiki_note_export_html",
        description: "导出一条 Wiki 笔记为 HTML（经宿主 wiki.store 写面）",
        command: "worker://wiki_note_export_html",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            noteId: { type: "string", description: "宿主侧笔记 id" },
            outputPath: { type: "string", description: "输出文件路径" },
          },
          required: ["noteId", "outputPath"],
        },
      },
      {
        name: "wiki_create_note",
        description: "创建一条 Wiki 笔记（经宿主 wiki.store 写面）",
        command: "worker://wiki_create_note",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            input: {
              type: "object",
              description: "宿主 CreateNoteInput（camelCase：vaultId/title/filePath/content/author）",
            },
          },
          required: ["input"],
        },
      },
      {
        name: "wiki_update_note",
        description: "更新一条 Wiki 笔记（经宿主 wiki.store 写面）",
        command: "worker://wiki_update_note",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "宿主侧笔记 id" },
            input: {
              type: "object",
              description: "宿主 UpdateNoteInput（camelCase：title/content/pageType/relatedPages）",
            },
          },
          required: ["id", "input"],
        },
      },
      {
        name: "wiki_delete_note",
        description: "删除一条 Wiki 笔记（经宿主 wiki.store 写面）",
        command: "worker://wiki_delete_note",
        // 宿主等价命令 `wiki_notes_delete` 是 `#[agent_command(safety = Dangerous)]`
        // 且**保留在宿主**（智能体面硬拒删笔记）⇒ 清单必须 danger-full-access，
        // 否则插件名成为「确认即可删」的旁路。包级 `execute` 的依据见上方 permissions 注释。
        requiredPermission: "danger-full-access",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "宿主侧笔记 id" } },
          required: ["id"],
        },
      },
      {
        name: "wiki_sync_note_to_knowledge_base",
        description: "把一条 Wiki 笔记同步进知识库（经宿主 wiki.store 写面）",
        command: "worker://wiki_sync_note_to_knowledge_base",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            noteId: { type: "string", description: "宿主侧笔记 id" },
            knowledgeBaseId: { type: "string", description: "宿主侧知识库 id" },
          },
          required: ["noteId", "knowledgeBaseId"],
        },
      },
      {
        name: "wiki_sync_note_links",
        description: "同步一条 Wiki 笔记的出链（经宿主 wiki.store 写面）",
        command: "worker://wiki_sync_note_links",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            vaultId: { type: "string", description: "宿主侧 vault id" },
            sourceNoteId: { type: "string", description: "宿主侧源笔记 id" },
            links: {
              type: "array",
              description: "三元组 [目标标题, 链接文本, 链接类型]",
              items: { type: "array", items: { type: "string" }, minItems: 3, maxItems: 3 },
            },
          },
          required: ["vaultId", "sourceNoteId", "links"],
        },
      },
      {
        name: "wiki_list_templates",
        description: "列出 Wiki 模板（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_list_templates",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_create_template",
        description: "创建 Wiki 模板（经宿主 wiki.store 写面）",
        command: "worker://wiki_create_template",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            input: {
              type: "object",
              description: "宿主 CreateWikiTemplateInput（camelCase）",
            },
          },
          required: ["input"],
        },
      },
      {
        name: "wiki_update_template",
        description: "更新 Wiki 模板（经宿主 wiki.store 写面）",
        command: "worker://wiki_update_template",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            input: {
              type: "object",
              description: "宿主 UpdateWikiTemplateInput（camelCase）",
            },
          },
          required: ["input"],
        },
      },
      {
        name: "wiki_template_delete",
        description: "删除 Wiki 模板（经宿主 wiki.store 写面）",
        command: "worker://wiki_template_delete",
        // 宿主等价命令 `wiki_template_delete` 原档是 `Dangerous`（P6.3 搜索/导入/导出/
        // 模板删除片退役）⇒ 清单必须 danger-full-access 保持等阶，否则智能体面把
        // 「禁止删模板」放松成「确认即可删」。包级 `execute` 的依据见上方 permissions 注释。
        requiredPermission: "danger-full-access",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string", description: "宿主侧模板 id" } },
          required: ["id"],
        },
      },
      {
        name: "wiki_get_graph_cached",
        description: "读取带缓存的 Wiki 图谱（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_graph_cached",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_get_graph_communities_cached",
        description: "读取带缓存的 Wiki 社区检测结果（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_graph_communities_cached",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_get_graph",
        description: "读取 Wiki 原图（笔记 + 链接，不含实体融合与缓存；经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_get_graph",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_detect_communities",
        description: "对 Wiki 笔记图实时跑一次 Louvain 社区检测（不写缓存；经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_detect_communities",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_invalidate_graph_cache",
        description: "失效 Wiki 图谱缓存（经宿主 wiki.store 写面）",
        command: "worker://wiki_invalidate_graph_cache",
        // 宿主等价命令 `invalidate_wiki_graph_cache` 是 `safety = Caution`
        // ⇒ 清单对齐 `workspace-write`（投影同为 Caution），档位不等阶才需升档。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_note_versions",
        description: "读取笔记版本历史（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_note_versions",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { noteId: { type: "string", description: "宿主侧笔记 id" } },
          required: ["noteId"],
        },
      },
      {
        name: "wiki_note_get_version",
        description: "读取指定笔记版本（经宿主 wiki.store 只读接缝）",
        command: "worker://wiki_note_get_version",
        requiredPermission: "read-only",
        inputSchema: {
          type: "object",
          properties: { versionId: { type: "integer", description: "宿主侧版本 id" } },
          required: ["versionId"],
        },
      },
      {
        name: "wiki_note_restore_version",
        description: "恢复笔记历史版本（经宿主 wiki.store 写面）",
        command: "worker://wiki_note_restore_version",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            noteId: { type: "string", description: "宿主侧笔记 id" },
            versionId: { type: "integer", description: "宿主侧版本 id" },
          },
          required: ["noteId", "versionId"],
        },
      },
      {
        name: "wiki_note_create_from_template",
        description: "从 Wiki 模板创建笔记（经宿主 wiki.store 写接缝）",
        command: "worker://wiki_note_create_from_template",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            vaultId: { type: "string", description: "宿主侧 vault id" },
            templateId: { type: "string", description: "宿主侧模板 id" },
            title: { type: "string", description: "可选笔记标题，缺省用模板名" },
          },
          required: ["vaultId", "templateId"],
        },
      },
      {
        name: "wiki_create_daily_note",
        description: "取得或创建当天 Wiki 每日笔记（经宿主 wiki.store 写接缝）",
        command: "worker://wiki_create_daily_note",
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: { vaultId: { type: "string", description: "宿主侧 vault id" } },
          required: ["vaultId"],
        },
      },
      {
        name: "wiki_update_wiki",
        description: "更新 Wiki 元数据（当前主要用途：改 embeddingProvider；经宿主 wiki.store 写面）",
        command: "worker://wiki_update_wiki",
        // 宿主等价命令 `update_wiki` 是 `safety = Caution` ⇒ 清单对齐 `workspace-write`
        // （投影同为 Caution），档位等阶，无需升档。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            id: { type: "string", description: "宿主侧 wiki id" },
            name: { type: "string", description: "可选新名称，缺省 / null 保持原值" },
            description: { type: "string", description: "可选新描述，缺省 / null 保持原值" },
            embeddingProvider: {
              type: "string",
              description: "可选新 embedding provider，缺省 / null 保持原值",
            },
            knowledgeBaseId: {
              type: "string",
              description: "可选关联知识库 id，缺省 / null 保持原值",
            },
          },
          required: ["id"],
        },
      },
      {
        name: "wiki_rebuild_index",
        description:
          "重建 Wiki 向量索引（清库后后台逐篇重索引，进度走 wiki-note-indexed 事件；经宿主 wiki.store 写面）",
        command: "worker://wiki_rebuild_index",
        // 宿主等价命令 `rebuild_wiki_index` 是 `safety = Caution` ⇒ 等阶 `workspace-write`。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_repair_graph",
        description: "修复 Wiki 图谱关联（自动关联 KB + 全量重建链接表 + 失效图谱缓存；经宿主 wiki.store 写面）",
        command: "worker://wiki_repair_graph",
        // 宿主等价命令 `repair_wiki_graph` 是 `safety = Caution` ⇒ 等阶 `workspace-write`。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: { wikiId: { type: "string", description: "宿主侧 wiki id" } },
          required: ["wikiId"],
        },
      },
      {
        name: "wiki_auto_connect_wiki",
        description: "按相似度自动连接 Wiki 笔记（可选 dryRun 预览；经宿主 wiki.store 写面，dryRun 时只算不写）",
        command: "worker://wiki_auto_connect_wiki",
        // dryRun 缺省为 false，会经 wiki.store 写数据库；宿主 worker 也将该 op 标为 Write。
        requiredPermission: "workspace-write",
        inputSchema: {
          type: "object",
          properties: {
            vaultId: { type: "string", description: "宿主侧 vault id" },
            threshold: { type: "number", description: "可选相似度阈值 0-1，缺省 0.7" },
            maxLinksPerNote: {
              type: "integer",
              description: "可选每笔记最大链接数 1-20，缺省 5",
            },
            dryRun: { type: "boolean", description: "可选预览模式，缺省 false（真实写入）" },
          },
          required: ["vaultId"],
        },
      },
    ],
    integrity: { algorithm: "sha256", hash: sha256 },
  };
}
