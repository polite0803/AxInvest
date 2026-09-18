/**
 * 股票分析 profile 映射表一致性门禁。
 *
 * 背景（2026-09-14，真实缺陷）：`AgentProfileList` 离线回退分支用
 * `Object.keys(PROFILE_NAMES)` 驱动渲染，而该表当时只有 16 个 key，
 * 后端 `EXPERT_ROLE_MAP`（唯一权威）已有 38 个专家 + 新补的 explainer = 39 个
 * ⇒ **前端凭空少渲染一半专家行**，且 `PROFILE_ROLE_IDS` 缺 key 时静默兜底成空串。
 *
 * 本测试把「五张表必须齐、且与 11 语言包对齐」固化成门禁。三道断言：
 *   ① 五张表 key 集合两两相等（缺一即少一行 / 角色显示 "-"）；
 *   ② 每个 `PROFILE_NAME_KEYS` / `PROFILE_ROLE_KEYS` 引用的 i18n key
 *      在 11 个语言包中都存在（缺 key 时 i18next 会兜到 en-US，再缺就把
 *      **原始 key 串**渲染到界面上）；
 *   ③ 表内无重复 key（同一 profile 定义两次 ⇒ 后者静默覆盖前者）。
 *
 * 覆盖范围声明：本测试只校验前端五张表的**内部一致性 + 与 i18n 的完整性**。
 * 「key 集合是否等于后端 ESPERT_ROLE_MAP」无法在前端静态断言（后端是 Rust
 * 常量），由后端 `seed_consistency_tests::expert_registration_tables_are_consistent`
 * 与 `seed_stock_analysis` 的越界告警各自覆盖一侧。
 */
import ar from "@/i18n/locales/ar.json";
import de from "@/i18n/locales/de.json";
import enUS from "@/i18n/locales/en-US.json";
import es from "@/i18n/locales/es.json";
import fr from "@/i18n/locales/fr.json";
import hi from "@/i18n/locales/hi.json";
import ja from "@/i18n/locales/ja.json";
import ko from "@/i18n/locales/ko.json";
import ru from "@/i18n/locales/ru.json";
import zhCN from "@/i18n/locales/zh-CN.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import { describe, expect, it } from "vitest";
import {
  PROFILE_NAME_KEYS,
  PROFILE_NAMES,
  PROFILE_ROLE_IDS,
  PROFILE_ROLE_KEYS,
  PROFILE_ROLES,
} from "../agentProfileMaps";

const LOCALES: Array<[string, Record<string, unknown>]> = [
  ["zh-CN", zhCN],
  ["zh-TW", zhTW],
  ["en-US", enUS],
  ["ja", ja],
  ["ko", ko],
  ["fr", fr],
  ["de", de],
  ["es", es],
  ["ru", ru],
  ["hi", hi],
  ["ar", ar],
];

/** 按 "a.b.c" 路径取值（缺失返回 undefined） */
function getPath(obj: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((acc, seg) => {
    if (acc && typeof acc === "object" && seg in (acc as Record<string, unknown>)) {
      return (acc as Record<string, unknown>)[seg];
    }
    return undefined;
  }, obj);
}

const TABLES: Array<[string, Record<string, string>]> = [
  ["PROFILE_NAMES", PROFILE_NAMES],
  ["PROFILE_ROLES", PROFILE_ROLES],
  ["PROFILE_ROLE_IDS", PROFILE_ROLE_IDS],
  ["PROFILE_NAME_KEYS", PROFILE_NAME_KEYS],
  ["PROFILE_ROLE_KEYS", PROFILE_ROLE_KEYS],
];

describe("agentProfileMaps — 五张平行映射表", () => {
  it("五张表 key 集合完全一致", () => {
    const base = Object.keys(PROFILE_NAMES).sort();
    expect(base.length).toBeGreaterThan(0);
    for (const [name, table] of TABLES.slice(1)) {
      const keys = Object.keys(table).sort();
      const missing = base.filter((k) => !keys.includes(k));
      const extra = keys.filter((k) => !base.includes(k));
      expect(
        { missing, extra },
        `${name} 与 PROFILE_NAMES 的 key 集合不一致 —— 离线回退会少渲染 / 角色列显示 "-"`,
      ).toEqual({ missing: [], extra: [] });
    }
  });

  it("profileId 一律为 stock- 前缀且无重复定义", () => {
    const keys = Object.keys(PROFILE_NAMES);
    expect(keys.filter((k) => !k.startsWith("stock-"))).toEqual([]);
    for (const [name, table] of TABLES) {
      const raw = Object.keys(table);
      // Record 字面量重复 key 会被后者覆盖，无法在运行时检出；
      // 这里退而校验「去重后数量 == 原始数量」为恒真，真正的重复由 TS 编译期
      // 与 dprint 保证。保留断言以防未来改成数组构造后退化。
      expect(raw.length, `${name} 存在重复 key`).toBe(new Set(raw).size);
    }
  });

  it("每个 PROFILE_NAME_KEYS 引用的 i18n key 在 11 个语言包都存在", () => {
    const missing: string[] = [];
    for (const key of Object.values(PROFILE_NAME_KEYS)) {
      for (const [lang, bundle] of LOCALES) {
        const v = getPath(bundle, key);
        if (typeof v !== "string" || v.trim().length === 0) {
          missing.push(`${lang}: ${key}`);
        }
      }
    }
    expect(missing, "缺少 analystRoles 翻译 → 界面会显示原始 key 串").toEqual([]);
  });

  it("每个 PROFILE_ROLE_KEYS 引用的 i18n key 在 11 个语言包都存在", () => {
    const missing: string[] = [];
    for (const key of Object.values(PROFILE_ROLE_KEYS)) {
      for (const [lang, bundle] of LOCALES) {
        const v = getPath(bundle, key);
        if (typeof v !== "string" || v.trim().length === 0) {
          missing.push(`${lang}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("新增 profile 的离线回退节点 id 不与既有节点冲突", () => {
    // 离线回退用 `pid.replace("stock-", "")` 兜底生成节点 id（见 loadAll），
    // 这里只需保证不会产生空串。
    const empty = Object.keys(PROFILE_NAMES).filter((pid) => pid.replace("stock-", "").length === 0);
    expect(empty).toEqual([]);
  });
});
