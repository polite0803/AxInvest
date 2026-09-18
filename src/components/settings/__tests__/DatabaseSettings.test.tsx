// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `Settings → 数据库` 卡片的三条回归防线（本轮修复）。
 * 第 3 条（修复明细块可达性 / F1）在文件末尾的 `describe` 里，含四条边界断言。
 *
 * ## 1. `fallback_to_sqlite` 不得被保存路径静默丢弃（P0 数据丢失）
 *
 * 后端 `DbConfig`（`dao::config`）有 **11** 个字段，而前端 `DbConfigForm` 曾经只有 **9**
 * 个（缺 `pg_password_enc` 与 `fallback_to_sqlite`）。`handleSave` 把
 * `validateFields()` 的结果**整体**当作 `DbConfig` 发给后端 ⇒ 缺席字段反序列化成
 * `None`。对 `fallback_to_sqlite` 而言 `None` 不是「没设置」而是**关掉 PG 降级**
 * （`init/database.rs:403` 的 `unwrap_or(true)` 只管住「读到 `None` 时按默认开」，
 * 而前端每次保存都写回 `null`... 反过来让 UI 与实际行为不一致）。
 *
 * 所以这里钉住两件事：
 * - 开关**无条件渲染** —— sqlite 档下也在（只是 `disabled`），不进条件分支。
 *   放进分支就重现了本 bug 的成因：字段是否出现在 `validateFields()` 里变成
 *   取决于 antd 卸载 / `preserve` 的细节。
 * - **保存载荷里带着它的当前值**（而不是 `undefined`）。
 *
 * ## 2. 连接测试失败：「本地化」不以「丢原因」为代价
 *
 * `translateBackendError` 命中错误码时**只返回译文**（`errorI18n.ts:146-163`，
 * `detail` 不进入结果）。故 `handleTest` 必须另行拼上
 * `parseBackendError(e).detail`（后端放在 detail 里的 sqlx 原文），
 * 否则用户看到「无法连接到数据库」却拿不到根因，无法自助排查。
 * 这里用 stub 把它钉成「两段都在最终文案里」，且**同一句话不重复出现两次**。
 */

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

// 从被测模块导入 DTO 类型，而不是在测试里再抄一份字段列表：抄来的副本会与真身各自腐烂，
// 而这里正是要拿它把 mock 载荷**钉在真实契约上**（mock 比真 payload 宽松 ⇒ 假绿）。
import { DatabaseSettings, type SchemaRepairReport } from "../DatabaseSettings";

const { invokeMock, logIpcErrorMock, translateMock, parseMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  logIpcErrorMock: vi.fn(() => () => {}),
  translateMock: vi.fn(),
  parseMock: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  logIpcError: logIpcErrorMock,
}));

vi.mock("@/lib/errorI18n", () => ({
  translateBackendError: translateMock,
  parseBackendError: parseMock,
}));

// `testFailed: "连接失败：{{error}}"` 的替身：把插值参数显式拼进来，便于断言两段原因都在。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts && "error" in opts ? `${key}|${String(opts.error)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

/** `DbConfig::default()` 的形态（`dao/src/config.rs:37-54`） */
function defaultDbConfig(): Record<string, unknown> {
  return {
    db_type: "sqlite",
    sqlite_path: null,
    pg_host: "localhost",
    pg_port: 5432,
    pg_database: "axagent",
    pg_user: "postgres",
    pg_password: null,
    pg_password_enc: null,
    pg_schema: null,
    use_ssl: false,
    fallback_to_sqlite: true,
  };
}

function mockSchemaStatus() {
  return {
    dialect: "",
    tables_expected: 0,
    tables_actual: 0,
    pending_apply: 0,
    pending_unsupported: 0,
    pending_manual: 0,
    advisories: 0,
    notes: [],
    applied_version: 0,
    latest_version: 0,
    probe_error: "test",
  };
}

function renderCard() {
  return render(
    <App>
      <DatabaseSettings />
    </App>,
  );
}

/**
 * 取 `fallback_to_sqlite` 那个开关。
 *
 * antd `Switch` 渲染成 `<button role="switch">`，`Form.Item` 的 `<label for>` 虽指向它，
 * 但测试环境的 accessible-name 计算拿不到该关联（实测 `findByRole("switch", {name})` 找不到），
 * 故按「标签所在 form-item 容器内的唯一 switch」定位。
 */
async function fallbackSwitch(): Promise<HTMLElement> {
  const label = await screen.findByText("settings.database.fallbackToSqlite");
  const item = label.closest(".ant-form-item");
  if (!item) { throw new Error("fallbackToSqlite 的 Form.Item 容器不存在"); }
  return within(item as HTMLElement).getByRole("switch");
}

describe("DatabaseSettings: fallback_to_sqlite 不被静默丢弃", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    logIpcErrorMock.mockImplementation(() => () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_db_config") { return defaultDbConfig(); }
      if (cmd === "get_schema_status") { return mockSchemaStatus(); }
      return undefined;
    });
  });

  it("sqlite 档下开关仍然渲染（无条件渲染），只是 disabled", async () => {
    renderCard();

    const sw = await fallbackSwitch();
    // sqlite 档下本项无意义 ⇒ disabled；但**必须在 DOM 里**，否则切到 postgres 前
    // 它的值会随卸载/挂载丢失。
    expect(sw).toBeDisabled();
  });

  it("保存载荷带着 fallback_to_sqlite（不是 undefined）", async () => {
    const user = userEvent.setup();
    renderCard();

    await fallbackSwitch();
    await user.click(screen.getByRole("button", { name: "settings.database.saveButton" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_db_config",
        expect.objectContaining({
          config: expect.objectContaining({ fallback_to_sqlite: true }),
        }),
      );
    });
  });

  it("后端给 null（= `None`，语义是「默认开启」）时开关显示为开，并被写回 true", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_db_config") {
        return { ...defaultDbConfig(), fallback_to_sqlite: null };
      }
      if (cmd === "get_schema_status") { return mockSchemaStatus(); }
      return undefined;
    });

    renderCard();

    const sw = await fallbackSwitch();
    // 直接 setFieldsValue(null) 会让 antd 把开关画成「关」，而后端读到 None 时
    // 实际是「开」（`unwrap_or(true)`）⇒ 界面说谎。组件必须归一到后端语义。
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "true"));

    await user.click(screen.getByRole("button", { name: "settings.database.saveButton" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_db_config",
        expect.objectContaining({
          config: expect.objectContaining({ fallback_to_sqlite: true }),
        }),
      );
    });
  });

  // ⚠ 下面两条为**独立验证者**补的用例（2026-09-17，verify-dbsettings）。
  // 目的不是复跑实现者的用例，而是去打探「原 bug 的成因是否真的被堵住」两个未被覆盖的角落：
  //   (a) `disabled` 的 Form.Item 值是否仍进 `validateFields()`（antd 对 disabled 无豁免，
  //       但这一点从未被本文件断言过，而它正是「界面显示 × 提交载荷」一致性的一环）；
  //   (b) 值能否活过 `dbType` 切换引起的重渲染 —— 即「放进条件分支里会丢值」这个成因，
  //       是否真的因为「无条件渲染」而消失。

  it("[补测] sqlite 档下 disabled 的开关值仍进保存载荷，且等于界面显示值（显式 false 活过 dbType 往返）", async () => {
    const user = userEvent.setup();
    renderCard();

    // 1) 切到 postgres：开关此时可交互（非 disabled）
    await user.click(await screen.findByText("settings.database.typePostgres"));
    const sw = await fallbackSwitch();
    await waitFor(() => expect(sw).not.toBeDisabled());

    // 2) 用户在 PG 档下**显式关闭**降级
    await user.click(sw);
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));

    // 3) 切回 sqlite：开关变 disabled，但必须在 DOM 内且**值保留**（不是回到 initialValues 的 true）
    await user.click(screen.getByText("settings.database.typeSqlite"));
    const sw2 = await fallbackSwitch();
    await waitFor(() => expect(sw2).toBeDisabled());
    expect(sw2).toHaveAttribute("aria-checked", "false");

    // 4) 保存：载荷必须仍带 `false`。
    //    「disabled 字段不进 values」或「切换后被 preserve 丢弃」两种实现都会在这里
    //    让它变成 `undefined` ⇒ 后端反序列化成 `None` ⇒ 语义反转成「允许降级」，
    //    也就是本 bug 的原始形态（静默清空 + 界面与后端语义相反）。
    await user.click(screen.getByRole("button", { name: "settings.database.saveButton" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_db_config",
        expect.objectContaining({
          config: expect.objectContaining({ fallback_to_sqlite: false }),
        }),
      );
    });
  });

  it("[补测] 后端显式给 false 时必须显示为关并原样写回 false（`?? true` 不得吃掉显式关闭）", async () => {
    // 归一化最典型的回归方向：`?? true` 被误写成 `|| true`（或对归一化结果再 `|| true`），
    // 显式 `false` 就会被吃掉、开关自己弹回「开」。先钉住 JS 语义本身，再钉住组件行为。
    //
    // ⚠ 必须用**类型拓宽过的变量**而不是字面量 `false`：字面量会被 TS 常量折叠
    // （`TS2869`：`??` 右操作数不可达）并被 oxlint 的 `no-constant-binary-expression`
    // 报出 —— 那是「断言能在运行时通过、但门禁过不去」的形态，门禁绿不了就失去意义。
    const explicitFalse: boolean | undefined = false;
    expect(explicitFalse ?? true).toBe(false);
    expect(explicitFalse || true).toBe(true);

    const user = userEvent.setup();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_db_config") {
        return { ...defaultDbConfig(), db_type: "postgres", fallback_to_sqlite: false };
      }
      if (cmd === "get_schema_status") { return mockSchemaStatus(); }
      return undefined;
    });

    renderCard();

    const sw = await fallbackSwitch();
    // 后端显式 false ⇒ 开关必须显示为「关」。若这里被归一成 true，说明显式关闭被吃掉。
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));

    await user.click(screen.getByRole("button", { name: "settings.database.saveButton" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "save_db_config",
        expect.objectContaining({
          config: expect.objectContaining({ fallback_to_sqlite: false }),
        }),
      );
    });
  });
});

describe("DatabaseSettings: 连接测试失败的展示", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    logIpcErrorMock.mockImplementation(() => () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_db_config") { return defaultDbConfig(); }
      if (cmd === "get_schema_status") { return mockSchemaStatus(); }
      if (cmd === "test_db_connection") {
        throw { code: "DB_CONNECT_FAILED", category: "retryable", detail: "sqlx: connection refused" };
      }
      return undefined;
    });
  });

  it("本地化译文与底层 sqlx 原文同时出现在提示里，且不套 testFailed 模板", async () => {
    const user = userEvent.setup();
    translateMock.mockReturnValue("TRANSLATED_REASON");
    parseMock.mockReturnValue({ code: "DB_CONNECT_FAILED", detail: "sqlx: connection refused" });

    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.testButton" }));

    const alert = await screen.findByText(/TRANSLATED_REASON/);
    expect(alert.textContent).toContain("sqlx: connection refused");
    // 有码 ⇒ 译文自足，不再套「连接失败：{{error}}」模板：该前缀对
    // `DB_QUERY_VERIFY_FAILED`（连接已建立、只是验证查询未通过）会自相矛盾。
    expect(alert.textContent).not.toContain("settings.database.testFailed");
    // 原 IPC 错误仍然进日志（诊断用），不能因为改走翻译层就丢掉
    expect(logIpcErrorMock).toHaveBeenCalledWith("test_db_connection");
  });

  it("无码（浏览器 mock 的纯文本）时用 testFailed 模板兜住，且句子不重复两次", async () => {
    const user = userEvent.setup();
    translateMock.mockReturnValue("PLAIN_REASON");
    parseMock.mockReturnValue({ detail: undefined });

    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.testButton" }));

    const alert = await screen.findByText(/PLAIN_REASON/);
    // 无码 ⇒ 回退既有模板
    expect(alert.textContent).toContain("settings.database.testFailed");
    // detail 缺失时不拼「— undefined」；译文只出现 1 次
    const occurrences = alert.textContent?.split("PLAIN_REASON").length ?? 0;
    expect(occurrences).toBe(2); // split 后恰好 2 段 = 出现 1 次
    expect(alert.textContent).not.toContain("undefined");
  });
});

/**
 * ## 3. 修复明细块必须可达（F1 回归，2026-09-17）
 *
 * 缺陷形状：`handleRepairSchema` 先 `setRepairIssues(report.errors)`，紧接着
 * `await refreshSchemaStatus()` —— 而后者当时无条件 `setRepairIssues([])`。
 * 后一次写入落在同一个同步续体里（两次 `setState` 之间**没有** `await`：
 * `refreshSchemaStatus()` 是被同步求值之后才 `await` 它返回的 promise），
 * **末次写入胜出** ⇒ 明细块的闸（`repairIssues.length > 0`）**恒为假**，
 * `SchemaRepairReport.errors` 从头到尾没有任何展示路径，而 toast 文案却写着
 * 「（明细见下方）」。
 *
 * ⚠ 归因只写到「末次写入胜出」，**不牵扯 React 批处理策略**：无论是否自动批处理，
 * 最终值都取后者；把成因挂到批处理上是错的。
 *
 * 四条断言覆盖修法的**两侧边界**：
 * 1. `errors` 非空 ⇒ 明细真的渲染出来（证明修复生效）；
 * 2. 用户主动点刷新 ⇒ 明细被清掉（证明不是「干脆别清了」：清空语义只属于刷新这个动作）；
 * 3. `errors` 为空 ⇒ 明细块**不渲染**（证明不是「无条件渲染」：过修同样是错的）；
 * 4. 超出 `REPAIR_ISSUES_SHOWN` ⇒ 只渲染前 N 条 + `… +M`（`slice` 阈值边界）。
 *
 * ⚠ 断言必须落在**条目文本**上。toast 与明细块复用同一个 i18n key
 * （`settings.database.schemaRepairPartial`），只断言 key 的话，明细块即使没渲染，
 * toast 也会让它命中 ⇒ 假绿。
 */
describe("DatabaseSettings: 修复明细块必须可达（F1 回归）", () => {
  const PARTIAL_ISSUE = "opc_capability: 未对照成功";
  /** 由各用例改写、`beforeEach` 复位，用来驱动 `repair_schema` 返回不同的 `errors`。 */
  let repairErrors: string[] = [];

  /** 载荷经 `SchemaRepairReport` 约束：少字段或写错类型都编不过，堵住「mock 比真 payload 宽松」。 */
  const repairReport = (errors: string[]): SchemaRepairReport => ({
    tables_scanned: 12,
    columns_added: [],
    types_healed: [],
    errors,
  });

  beforeEach(() => {
    repairErrors = [PARTIAL_ISSUE];
    vi.clearAllMocks();
    logIpcErrorMock.mockImplementation(() => () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_db_config") { return defaultDbConfig(); }
      if (cmd === "get_schema_status") { return mockSchemaStatus(); }
      if (cmd === "repair_schema") { return repairReport(repairErrors); }
      return undefined;
    });
  });

  it("[F1 回归] 修复返回非空 errors 时明细必须渲染出来（不得被随后的自动重查清掉）", async () => {
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.schemaRepairButton" }));

    expect(await screen.findByText(PARTIAL_ISSUE)).toBeInTheDocument();
  });

  it("[F1 回归] 用户主动点刷新后才清除明细（「清空」只属于刷新这个动作，不属于自动重查）", async () => {
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.schemaRepairButton" }));
    await screen.findByText(PARTIAL_ISSUE);

    await user.click(screen.getByRole("button", { name: "settings.database.schemaStatusRefresh" }));

    await waitFor(() => expect(screen.queryByText(PARTIAL_ISSUE)).toBeNull());
  });

  it("[F1 回归] errors 为空时明细块不得渲染（反向用例：堵「无条件渲染」这种过修）", async () => {
    repairErrors = [];
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.schemaRepairButton" }));

    // 先确认修复真的跑完了（`errors` 为空走 else 分支的 success toast）。少了这一步，
    // 下面那句在「repair 尚未返回」时同样成立 ⇒ 变成靠时机取胜的假绿。
    expect(await screen.findByText("settings.database.schemaRepairSuccess")).toBeInTheDocument();
    // 明细块标题与 toast 复用同一 key；`errors` 为空时它在整页应出现 0 次。
    // 若把闸写成无条件渲染，这里会命中 1 次 ⇒ 本条红。
    expect(screen.queryByText("settings.database.schemaRepairPartial")).toBeNull();
  });

  it("[F1 回归] 超出展示条数时只渲染前 N 条并折叠出「… +M」", async () => {
    // 7 条 > REPAIR_ISSUES_SHOWN(5)。这里刻意把阈值 5 写进断言：它是**行为契约**，
    // 阈值被改动时本条应当报红提醒复核，而不是静默跟随。
    repairErrors = Array.from({ length: 7 }, (_, i) => `tbl_${i + 1}: 未对照成功`);
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole("button", { name: "settings.database.schemaRepairButton" }));

    expect(await screen.findByText("tbl_1: 未对照成功")).toBeInTheDocument();
    expect(screen.getByText("tbl_5: 未对照成功")).toBeInTheDocument();
    // 第 6、7 条被 `slice(0, REPAIR_ISSUES_SHOWN)` 截掉，但条数要如实报出
    expect(screen.queryByText("tbl_6: 未对照成功")).toBeNull();
    expect(screen.queryByText("tbl_7: 未对照成功")).toBeNull();
    expect(screen.getByText("… +2")).toBeInTheDocument();
  });
});
