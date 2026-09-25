#!/usr/bin/env node
/**
 * check-office-scene-align.mjs — 「办公室场景 ↔ OPC 域包 ↔ i18n」一致性门禁
 *
 * 存在理由（`PLAN-office-scene-domain-align.md`）
 * --------------------------------------------
 * 行业场景模板（`src/components/office/phaser/sceneTemplates.ts`）的 slug 与
 * OPC 域包（`config/opc/domain_packs/`）id 逐字一致，显示名权威源是 `opc.domains.*`，
 * 房间名走 `office.room.*`。这三份清单此前**一份都没被守**：
 * 实测 9 个行业场景的显示名在 `office.scene.*` 平行命名空间缺键 ×11 语言，
 * 下拉框直接显示 `office.scene.finance_invest` 原始串；23 个房间名同样全缺，
 * Phaser 画布房间标签显示裸 id——tsc / vitest / clippy 全绿，没有任何东西会红。
 *
 * 判据分档（沿用 `check-domain-single-source.mjs` / `check-ontology-consistency.mjs` 纪律）
 * --------------------------------------------------------------
 * | 档 | 对象 | 处置 |
 * |---|---|---|
 * | **硬拦** | a) 场景 slug ∈ 域包目录 ∪ 通用白名单，`SCENE_DOMAIN_SLUGS` 成员必须有同名场景；b) 行业场景的 `opc.domains.<slug>`(+`_desc`) 在 11 语言齐备；c) 每个场景（含 invest 注入版**与域包 office_scene.yaml 版**）的每个 room id/nameKey 的 `office.room.*` 在 11 语言齐备；e) YAML 场景 slug 必须 = 域包目录 id，manifest `office.seed_members[].room` 必须 ∈ 该域包场景房间（阶段 3，`PLAN-office-auto-provision.md`）；YAML 文件存在但解析不出 slug/id-nameKey 不配对 ⇒ 视同结构腐烂 | exit 1 / exit 3 |
 * | **报告** | d) 域包无同名场景 ⇒ 应落豁免清单；豁免清单里的域若已补场景（含 YAML 方式）⇒ 提示清理；YAML 与内置 TS 模板同名 ⇒ 提示阶段 4 迁移删 TS 版。补不补场景是产品裁决，硬拦会逼人删登记项——信号不是被解决，是被消灭 | 只打印 |
 *
 * 设计纪律
 * --------
 *  1. 任何一侧解析不出来 ⇒ **exit 3**，绝不静默按 0 条通过（否则门禁变永久绿灯）。
 *  2. locale 文件数 ≠ 11、场景数 = 0、域包目录为空 ⇒ exit 3。
 *  3. `--selftest` 用纯函数 `evaluate()` 做正负对照：坏样本必须被抓到、
 *     好样本必须零问题（比较器不能恒真也不能恒假）。
 *
 * 用法
 * ----
 *   node scripts/check-office-scene-align.mjs            # 门禁
 *   node scripts/check-office-scene-align.mjs --selftest # 正负对照
 *   node scripts/check-office-scene-align.mjs --json     # 机器可读输出
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

const argv = process.argv.slice(2);
const SELFTEST = argv.includes("--selftest");
const JSON_OUT = argv.includes("--json");

const LANGS = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];

/** 非域包的通用/注入场景（刻意无域包，见计划 §2 非目标） */
const GENERIC_WHITELIST = new Set(["default_office", "startup_loft", "investment_office"]);

/** 暂无场景的域包（阶段 4 backlog，见 `PLAN-office-scene-domain-align.md`） */
const EXEMPT_NO_SCENE = new Set(["design", "project_management", "security", "geospatial", "game_dev"]);

// ── 现场采集（真实数据）────────────────────────────────────────────

/** 从 TS 源文本抽取 `slug → { id 列表, nameKey 列表 }`；解析不出任何模板 ⇒ null */
function parseSceneTemplates(...sources) {
  const scenes = [];
  const marker = ": OfficeSceneTemplate = {";
  for (const src of sources) {
    const blocks = src.split(marker).slice(1);
    for (const block of blocks) {
      const slug = /slug:\s*"([^"]+)"/.exec(block)?.[1];
      const roomsBody = /rooms:\s*\[([\s\S]*?)\],/.exec(block)?.[1];
      if (!slug || roomsBody === undefined) { return null; }
      const ids = [...roomsBody.matchAll(/id:\s*"([^"]+)"/g)].map((m) => m[1]);
      const nameKeys = [...roomsBody.matchAll(/nameKey:\s*"([^"]+)"/g)].map((m) => m[1]);
      if (ids.length === 0 || ids.length !== nameKeys.length) { return null; }
      scenes.push({ slug, ids, nameKeys });
    }
  }
  return scenes.length > 0 ? scenes : null;
}

/**
 * 域包 `office_scene.yaml` 的行式解析（阶段 3 数据驱动场景；刻意不引 YAML 依赖，
 * 只取门禁判据需要的 slug / rooms.id / rooms.nameKey——缩进式 `- id: x` / `- nameKey: y` 行）。
 * 读到半截的模板（有 slug 无 rooms 行 ⇒ YAML 结构腐烂）由调用方判 exit 3。
 */
function parseOfficeSceneYamlScenes(text, origin) {
  const scenes = [];
  let cur = null;
  const flush = () => {
    if (cur) {
      if (cur.ids.length === 0) { fail(`${origin}: slug "${cur.slug}" 有房间名但 rooms 缺 id（YAML 结构腐烂）`); }
      scenes.push(cur);
    }
    cur = null;
  };
  for (const line of text.split(/\r?\n/)) {
    const slug = /^slug:\s*(\S+)/.exec(line)?.[1];
    if (slug) { flush(); cur = { slug, ids: [], nameKeys: [] }; continue; }
    if (!cur) { continue; }
    const id = /^\s*-?\s*id:\s*(\S+)\s*$/.exec(line)?.[1];
    if (id) { cur.ids.push(id); continue; }
    const nameKey = /^\s*-?\s*nameKey:\s*(\S+)\s*$/.exec(line)?.[1];
    if (nameKey) { cur.nameKeys.push(nameKey); }
  }
  flush();
  return scenes;
}

function parseDomainSlugs(sceneTemplatesSrc) {
  const body = /SCENE_DOMAIN_SLUGS[^=]*=\s*new Set\(\[([\s\S]*?)\]\)/.exec(sceneTemplatesSrc)?.[1];
  if (body === undefined) { return null; }
  return new Set([...body.matchAll(/"([^"]+)"/g)].map((m) => m[1]));
}

function collectReal() {
  const sceneSrc = fs.readFileSync(path.join(ROOT, "src/components/office/phaser/sceneTemplates.ts"), "utf8");
  const investSrc = fs.readFileSync(path.join(ROOT, "src/components/office/phaser/investSceneTemplates.ts"), "utf8");
  const scenes = parseSceneTemplates(sceneSrc, investSrc);
  if (!scenes) { fail("sceneTemplates.ts / investSceneTemplates.ts 场景解析不出（正则腐烂？）"); }
  const domainSlugs = parseDomainSlugs(sceneSrc);
  if (!domainSlugs) { fail("SCENE_DOMAIN_SLUGS 解析不出"); }

  const packsDir = path.join(ROOT, "config/opc/domain_packs");
  if (!fs.existsSync(packsDir)) { fail(`域包目录不存在: ${packsDir}`); }
  const domainPackIds = new Set(
    fs.readdirSync(packsDir, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name),
  );
  if (domainPackIds.size === 0) { fail("域包目录扫描到 0 个条目"); }

  const locales = LANGS.map((lang) => {
    const file = path.join(ROOT, `src/i18n/locales/${lang}.json`);
    if (!fs.existsSync(file)) { fail(`locale 文件缺失: ${lang}.json`); }
    const j = JSON.parse(fs.readFileSync(file, "utf8"));
    return {
      lang,
      officeRoom: new Set(Object.keys(j.office?.room ?? {})),
      officeScene: new Set(Object.keys(j.office?.scene ?? {})),
      opcDomains: new Set(Object.keys(j.opc?.domains ?? {})),
    };
  });
  if (locales.length !== 11) { fail(`locale 数 ${locales.length} ≠ 11`); }
  // office.scene 必须至少能解析出已登记的通用场景键，防空对象假绿
  for (const l of locales) {
    if (!l.officeScene.has("default_office")) { fail(`${l.lang}: office.scene 解析不出 default_office`); }
  }

  // 阶段 3 数据驱动面：域包 office_scene.yaml 场景 + manifest office.seed_members 房间
  const yamlScenes = [];
  const seedRoomsByPack = new Map();
  for (const id of domainPackIds) {
    const sceneFile = path.join(packsDir, id, "office_scene.yaml");
    if (fs.existsSync(sceneFile)) {
      const origin = `config/opc/domain_packs/${id}/office_scene.yaml`;
      const parsed = parseOfficeSceneYamlScenes(fs.readFileSync(sceneFile, "utf8"), origin);
      if (parsed.length === 0) { fail(`${origin}: 解析不出 slug（YAML 结构腐烂？）`); }
      for (const s of parsed) {
        if (s.ids.length !== s.nameKeys.length) { fail(`${origin}: slug "${s.slug}" 的 rooms id/nameKey 数量不配对`); }
        yamlScenes.push({ ...s, originPack: id });
      }
    }
    const manifestFile = path.join(packsDir, id, "manifest.yaml");
    if (fs.existsSync(manifestFile)) {
      seedRoomsByPack.set(id, parseManifestSeedRooms(fs.readFileSync(manifestFile, "utf8")));
    }
  }

  return { scenes, yamlScenes, seedRoomsByPack, domainSlugs, domainPackIds, locales };
}

/** manifest `office.seed_members[].room` 行式抽取（缺 room 的成员不计） */
function parseManifestSeedRooms(text) {
  const rooms = [];
  let inOffice = false;
  for (const line of text.split(/\r?\n/)) {
    if (/^office:\s*$/.test(line)) { inOffice = true; continue; }
    if (inOffice && /^\S/.test(line)) { inOffice = false; } // 新顶层键结束 office 段
    if (!inOffice) { continue; }
    const room = /^\s+room:\s*(\S+)\s*$/.exec(line)?.[1];
    if (room) { rooms.push(room); }
  }
  return rooms;
}

// ── 判据（纯函数，selftest 直接喂合成数据）──────────────────────────

function evaluate({ scenes, yamlScenes = [], seedRoomsByPack = new Map(), domainSlugs, domainPackIds, locales, exemptNoScene = EXEMPT_NO_SCENE }) {
  const hard = [];
  const reports = [];
  const sceneBySlug = new Map(scenes.map((s) => [s.slug, s]));

  // 阶段 3 数据驱动场景：合并进判据（内置同 slug 优先 ⇒ YAML 版仅报告重复声明）
  const allScenes = [...scenes];
  for (const ys of yamlScenes) {
    if (ys.slug !== ys.originPack) {
      hard.push(`office_scene.yaml（域包 ${ys.originPack}）的 slug "${ys.slug}" ≠ 域包 id（场景必须与域包同名对齐）`);
    }
    if (sceneBySlug.has(ys.slug)) {
      reports.push(`域包 "${ys.slug}" 的 office_scene.yaml 与内置 TS 模板同名 ⇒ 运行时内置优先，阶段 4 迁移时删除 TS 版`);
      continue;
    }
    if (!domainPackIds.has(ys.originPack)) { hard.push(`office_scene.yaml 位于不存在的域包目录 ${ys.originPack}`); }
    allScenes.push(ys);
    sceneBySlug.set(ys.slug, ys);
  }

  // a) slug 本体对齐
  for (const slug of domainSlugs) {
    if (!sceneBySlug.has(slug)) { hard.push(`SCENE_DOMAIN_SLUGS 含 "${slug}" 但无同名场景（悬空声明）`); }
  }
  for (const s of allScenes) {
    if (domainSlugs.has(s.slug)) {
      if (!domainPackIds.has(s.slug)) { hard.push(`场景 "${s.slug}" 声明为行业场景，但 config/opc/domain_packs/${s.slug} 不存在`); }
    } else if (!GENERIC_WHITELIST.has(s.slug) && !s.originPack) {
      hard.push(`场景 "${s.slug}" 既不在 SCENE_DOMAIN_SLUGS 也不在通用白名单`);
    }
  }

  // b) 行业场景显示名 = 权威源 opc.domains.<slug>（11 语言 × 含 _desc）
  for (const s of allScenes) {
    if (!domainSlugs.has(s.slug) && !s.originPack) { continue; }
    for (const l of locales) {
      for (const key of [s.slug, `${s.slug}_desc`]) {
        if (!l.opcDomains.has(key)) { hard.push(`opc.domains.${key} 在 ${l.lang} 缺失（场景 "${s.slug}"）`); }
      }
    }
  }

  // c) 房间名 office.room（11 语言；room.id 与 nameKey 都验；含 YAML 场景房间）
  for (const s of allScenes) {
    for (const key of new Set([...s.ids, ...s.nameKeys])) {
      for (const l of locales) {
        if (!l.officeRoom.has(key)) { hard.push(`office.room.${key} 在 ${l.lang} 缺失（场景 "${s.slug}" 房间）`); }
      }
    }
  }

  // e) manifest office.seed_members[].room 必须 ∈ 该域包场景房间（阶段 3，硬拦）
  for (const [packId, rooms] of seedRoomsByPack) {
    if (rooms.length === 0) { continue; }
    const scene = sceneBySlug.get(packId);
    if (!scene) {
      hard.push(`域包 "${packId}" 的 manifest 声明了 seed_members 房间，但无同名场景`);
      continue;
    }
    for (const room of rooms) {
      if (!scene.ids.includes(room)) { hard.push(`域包 "${packId}" seed_members 房间 "${room}" 不在场景房间 ${JSON.stringify(scene.ids)}`); }
    }
  }

  // d) 域包 → 场景覆盖（报告档）
  for (const id of domainPackIds) {
    if (sceneBySlug.has(id)) {
      if (exemptNoScene.has(id)) { reports.push(`豁免清单里的 "${id}" 已有场景 ⇒ 应从 EXEMPT_NO_SCENE 移除`); }
    } else if (exemptNoScene.has(id)) {
      reports.push(`域包 "${id}" 无场景（已登记豁免，阶段 4 backlog）`);
    } else {
      reports.push(`域包 "${id}" 无同名场景且未豁免 ⇒ 补场景或显式登记 EXEMPT_NO_SCENE`);
    }
  }
  for (const id of exemptNoScene) {
    if (!domainPackIds.has(id)) { hard.push(`EXEMPT_NO_SCENE 含 "${id}" 但域包目录不存在（豁免清单腐烂）`); }
  }

  return { hard, reports };
}

// ── selftest：正负对照 ─────────────────────────────────────────────

function selftest() {
  let bad = 0;
  const check = (name, cond) => {
    if (!cond) { bad += 1; console.log(`FAIL ${name}`); }
  };
  const mkLocale = (extra = {}) => ({
    lang: "t",
    officeRoom: new Set(["r1", ...extra.rooms ?? []]),
    officeScene: new Set(["default_office"]),
    opcDomains: new Set(["dom_a", "dom_a_desc", ...extra.opc ?? []]),
  });
  const base = {
    scenes: [
      { slug: "dom_a", ids: ["r1"], nameKeys: ["r1"] },
      { slug: "default_office", ids: ["r1"], nameKeys: ["r1"] },
    ],
    domainSlugs: new Set(["dom_a"]),
    domainPackIds: new Set(["dom_a", "dom_b"]),
    locales: [mkLocale()],
    // 合成豁免集——不引用真实 EXEMPT_NO_SCENE，否则「豁免清单腐烂」判据会把好样本打成硬拦
    exemptNoScene: new Set(["dom_b"]),
  };
  // 正控：好样本零硬拦
  const ok = evaluate(base);
  check("好样本必须零硬拦", ok.hard.length === 0);
  check("域包缺场景必须落报告档", ok.reports.some((r) => r.includes("dom_b")));
  // 负控 1：opc.domains 缺 _desc
  const m1 = evaluate({ ...base, locales: [{ ...base.locales[0], opcDomains: new Set(["dom_a"]) }] });
  check("缺 dom_a_desc 必须硬拦", m1.hard.some((p) => p.includes("dom_a_desc")));
  // 负控 2：office.room 缺房间键
  const m2 = evaluate({ ...base, scenes: [{ slug: "dom_a", ids: ["r1", "r2"], nameKeys: ["r1", "r2"] }, base.scenes[1]] });
  check("缺 office.room.r2 必须硬拦", m2.hard.some((p) => p.includes("office.room.r2")));
  // 负控 3：既非域包又非白名单的裸 slug
  const m3 = evaluate({ ...base, scenes: [...base.scenes, { slug: "rogue", ids: ["r1"], nameKeys: ["r1"] }] });
  check("rogue 场景必须硬拦", m3.hard.some((p) => p.includes("rogue")));
  // 负控 4：SCENE_DOMAIN_SLUGS 悬空声明
  const m4 = evaluate({ ...base, domainSlugs: new Set(["dom_a", "missing_scene"]) });
  check("悬空 slug 必须硬拦", m4.hard.some((p) => p.includes("missing_scene")));
  // 负控 5：场景声明为行业但域包目录没有
  const m5 = evaluate({ ...base, domainPackIds: new Set(["other"]) });
  check("无域包的行业场景必须硬拦", m5.hard.some((p) => p.includes("domain_packs/dom_a")));
  // 负控 6：seed_members 声明房间不在该域包场景房间（规则 e）
  const m6 = evaluate({ ...base, seedRoomsByPack: new Map([["dom_a", ["r1", "ghost"]]]) });
  check("seed 房间 ghost 必须硬拦", m6.hard.some((p) => p.includes("ghost")));
  // 负控 7：YAML 场景 slug ≠ 域包目录 id
  const m7 = evaluate({ ...base, yamlScenes: [{ slug: "wrong", originPack: "dom_a", ids: ["r1"], nameKeys: ["r1"] }] });
  check("YAML slug 错位必须硬拦", m7.hard.some((p) => p.includes("≠ 域包 id")));
  // 正控 2：为豁免域 dom_b 合法新增同名 YAML 场景 ⇒ 零硬拦（豁免过期只进报告档）
  const ok2 = evaluate({
    ...base,
    yamlScenes: [{ slug: "dom_b", originPack: "dom_b", ids: ["r1"], nameKeys: ["r1"] }],
    locales: [mkLocale({ opc: ["dom_b", "dom_b_desc"] })],
  });
  check("合法 YAML 场景必须零硬拦", ok2.hard.length === 0);
  check("YAML 补场景后豁免过期落报告档", ok2.reports.some((r) => r.includes("dom_b")));
  process.exit(bad === 0 ? 0 : 1);
}

// ── 主流程 ─────────────────────────────────────────────────────────

function fail(msg) {
  console.error(`EXIT3: ${msg}`);
  process.exit(3);
}

if (SELFTEST) { selftest(); console.log("selftest OK"); process.exit(0); }

const real = collectReal();
const { hard, reports } = evaluate(real);

if (JSON_OUT) {
  console.log(JSON.stringify({ hard, reports }, null, 2));
} else {
  for (const r of reports) { console.log(`ℹ️  ${r}`); }
  if (hard.length === 0) {
    console.log(`✅ 办公室场景↔域包↔i18n 对齐通过（${real.scenes.length} 场景 / ${real.domainPackIds.size} 域包 / 11 语言）`);
  } else {
    for (const p of hard) { console.error(`❌ ${p}`); }
    console.error(`\n共 ${hard.length} 条硬拦违规`);
  }
}
process.exit(hard.length === 0 ? 0 : 1);
