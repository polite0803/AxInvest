# PLAN-office-scene-domain-align — 办公室场景模板与 OPC 域包本体对齐

> 状态：active ｜ 日期：2026-09-25 ｜ 对应版本：v2.9.9


日期：2026-09-25　状态：✅ 三阶段全部完成，全量门禁绿，未提交

> 执行纪要：阶段 1+2 产出见上；阶段 2 实际补 **24** 个房间名（最初清单漏记 `finance_invest` 的 `analysis`，由阶段 3 门禁首跑抓出后补）+ `investment_office_desc`。阶段 3 落地 `scripts/check-office-scene-align.mjs`（三条硬拦 + d 报告档 + 豁免清单腐烂硬拦 + `--selftest` 六负控一正控），npm 脚本 `check:office-align[:selftest]`，成对接入 `ci-check.mjs:131-138` 与 `ci.yml`（逐字同源）。终验：typecheck / 全量 vitest 178 文件 2024 用例 / oxlint / dprint / key-parity / check_i18n --strict / EOL 全绿。待办：按计划 §5 分两个 commit（功能 / 门禁）。

## 1. 现状与证据

| 事实 | 证据 |
|---|---|
| 场景模板共 11 个：9 个行业版 slug 与域包 id **逐字一致**，另有 2 个通用版（`default_office` / `startup_loft`）+ 1 个扩展点注入版（`investment_office`，经 `registerSceneTemplate` 注册） | `src/components/office/phaser/sceneTemplates.ts:892`、`src/components/office/phaser/investSceneTemplates.ts:104`、`src/main.tsx:43` |
| 域包共 14 个：`config/opc/domain_packs/`（accounting / ai_research / consulting / content_media / design / ecommerce / education / finance_invest / game_dev / geospatial / project_management / sales_growth / security / software_dev） | 目录实测 |
| 行业显示名的**既有权威翻译**在 `opc.domains.*`，11 语言 × 14 行业全量齐备（含 `_desc`） | `src/i18n/locales/zh-CN.json:11421` 起 |
| 但 3 个消费点全部读 `office.scene.*` / `office.room.*` 平行命名空间：`office.scene` 只有 `default_office`、`startup_loft`、`investment_office` 3 个显示名，**9 个行业模板缺键** | `src/components/office/CreateFleetModal.tsx:127`、`src/components/office/OfficeTab.tsx:447,454` |
| i18next 未配 `parseMissingKeyHandler`（`src/i18n/index.ts`），缺键时 `t()` 返回键名本身 ⇒ `CreateFleetModal.tsx:127` 的 `\|\| tpl.slug` 兜底**永不生效**，下拉框显示 `office.scene.finance_invest` 原始串 | 实测键集对比 |
| 房间名同理：`office.room.*` 现有 12 键只覆盖 default/startup/invest 三套场景；9 个行业模板共 **23 个去重后的房间 id 无翻译**（dev, review, lab, finance, archive, audit, compute, paper, creation, shooting, editing, publishing, operations, showroom, service, warehouse, classroom, prepare, consulting, library, reception, sales, marketing） | `src/components/office/AddMemberModal.tsx:71`、`src/components/office/OfficeTab.tsx:91` |
| 无任何门禁检查「sceneTemplates ↔ domain_packs ↔ i18n」三者一致性；`scripts/check-domain-single-source.mjs` 管的是 `CapabilityDomain`（能力域），与 OPC 行业域是两个正交轴 | 脚本头注释实测 |

根因定性：同一概念（行业显示名）存在两套平行键体系（`office.scene.*` vs `opc.domains.*`），且后者已完备——这是 AGENTS.md 禁区 12「禁止重复定义」在本仓的新实例，与 `PLAN-domain-single-source.md`、`PLAN-industry-to-domain-migration.md` 处理的是同一类漂移，只是当时未覆盖 office 轴。

## 2. 目标与非目标

**目标**
1. 行业场景显示名收敛为**单一权威源**：域包 id（slug）+ `opc.domains.*` i18n；`office.scene.*` 只保留非域包的通用/注入场景。
2. 补齐 23 个房间名 × 11 语言（禁区 2 本来就要求，缺的是执行）。
3. 立门禁，把「域包 ↔ 场景 ↔ i18n」三向一致性从无人看守变成硬拦。

**非目标**
- 不为 5 个无场景行业（design / project_management / security / geospatial / game_dev）新建场景——是否补是产品决策，本计划只把它变成**可见豁免清单**。
- 不动后端、不动 `CapabilityDomain` 轴、不改 `investment_office`（它是上游扩展点注入的示范，刻意无域包）。

## 3. 分阶段方案

### 阶段 1 — 显示名单源化（前端，无新翻译）

- `sceneTemplates.ts` 新增导出 helper：
  ```ts
  /** 场景模板显示名：行业模板走权威源 opc.domains.<slug>；通用/注入场景走 office.scene.<displayNameKey> */
  export function isDomainScene(tpl) { return SCENE_DOMAIN_SLUGS.has(tpl.slug); }
  export function sceneTemplateLabelKey(tpl) { return isDomainScene(tpl) ? `opc.domains.${tpl.slug}` : `office.scene.${tpl.displayNameKey}`; }
  ```
  `SCENE_DOMAIN_SLUGS` 为 9 个行业 slug 的常量集合（与阶段 3 门禁的豁免白名单同源声明）。
- 改 3 个消费点用 helper：`CreateFleetModal.tsx:127`、`OfficeTab.tsx:447,454`；同时删掉失效的 `|| tpl.slug` 兜底（它是「假绿」，缺键症状从显示原始串变成显示键名，helper 收敛后不会再缺键）。
- `investSceneTemplates.ts` 的 `investment_office` 不在 slug 集合内 ⇒ 继续走 `office.scene.investment_office`，行为不变。

### 阶段 2 — 补 23 个房间名 × 11 语言

- 向 `src/i18n/locales/*.json`（11 个文件）的 `office.room` 段追加 23 键。zh-CN 用 sceneTemplates.ts 内注释里的房间名（如 trading=交易室已存在、dev=开发工位、review=代码审查区、lab=测试实验室…）；其余 10 语言按 en-US 语义翻译。
- 编辑 locale 时遵守禁区 14：LF 落盘 + 保序（`json.dumps(ensure_ascii=False, indent=2)` 重写）。
- 房间 id 与既有 `investment_office` 键（research/trading/risk/meeting）重叠 ⇒ 复用不新增，上表 23 键已去重。

### 阶段 3 — 门禁 `scripts/check-office-scene-align.mjs`

照 `check-domain-single-source.mjs` 的设计纪律（解析不出 ⇒ exit 3、扫 0 文件 ⇒ 非 0、带 `--selftest` 正负对照）：

| # | 判据 | 档 |
|---|---|---|
| a | 每个 `SCENE_TEMPLATES` 场景 slug ∈ `config/opc/domain_packs/` 目录 id ∪ 白名单 `{default_office, startup_loft, investment_office}` | **硬拦** |
| b | 每个行业场景 slug 的 `opc.domains.<slug>` 在 **11 个 locale** 均存在（含 `_desc`） | **硬拦** |
| c | 每个场景模板（含 invest 注入版）的每个 room id 的 `office.room.<id>` 在 11 个 locale 均存在 | **硬拦** |
| d | 每个 domain pack id 有同名场景模板，否则落豁免清单 `{design, project_management, security, geospatial, game_dev}` | **报告档**（补不补场景是产品裁决，硬拦会逼人删登记项——沿用 `check-ontology-consistency.mjs` 的「报告档不硬拦」纪律） |

- 解析方式：a/c 直接 regex 提取 `sceneTemplates.ts` + `investSceneTemplates.ts` 的 `slug:` 与 `rooms: [ { id: ... } ]`（避免 import TS）；b/d 读目录与 JSON。
- `package.json` 加 `"check:office-align": "node scripts/check-office-scene-align.mjs"`；在 `scripts/ci-check.mjs` 按既有成对形态登记（`ci-check.mjs:128-129` 先例）：`step("办公室场景↔域包对齐自检", … --selftest)` + `step("办公室场景↔域包/i18n 一致性", …)`，紧跟「能力域单一真相源」两行之后。

### 阶段 4 — Backlog（不实施）

5 个无场景行业补 Phaser 场景；届时从豁免清单移除、判据 d 可升级硬拦。

## 4. 验收（提交前全跑，缺一不报完成）

1. `npm run typecheck`
2. `npm run test:run`（office 相关：`src/stores/feature/__tests__/officeStore.conversation.test.ts` 及新增 helper 单测——阶段 1 给 `sceneTemplateLabelKey` 补一条 vitest：行业 slug 出 `opc.domains.*`、白名单出 `office.scene.*`）
3. `npm run check:i18n:key-parity` + `npm run check:i18n:untranslated:strict`
4. `node scripts/check-office-scene-align.mjs --selftest` 与不带参数跑绿
5. `npm run format`（dprint）+ `npm run lint:eslint`（oxlint，勿碰 legacy ESLint）
6. UI 实测：`npm run dev` 打开「创建办公室」下拉，确认 11 个模板全部显示中文名；切到某行业房间确认 Phaser 房间标签为中文。

## 5. Commit 切分

- Commit 1（功能）：阶段 1 + 阶段 2 + 单测；
- Commit 2（门禁）：阶段 3 脚本 + package.json/ci-check 接线。
既有 lint 顺手修复若出现，单独 commit（用户惯例：功能与 lint 修复分离）。

## 6. 风险

- `office.scene.<行业键>` 全仓 grep 仅 3 个消费点，无插件/后端引用该命名空间 ⇒ 删除路径安全；不改 `office.scene` 现有 3 键，无迁移。
- `check-i18n-untranslated.mjs` 有 baseline 机制，新增键若被判「zh 抄写嫌疑」需重跑 `check:i18n:untranslated:baseline`。
