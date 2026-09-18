// SPDX-License-Identifier: AGPL-3.0-only

import { parseBackendError, translateBackendError } from "@/lib/errorI18n";
import { invoke, logIpcError } from "@/lib/invoke";
import { Alert, App, Button, Card, Form, Input, InputNumber, Radio, Space, Switch, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

// 本表单字段集 **逐字段核对** 过 `axagent_dao::config::DbConfig` 的全部 11 个字段
// （`src-tauri/crates/dao/src/config.rs:11-35`），差集**只有一个**：
//
//   `pg_password_enc`（后端内部的密文槽位，前端不认识、也不该认识）
//
// 它的「缺席」语义已在 `save_db_config` 里显式定义 —— **保留落盘密文**，
// 绝不能被读成「清空」（见 `src-tauri/src/commands/db_config.rs` 的
// `save_db_config` 文档注释）。
//
// ⚠ 这段注释是**收口断言**，不是背景介绍：本表单曾整体漏掉
// `fallback_to_sqlite`，而 `handleSave` 直接把 `validateFields()` 的结果当作整个
// `DbConfig` 发给后端 ⇒ 每次点「保存」都把它静默清成 `null`（后端据此关闭 PG 降级，
// PG 连不上时应用直接起不来）。**新增/重命名 DbConfig 字段时必须回到这里核对差集**，
// 否则同一个「前端字段集 ⊂ 后端 DTO ⇒ 缺席字段被静默清空」的缺陷会再次发生。
interface DbConfigForm {
  db_type: "sqlite" | "postgres";
  sqlite_path?: string;
  pg_host?: string;
  pg_port?: number;
  pg_database?: string;
  pg_user?: string;
  pg_password?: string;
  pg_schema?: string;
  use_ssl?: boolean;
  /** PG 不可达时是否降级到本地 SQLite；后端 `None` 的语义是「默认开启」（`init/database.rs:403`） */
  fallback_to_sqlite?: boolean;
}

// 数据库结构状态（与后端 axagent_dao 的 SchemaStatus 对齐）。
//
// ⚠ `applied_version` / `latest_version` 是「版本化迁移」时代的遗留记账
// （分别来自版本表 MAX(version) 与常量 CURRENT_VERSION）。建表机制已切换为声明式
// 引擎，二者不再代表任何真实状态，本 UI 刻意不渲染它们 —— 后端只用于识别 fork 库。
interface SchemaStatus {
  /** ⚠ 探测失败时后端给的是**空串**（不是 `null`、也不是「未知」）⇒ 渲染前必须判空 */
  dialect: string;
  tables_expected: number;
  tables_actual: number;
  pending_apply: number;
  pending_unsupported: number;
  pending_manual: number;
  advisories: number;
  notes: string[];
  applied_version: number;
  latest_version: number;
  /** 非 null 时结构面字段全为 0 且不可信，不能据此报「已收敛」 */
  probe_error: string | null;
}

// 修复结构的结果（与后端 SchemaRepairReport 对齐）
// 导出供测试 import：测试的 mock 载荷若靠手抄字段名，就会比真实 payload 宽松
// （少字段不报错）⇒ 组件一旦读那个字段就拿到 `undefined`，测试却仍绿。
export interface SchemaRepairReport {
  tables_scanned: number;
  columns_added: string[];
  types_healed: string[];
  /** 逐实体对照失败的原因列表（形如 `"conversations: Execution Error: …"`），正常为空 */
  errors: string[];
}

/** 修复明细最多展示的条数，超出部分只报数量（完整原因在日志里） */
const REPAIR_ISSUES_SHOWN = 5;

export function DatabaseSettings() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const [form] = Form.useForm<DbConfigForm>();
  const [loading, setLoading] = useState(false);
  const [initialLoading, setInitialLoading] = useState(true);
  const [testing, setTesting] = useState(false);
  const dbType = Form.useWatch("db_type", form);

  // 数据库结构状态
  const [schemaStatus, setSchemaStatus] = useState<SchemaStatus | null>(null);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [schemaError, setSchemaError] = useState<string | null>(null);
  const [repairError, setRepairError] = useState<string | null>(null);
  const [repairIssues, setRepairIssues] = useState<string[]>([]);
  const [repairing, setRepairing] = useState(false);

  // 职责收窄为「查询」：这里只取数，不碰 `repairIssues`。
  // 清空上一次修复结果是**用户点刷新**这个动作的语义。把它挂在查询函数上，则
  // `handleRepairSchema` 内部的自动重查也会去清 —— 它刚写进 `repairIssues` 的
  // `report.errors` 会被覆盖成 `[]` ⇒ 下方明细块恒不可达，而 toast 却写着「（明细见下方）」。
  // ⚠ 这里用函数名/字段名定位，**不写行号**：行号随每次编辑腐烂，写进注释就是埋了个必然失效的指针。
  const refreshSchemaStatus = useCallback(() => {
    setSchemaLoading(true);
    setSchemaError(null);
    invoke<SchemaStatus>("get_schema_status")
      .then((status) => {
        setSchemaStatus(status);
      })
      .catch((e) => {
        const msg = e instanceof Error ? e.message : String(e);
        setSchemaError(msg);
        logIpcError("get_schema_status")(e);
      })
      .finally(() => setSchemaLoading(false));
  }, []);

  // 用户主动重查：先消费掉上一次的修复结果（明细 + 错误），再查询。
  // 刻意与 `refreshSchemaStatus` 拆成两个函数，而不是给查询函数加 `clearRepair` 参数：
  // 那样两个调用点需要**相反**的取值（用户刷新要清、修复后自动重查不能清），
  // 而两者只能靠「默认值 + 记得传参」维持默契 —— 漏传一次，缺陷就静默复发。
  // 拆开之后，两处各自的意图在源码里是显式的，不依赖任何默认值。
  const handleRefreshStatus = useCallback(() => {
    setRepairError(null);
    setRepairIssues([]);
    refreshSchemaStatus();
  }, [refreshSchemaStatus]);

  const handleRepairSchema = useCallback(async () => {
    setRepairing(true);
    setRepairError(null);
    setRepairIssues([]);
    try {
      const report = await invoke<SchemaRepairReport>("repair_schema");
      setRepairIssues(report.errors);
      // 「有实体没对照上」不等于修复成功：命令层是 Err 才算整体失败，这里是部分未完成
      if (report.errors.length > 0) {
        message.warning(t("settings.database.schemaRepairPartial", { count: report.errors.length }));
      } else {
        message.success(t("settings.database.schemaRepairSuccess", {
          columns: report.columns_added.length,
          types: report.types_healed.length,
        }));
      }
      await refreshSchemaStatus();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // 与「状态查询失败」分开：那个槽位的文案模板是 schemaStatusError，语义不同
      setRepairError(msg);
      logIpcError("repair_schema")(e);
    } finally {
      setRepairing(false);
    }
  }, [refreshSchemaStatus, t, message]);

  useEffect(() => {
    invoke<DbConfigForm>("get_db_config")
      .then((cfg) => {
        // `fallback_to_sqlite` 是 `Option<bool>`：文件里没有该键（旧配置）时后端
        // 反序列化得到 `None`、序列化成 `null`，而 `null` 在 antd 里等价于「关」。
        // 直接 `setFieldsValue(cfg)` 会把开关显示成关闭，**而后端 `None` 的真实语义是
        // 「默认开启」**（`init/database.rs:403` 的 `unwrap_or(true)`）⇒ 界面在说谎。
        // 这里归一到后端语义，保证「看到的就是实际生效的」。
        form.setFieldsValue({ ...cfg, fallback_to_sqlite: cfg.fallback_to_sqlite ?? true });
      })
      .catch(logIpcError("get_db_config"))
      .finally(() => setInitialLoading(false));
    // P2-9: 同时加载 schema 状态
    refreshSchemaStatus();
  }, [form, refreshSchemaStatus]);

  const handleSave = async () => {
    setLoading(true);
    try {
      const values = await form.validateFields();
      await invoke("save_db_config", { config: values });
      message.success(t("settings.database.saved"));
    } catch (e) {
      logIpcError("save_db_config")(e);
    } finally {
      setLoading(false);
    }
  };

  const handleTest = async () => {
    setTesting(true);
    try {
      const values = await form.validateFields();
      // 成功路径**只由 i18n 供文案**：后端签名已改为 `Result<(), String>`，
      // 不再回传 `"连接成功"` 这类自由文本（否则非中文语言用户会看到中文，
      // 且本行会退化成 `result || t(...)` 那种「后端说了算」的隐式契约）。
      await invoke<void>("test_db_connection", { config: values });
      message.success(t("settings.database.testSuccess"));
    } catch (e) {
      // 展示取舍：**本地化原因**与**底层技术原因**必须同时给到用户。
      // `translateBackendError` 命中码时只返回译文（`errorI18n.ts:146-163`，
      // `detail` 不再出现在结果里），所以本地化会以「丢根因」为代价 ——
      // 用户看到「无法连接到数据库」却拿不到 sqlx 的原文，无法自助排查
      // （是 DNS？是密码错？是数据库不存在？）。故这里另行取
      // `parseBackendError(e).detail`（后端放在 detail 里的 sqlx 原文）拼在其后。
      // 命中码时两者不同 ⇒ 拼接；未命中码（如浏览器 mock 抛的纯文本）时
      // 两者相同 ⇒ 不拼，避免同一句话出现两遍。
      const parsed = parseBackendError(e);
      const localized = translateBackendError(e);
      const reason = parsed.detail && parsed.detail !== localized
        ? `${localized} — ${parsed.detail}`
        : localized;
      // 有结构化码时**不再套 `testFailed` 模板**：`error.*` 的译文本身已是自足的原因句，
      // 而模板前缀「连接失败：」对 `DB_QUERY_VERIFY_FAILED`（连接其实已建立、只是
      // 验证查询未通过）会自相矛盾。无码（浏览器 mock / 未知失败）时才用模板兜住。
      message.error(
        parsed.code ? reason : t("settings.database.testFailed", { error: reason }),
      );
      logIpcError("test_db_connection")(e);
    } finally {
      setTesting(false);
    }
  };

  if (initialLoading) {
    return (
      <Card title={t("settings.database.title")} style={{ marginBottom: 16 }}>
        <Typography.Text>{t("settings.database.loading")}</Typography.Text>
      </Card>
    );
  }

  return (
    <Card
      title={t("settings.database.title")}
      style={{ marginBottom: 16 }}
      extra={
        <Space>
          <Button onClick={handleTest} loading={testing}>
            {t("settings.database.testButton")}
          </Button>
          <Button type="primary" onClick={handleSave} loading={loading}>
            {t("settings.database.saveButton")}
          </Button>
        </Space>
      }
    >
      <Alert
        type="info"
        showIcon
        style={{ marginBottom: 16 }}
        title={t("settings.database.restartHint")}
      />

      <Form
        form={form}
        layout="vertical"
        initialValues={{ db_type: "sqlite", pg_port: 5432, use_ssl: false, fallback_to_sqlite: true }}
      >
        <Form.Item name="db_type" label={t("settings.database.typeLabel")}>
          <Radio.Group>
            <Radio value="sqlite">{t("settings.database.typeSqlite")}</Radio>
            <Radio value="postgres">{t("settings.database.typePostgres")}</Radio>
          </Radio.Group>
        </Form.Item>

        {dbType === "sqlite"
          ? (
            <Form.Item
              name="sqlite_path"
              label={t("settings.database.sqlitePathLabel")}
              tooltip={t("settings.database.sqliteNote")}
            >
              <Input placeholder={t("settings.database.sqlitePathPlaceholder")} />
            </Form.Item>
          )
          : (
            <>
              <Alert
                type="warning"
                showIcon
                style={{ marginBottom: 16 }}
                title={t("settings.database.pgNote")}
              />
              <Form.Item
                name="pg_host"
                label={t("settings.database.pgHostLabel")}
              >
                <Input placeholder={t("settings.database.pgHostPlaceholder")} />
              </Form.Item>
              <Form.Item
                name="pg_port"
                label={t("settings.database.pgPortLabel")}
              >
                <InputNumber min={1} max={65535} style={{ width: "100%" }} />
              </Form.Item>
              <Form.Item
                name="pg_database"
                label={t("settings.database.pgDatabaseLabel")}
              >
                <Input placeholder={t("settings.database.pgDatabasePlaceholder")} />
              </Form.Item>
              <Form.Item
                name="pg_user"
                label={t("settings.database.pgUserLabel")}
              >
                <Input placeholder={t("settings.database.pgUserPlaceholder")} />
              </Form.Item>
              <Form.Item
                name="pg_password"
                label={t("settings.database.pgPasswordLabel")}
              >
                <Input.Password
                  placeholder={t("settings.database.pgPasswordPlaceholder")}
                />
              </Form.Item>
              <Form.Item
                name="pg_schema"
                label={t("settings.database.pgSchemaLabel")}
              >
                <Input placeholder={t("settings.database.pgSchemaPlaceholder")} />
              </Form.Item>
              <Form.Item
                name="use_ssl"
                label={t("settings.database.useSslLabel")}
                valuePropName="checked"
              >
                <Switch />
              </Form.Item>
            </>
          )}

        {
          /* ⚠ 本项**无条件渲染**，刻意不放进上面 `dbType === "sqlite" ? … : …` 的分支里：
             分支里的 Form.Item 在切换 dbType 时会卸载，字段是否仍出现在
             `validateFields()` 的结果里就取决于 antd 的卸载 / `preserve` 细节 ——
             而 `handleSave` 把该结果整体当作 `DbConfig` 发送，字段一旦缺席就等于把它
             静默写空（这正是本项本轮修的那个 bug）。
             非 PG 类型下用 `disabled` 表达「只对 postgres 有意义」，值仍保留在表单里。 */
        }
        <Form.Item
          name="fallback_to_sqlite"
          label={t("settings.database.fallbackToSqlite")}
          tooltip={t("settings.database.fallbackToSqliteHint")}
          valuePropName="checked"
        >
          <Switch disabled={dbType === "sqlite"} />
        </Form.Item>
      </Form>

      {/* 数据库结构状态：由声明式引擎（reconcile）收敛，不再有版本化迁移 */}
      <Card
        size="small"
        title={t("settings.database.schemaStatusTitle")}
        style={{ marginTop: 16 }}
        extra={
          <Space>
            <Button
              size="small"
              type="primary"
              danger
              loading={repairing}
              onClick={handleRepairSchema}
            >
              {repairing ? t("settings.database.schemaRepairRunning") : t("settings.database.schemaRepairButton")}
            </Button>
            <Button
              size="small"
              onClick={handleRefreshStatus}
              loading={schemaLoading}
            >
              {t("settings.database.schemaStatusRefresh")}
            </Button>
          </Space>
        }
      >
        {schemaError
          ? (
            <Alert
              type="error"
              showIcon
              title={t("settings.database.schemaStatusError", { error: schemaError })}
            />
          )
          : schemaStatus
          ? (
            <>
              {/* 探测失败必须走 warning：此时结构面数字全为 0，不可信，不能报「已收敛」 */}
              <Alert
                type={schemaStatus.probe_error || schemaStatus.pending_apply > 0 ? "warning" : "success"}
                showIcon
                title={schemaStatus.probe_error
                  ? t("settings.database.schemaStatusProbeFailed", { error: schemaStatus.probe_error })
                  : schemaStatus.pending_apply > 0
                  ? t("settings.database.schemaStatusPending", { count: schemaStatus.pending_apply })
                  : t("settings.database.schemaStatusUpToDate", {
                    actual: schemaStatus.tables_actual,
                    expected: schemaStatus.tables_expected,
                  })}
              />
              {/* 以下均非错误，只是「已知限制 / 需人工 / 纯文本漂移」的说明，故弱化展示 */}
              {
                /* 方言必须显示：用户要求「跑在 sqlite 还是 postgres 由设置决定、不能硬编码」，
                  而这一行是用户唯一能核对实际方言的地方。
                  ⚠ 后端在探测失败时把 `dialect` 置为**空串**（`migrations/mod.rs:356`），
                  故空串不渲染 —— 否则会多出一行「当前方言：」后面空着，又成一张说谎的卡片。

                  ⚠ 值取后端的**规范标识符**（`"sqlite"` / `"postgres"`，见
                  `reconcile/model.rs:52-58` 的 `Serialize` 与 `reconcile/status.rs:140`），
                  **刻意不做前端映射表、也不本地化**（team-lead 2026-09-16 拍板）：
                  映射表等于手抄一份后端值域，`Dialect` 枚举一旦扩展就会漏映射、静默显示
                  空值或错值；本地化则等于把同一份值域抄 11 份。这里是「原样呈现证据」，
                  不是展示名 —— 请不要「好心」加映射或 i18n 化。 */
              }
              {schemaStatus.dialect && (
                <Typography.Text type="secondary" style={{ display: "block", marginTop: 4 }}>
                  {t("settings.database.schemaStatusDialect", { dialect: schemaStatus.dialect })}
                </Typography.Text>
              )}
              {schemaStatus.pending_unsupported > 0 && (
                <Typography.Text type="secondary" style={{ display: "block", marginTop: 4 }}>
                  {t("settings.database.schemaStatusUnsupported", { count: schemaStatus.pending_unsupported })}
                </Typography.Text>
              )}
              {schemaStatus.pending_manual > 0 && (
                <Typography.Text type="secondary" style={{ display: "block", marginTop: 4 }}>
                  {t("settings.database.schemaStatusManual", { count: schemaStatus.pending_manual })}
                </Typography.Text>
              )}
              {schemaStatus.advisories > 0 && (
                <Typography.Text type="secondary" style={{ display: "block", marginTop: 4 }}>
                  {t("settings.database.schemaStatusAdvisories", { count: schemaStatus.advisories })}
                </Typography.Text>
              )}
              {schemaStatus.notes.map((note, i) => (
                <Typography.Text
                  key={`${i}:${note}`}
                  type="secondary"
                  style={{ display: "block", marginTop: 4 }}
                >
                  {note}
                </Typography.Text>
              ))}
            </>
          )
          : (
            <Typography.Text type="secondary">
              {t("settings.database.schemaStatusLoadFailed")}
            </Typography.Text>
          )}
        {repairError && (
          <Alert
            type="error"
            showIcon
            style={{ marginTop: 8 }}
            title={t("settings.database.schemaRepairFailed", { error: repairError })}
          />
        )}
        {/* 部分表未对照成功 ≠ 整体失败：用 secondary 明细而非红色 Alert */}
        {repairIssues.length > 0 && (
          <>
            <Typography.Text type="secondary" style={{ display: "block", marginTop: 8 }}>
              {t("settings.database.schemaRepairPartial", { count: repairIssues.length })}
            </Typography.Text>
            {repairIssues.slice(0, REPAIR_ISSUES_SHOWN).map((issue, i) => (
              <Typography.Text
                key={`${i}:${issue}`}
                type="secondary"
                style={{ display: "block", marginTop: 4 }}
              >
                {issue}
              </Typography.Text>
            ))}
            {repairIssues.length > REPAIR_ISSUES_SHOWN && (
              <Typography.Text type="secondary" style={{ display: "block", marginTop: 4 }}>
                {`… +${repairIssues.length - REPAIR_ISSUES_SHOWN}`}
              </Typography.Text>
            )}
          </>
        )}
      </Card>
    </Card>
  );
}
