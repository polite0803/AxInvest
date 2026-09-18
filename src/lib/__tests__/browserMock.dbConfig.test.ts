// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 浏览器模式（`npm run dev`，无 Tauri 后端）下 `Settings → 数据库` 卡片
 * **连接配置**侧的 mock 契约（结构状态侧见 `browserMock.schemaStatus.test.ts`）。
 *
 * ## 这三个命令修前分别错在哪
 *
 * 三者原本都没有桩，全部落到 `browserMock.ts` default 分支：
 *
 * * `get_db_config` —— 命中 `cmd.startsWith("get_")` ⇒ 返回 `{}`。组件
 *   `form.setFieldsValue({})` 是**静默**的（没有字段被写入、不抛错），于是表单
 *   停在 `initialValues` 的 3 个字段上，host / port / database / user / ssl
 *   全是 `undefined` —— 界面看起来正常，值却既不是配置值也不是后端默认值。
 * * `save_db_config` —— 返回 `undefined`，**什么都没存**；用户点「保存」看到
 *   「已保存」，刷新后改动全部消失。
 * * `test_db_connection` —— 返回 `undefined`（`endsWith("s")` / `startsWith("get_")`
 *   都不命中）⇒ **resolve**。组件成功路径**不消费返回值**：
 *   `DatabaseSettings.tsx:183-184` 是 `await invoke<void>("test_db_connection", …)`
 *   紧跟 `message.success(t("settings.database.testSuccess"))`，`result ||` 兜底已移除。
 *   所以 mock 只要 resolve，无论 resolve 出什么值，都会弹出一个**从未发生过的「连接成功」**。
 *   这条最危险：没有报错、没有日志，用户会以为自己填对了 PG 凭据。
 *
 * 所以本文件钉三件事：**字段形态完整**（snake_case、键集与 `DbConfig` 一致）、
 * **往返一致**（存什么就回什么）、**连接测试必须 reject**。
 *
 * ⚠ 断言在**修前必然失败**（`{}` 的键集不对、往返返回空对象、
 * `test_db_connection` 是 resolve 而非 reject），是有意为之的回归防线。
 */

import { beforeEach, describe, expect, it } from "vitest";

import { handleCommand } from "../browserMock";

/** 与 `dao::config::DbConfig` / `DatabaseSettings.tsx` 的 `DbConfigForm` 同集合 */
const DB_CONFIG_KEYS = [
  "db_type",
  "sqlite_path",
  "pg_host",
  "pg_port",
  "pg_database",
  "pg_user",
  "pg_password",
  "pg_password_enc",
  "pg_schema",
  "use_ssl",
  "fallback_to_sqlite",
].sort();

describe("browserMock: get_db_config", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("返回 11 个字段齐全的 DbConfig（与 DbConfig::default() 同集合）", async () => {
    const cfg = await handleCommand<Record<string, unknown>>("get_db_config");

    // 修前：default 分支的 `get_*` 给 `{}` ⇒ 本条当场失败。
    // 组件对缺失字段不抛错（`setFieldsValue` 静默），所以「键少了」这件事
    // **只能靠这个断言兜住** —— 没有第二道防线。
    expect(Object.keys(cfg).sort()).toEqual(DB_CONFIG_KEYS);
  });

  it("无保存记录时给 DbConfig::default() 的真值，不是空对象、也不是组件 initialValues", async () => {
    const cfg = await handleCommand<Record<string, unknown>>("get_db_config");

    // 与 `dao/src/config.rs:37-54` 的 Default 实现逐字对齐。
    // 修前：全部 `undefined` ⇒ 本条当场失败。
    expect(cfg.db_type).toBe("sqlite");
    expect(cfg.pg_host).toBe("localhost");
    expect(cfg.pg_port).toBe(5432);
    expect(cfg.pg_database).toBe("axagent");
    expect(cfg.pg_user).toBe("postgres");
    expect(cfg.use_ssl).toBe(false);
    expect(cfg.fallback_to_sqlite).toBe(true);
    // 组件 initialValues 只给了 `{db_type, pg_port, use_ssl}`；若 mock 返回空对象，
    // host/database/user 会变成 undefined（既非配置值也非后端默认值）——
    // 这正是「只在浏览器模式存在的偏差」，必须靠默认值消灭。
    expect(cfg.sqlite_path).toBeNull();
    expect(cfg.pg_schema).toBeNull();
    expect(cfg.pg_password).toBeNull();
    expect(cfg.pg_password_enc).toBeNull();
  });

  it("出口不做 camel 转换（否则表单每个字段都读成 undefined 且不报错）", async () => {
    const cfg = await handleCommand<Record<string, unknown>>("get_db_config");

    // 漏登记 `SNAKE_CASE_RESPONSE_COMMANDS` 时，出口会把 `db_type` 转成
    // `dbType`、`pg_host` 转成 `pgHost`：键都在、值全 undefined，
    // 而 `form.setFieldsValue` 不会因此报错 —— 静默错值。
    expect(cfg).not.toHaveProperty("dbType");
    expect(cfg).not.toHaveProperty("pgHost");
    expect(cfg).toHaveProperty("db_type");
    expect(cfg).toHaveProperty("pg_host");
  });
});

describe("browserMock: save_db_config", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("save → get 往返一致（用户点保存后刷新不该丢改动）", async () => {
    const saved = {
      db_type: "postgres",
      sqlite_path: null,
      pg_host: "db.internal",
      pg_port: 6543,
      pg_database: "axinvest",
      pg_user: "ax",
      pg_password: null,
      pg_schema: "public",
      use_ssl: true,
    };

    // 组件侧就是这样调的：`invoke("save_db_config", { config: values })`。
    await handleCommand("save_db_config", { config: saved });
    const back = await handleCommand<Record<string, unknown>>("get_db_config");

    // 修前：save 是 no-op、get 返回 `{}` ⇒ 本条当场失败。
    expect(back.pg_host).toBe("db.internal");
    expect(back.pg_port).toBe(6543);
    expect(back.pg_database).toBe("axinvest");
    expect(back.db_type).toBe("postgres");
    expect(back.use_ssl).toBe(true);
    expect(back.pg_schema).toBe("public");
  });

  it("是整体覆盖而不是字段合并（与真实后端一致）", async () => {
    await handleCommand("save_db_config", { config: { db_type: "postgres", pg_host: "h1" } });
    await handleCommand("save_db_config", { config: { db_type: "sqlite" } });
    const back = await handleCommand<Record<string, unknown>>("get_db_config");

    // 真实后端 `db_config.rs:129-168` 把整个 config 序列化后覆写文件，不做字段级合并 ——
    // **唯一例外是 `pg_password_enc`**：`pg_password` 缺席（`None`）时后端保留磁盘上原有密文
    // （`db_config.rs:139-141` 的 `PasswordAction::Keep`）。
    // mock 不实现该例外：浏览器模式没有 master.key ⇒ 这个字段恒为 `null`
    // ⇒ 两者在该字段上的差异**不可观测**。
    // 若 mock 改成通用字段合并，浏览器模式会掩盖「前端没传的字段被清空」这类真问题。
    expect(back).toEqual({ db_type: "sqlite" });
    expect(back).not.toHaveProperty("pg_host");
  });
});

describe("browserMock: test_db_connection", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("reject —— 「没连过」只能用 Err 表达，不能返回任何成功值", async () => {
    // 修前：default 分支返回 `undefined`（resolve）⇒ 本条当场失败。
    // 组件成功路径不消费返回值（`DatabaseSettings.tsx:183-184` 的 `await invoke<void>`
    // + `message.success(t(...))`，`result ||` 兜底已移除）⇒ mock 一旦 resolve 就弹「连接成功」。
    await expect(
      handleCommand("test_db_connection", { config: { db_type: "postgres" } }),
    ).rejects.toThrow();
  });

  it("抛的是给人看的原因，不是 JSON 包错误码", async () => {
    // 浏览器模式没有真实数据库、也就没有真实的 sqlx 错误可归因，编一个错误码
    // 等于伪造证据 ⇒ mock 只抛**纯文本**。
    // `handleTest` 的 catch 走 `translateBackendError` / `parseBackendError`：
    // 无码可查时原样回退原文（`errorI18n.ts:146-163` 的兜底分支），
    // 正是应有的表现；若这里改成 `JSON.stringify({code,…})`，界面会显示一串 JSON。
    const err = await handleCommand("test_db_connection").then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(Error);
    const msg = (err as Error).message;
    expect(msg.length).toBeGreaterThan(0);
    expect(msg.startsWith("{")).toBe(false);
  });
});
