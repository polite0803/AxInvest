import type { AiChatAction } from "@/components/workflow/types/workflow.types";
import { invoke, listen } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import {
  Button,
  Card,
  Checkbox,
  DatePicker,
  Empty,
  Input,
  message,
  Modal,
  Popconfirm,
  Select,
  Space,
  Spin,
  Switch,
  Table,
  Tag,
  Typography,
} from "antd";
import type { Dayjs } from "dayjs";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

interface ReflectionRow {
  id: string;
  stockCode: string;
  stockName: string;
  asOfDate: string;
  hindsightDate: string;
  actualOutcome: string;
  whatWentWrong: string | null;
  missedSignals: string | null;
  fixForFuture: string | null;
  reflectionDepth: string;
  minConfidenceThreshold: number;
  status: string;
  createdAt: number;
  decisionJson?: string;
  blackboardSnapshot?: string;
}

interface CronJobResponse {
  id: string;
  name: string;
  schedule: string;
  status: string;
}

export function ReflectionPanel() {
  const { t } = useTranslation();

  const CRON_PRESETS = [
    { label: t("stockAnalysis.reflection.daily0600"), value: "0 6 * * *" },
    { label: t("stockAnalysis.reflection.daily1200"), value: "0 12 * * *" },
    { label: t("stockAnalysis.reflection.daily1800"), value: "0 18 * * *" },
    { label: t("stockAnalysis.reflection.weekday0600"), value: "0 6 * * 1-5" },
  ];

  const [reflections, setReflections] = useState<ReflectionRow[]>([]);
  const [cronJobs, setCronJobs] = useState<CronJobResponse[]>([]);
  const [cronExpr, setCronExpr] = useState("0 6 * * *");
  const [threshold, setThreshold] = useState(0);
  const [depth, setDepth] = useState("light");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const [manualAsOf, setManualAsOf] = useState<Dayjs | null>(null);
  const [manualOutcome, setManualOutcome] = useState("");
  const [manualDepth, setManualDepth] = useState("light");
  const [running, setRunning] = useState(false);

  // 个股分析页内自动取当前股票代码（首次加载时填入）
  const stockCodeFromStore = useStockAnalysisStore((s) => s.stockCode);
  const [manualCode, setManualCode] = useState(stockCodeFromStore ?? "");
  // 当 store 股票代码变化但 manualCode 为空时自动填充
  const prevCodeRef = useRef(stockCodeFromStore);
  useEffect(() => {
    if (!manualCode && stockCodeFromStore && stockCodeFromStore !== prevCodeRef.current) {
      prevCodeRef.current = stockCodeFromStore;
      setManualCode(stockCodeFromStore);
    }
  }, [stockCodeFromStore, manualCode]);

  // Bug 4 修复: 统一请求级取消令牌,避免 useEffect 与 onClick 双轨加载
  // 各自维护一份 cancelled 标记造成的乱序写入。
  const loadTokenRef = useRef(0);
  const load = async () => {
    const myToken = ++loadTokenRef.current;
    try {
      const [r, c] = await Promise.all([
        invoke<ReflectionRow[]>("list_reflections", {}),
        invoke<CronJobResponse[]>("list_validate_decisions_crons", {}),
      ]);
      if (myToken !== loadTokenRef.current) { return; }
      if (Array.isArray(r)) { setReflections(r); }
      if (Array.isArray(c)) { setCronJobs(c); }
    } catch { /* ignore */ }
  };

  useEffect(() => {
    // Bug 4 修复: 走统一入口
    Promise.resolve().then(() => load());
  }, []);

  // 后端 CronJobStatus 用 #[serde(rename_all = "snake_case")] 序列化,
  // 实际 status 字符串是 "active" / "paused" / "disabled"（小写）
  // 这里必须用小写比较,否则 isEnabled/activeCron 永远错误。
  const activeCron = cronJobs.find((j) => j.status === "active") ?? cronJobs[0];
  const isEnabled = cronJobs.some((j) => j.status === "active");

  const toggleCron = async (enable: boolean) => {
    try {
      if (enable && !activeCron) {
        await invoke("create_validate_decisions_cron", {
          cronExpression: cronExpr,
          minConfidenceThreshold: threshold,
          reflectionDepth: depth,
          enabled: true,
        });
      } else if (activeCron) {
        await invoke("toggle_validate_decisions_cron", { id: activeCron.id, enabled: enable });
      }
      await load();
    } catch { /* ignore */ }
  };

  const deleteCron = async () => {
    if (!activeCron) { return; }
    try {
      await invoke("delete_validate_decisions_cron", { id: activeCron.id });
      await load();
    } catch { /* ignore */ }
  };

  const deleteReflection = async (id: string) => {
    try {
      await invoke("delete_reflection", { reflectionId: id });
      message.success(t("stockAnalysis.reflection.deleteSuccess"));
      await load();
    } catch (e) {
      message.error(t("stockAnalysis.reflection.deleteFailed", { error: String(e) }));
    }
  };

  const handleManualReflection = async () => {
    if (!manualCode.trim() || !manualAsOf || !manualAsOf.isValid() || !manualOutcome.trim()) {
      message.warning(t("stockAnalysis.reflection.fillRequired"));
      return;
    }
    setRunning(true);
    try {
      await invoke("run_reflection_now", {
        stockCode: manualCode.trim(),
        stockName: "",
        asOfDate: manualAsOf.format("YYYY-MM-DD"),
        actualOutcome: manualOutcome.trim(),
        reflectionDepth: manualDepth,
      });
      message.success(t("stockAnalysis.reflection.triggerSuccess"));
      await load();
    } catch (e) {
      message.error(t("stockAnalysis.reflection.triggerFailed", { error: String(e) }));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div style={{ padding: 16 }}>
      {/* 手动触发 */}
      <Card
        title={t("stockAnalysis.reflection.manualTitle")}
        size="small"
        style={{ marginBottom: 16 }}
      >
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space wrap>
            <Input
              placeholder={t("stockAnalysis.reflection.placeholderCode")}
              value={manualCode}
              onChange={(e) => setManualCode(e.target.value)}
              style={{ width: 120 }}
            />
            <DatePicker
              placeholder={t("stockAnalysis.reflection.placeholderDate")}
              value={manualAsOf}
              onChange={(d) => setManualAsOf(d)}
              format="YYYY-MM-DD"
              allowClear
              style={{ width: 160 }}
            />
            <Input
              placeholder={t("stockAnalysis.reflection.placeholderOutcome")}
              value={manualOutcome}
              onChange={(e) => setManualOutcome(e.target.value)}
              style={{ width: 200 }}
            />
            <Select
              value={manualDepth}
              onChange={setManualDepth}
              options={[
                { label: t("stockAnalysis.reflection.depthLight"), value: "light" },
                { label: t("stockAnalysis.reflection.depthDeep"), value: "deep" },
              ]}
              style={{ width: 100 }}
            />
            <Button
              type="primary"
              onClick={handleManualReflection}
              loading={running}
              disabled={running}
            >
              {t("stockAnalysis.reflection.startReflection")}
            </Button>
          </Space>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t("stockAnalysis.reflection.manualHint")}
          </Text>
        </Space>
      </Card>

      {/* 定时校验配置 */}
      <Card title={t("stockAnalysis.reflection.scheduleTitle")} size="small" style={{ marginBottom: 16 }}>
        <Space direction="vertical" style={{ width: "100%" }}>
          <Space>
            <Switch checked={isEnabled} onChange={toggleCron} />
            <Text>{t("stockAnalysis.reflection.scheduleDesc")}</Text>
          </Space>
          <Space wrap>
            <Select
              value={cronExpr}
              onChange={setCronExpr}
              options={CRON_PRESETS}
              style={{ width: 150 }}
              disabled={isEnabled}
            />
            <Select
              value={depth}
              onChange={setDepth}
              options={[
                { label: t("stockAnalysis.reflection.depthLight"), value: "light" },
                { label: t("stockAnalysis.reflection.depthDeep"), value: "deep" },
              ]}
              style={{ width: 110 }}
              disabled={isEnabled}
            />
            <Input
              type="number"
              min={0}
              max={100}
              value={threshold}
              onChange={(e) => setThreshold(Number(e.target.value))}
              style={{ width: 100 }}
              disabled={isEnabled}
              addonAfter={t("stockAnalysis.reflection.confidence")}
            />
            {activeCron && (
              <Button danger size="small" onClick={deleteCron}>
                {t("stockAnalysis.reflection.deleteBtn")}
              </Button>
            )}
          </Space>
          {activeCron && (
            <Text type="secondary">
              {t("stockAnalysis.reflection.scheduleDescFull", {
                schedule: activeCron.schedule,
                status: isEnabled
                  ? t("stockAnalysis.reflection.running")
                  : t("stockAnalysis.reflection.paused"),
              })}
            </Text>
          )}
        </Space>
      </Card>

      {/* 反思历史 */}
      <Card
        title={t("stockAnalysis.reflection.historyTitle")}
        size="small"
        extra={<Button size="small" onClick={load}>{t("stockAnalysis.reflection.refreshBtn")}</Button>}
      >
        {reflections.length === 0
          ? <Empty description={t("stockAnalysis.reflection.empty")} />
          : (
            <Table
              dataSource={reflections}
              rowKey="id"
              pagination={{ pageSize: 10, size: "small" }}
              size="small"
              scroll={{ x: 800 }}
              expandable={{
                expandedRowKeys: expandedId ? [expandedId] : [],
                onExpandedRowsChange: (keys: readonly React.Key[]) => setExpandedId(keys[0] as string || null),
                expandedRowRender: (r: ReflectionRow) => (
                  <ExpandedReflectionRow row={r} t={t as (key: string, opts?: object) => string} />
                ),
              }}
            >
              <Table.Column title={t("stockAnalysis.reflection.colCode")} dataIndex="stockCode" width={90} />
              <Table.Column title={t("stockAnalysis.reflection.colName")} dataIndex="stockName" width={100} />
              <Table.Column title={t("stockAnalysis.reflection.colAsOf")} dataIndex="asOfDate" width={100} />
              <Table.Column
                title={t("stockAnalysis.reflection.colResult")}
                dataIndex="actualOutcome"
                width={160}
                render={(v: string) => <Text type="danger">{v}</Text>}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colStatus")}
                dataIndex="status"
                width={140}
                render={(v: string) => {
                  const color = v === "completed" ? "green" : v.startsWith("failed:") ? "red" : "orange";
                  return <Tag color={color}>{v || "running"}</Tag>;
                }}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colCause")}
                dataIndex="whatWentWrong"
                ellipsis
                render={(v: string | null) => v || "-"}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colTime")}
                dataIndex="createdAt"
                width={150}
                render={(v: number) => new Date(v).toLocaleDateString("zh-CN")}
              />
              <Table.Column
                title={t("stockAnalysis.reflection.colAction")}
                width={90}
                fixed="right"
                render={(_: unknown, r: ReflectionRow) => (
                  <Popconfirm
                    title={t("stockAnalysis.reflection.deleteConfirm")}
                    okText={t("common.confirm")}
                    cancelText={t("common.cancel")}
                    onConfirm={() => deleteReflection(r.id)}
                  >
                    <Button danger size="small">
                      {t("stockAnalysis.reflection.deleteBtn")}
                    </Button>
                  </Popconfirm>
                )}
              />
            </Table>
          )}
      </Card>
    </div>
  );
}

function formatJson(val: string | null): string {
  if (!val) { return "-"; }
  try {
    const parsed = JSON.parse(val);
    return Array.isArray(parsed) ? parsed.join("; ") : val;
  } catch {
    return val;
  }
}

interface ParamSuggestion {
  param: string;
  current_value: number;
  suggested_value: number;
  reason: string;
}

function parseParamsSuggestion(row: ReflectionRow): ParamSuggestion[] {
  if (!row.decisionJson) { return []; }
  try {
    const parsed = JSON.parse(row.decisionJson);
    // 后端 stock_workflow.rs 写入的 decision_json 结构是
    //   { reflection: { what_went_wrong, missed_signals, fix_for_future, params_suggestion, ... } }
    // 因此 params_suggestion 一定在 parsed.reflection 下。
    // Bug 11 修复: 删掉 `parsed?.params_suggestion` 这条死分支(永远不会命中),
    // 避免误导后续维护者。
    const raw = parsed?.reflection?.params_suggestion;
    if (Array.isArray(raw)) {
      return raw as ParamSuggestion[];
    }
    return [];
  } catch {
    return [];
  }
}

function renderPrettyJson(label: string, raw: string, maxHeight: number) {
  let body = raw;
  try {
    body = JSON.stringify(JSON.parse(raw), null, 2);
  } catch { /* keep raw */ }
  return (
    <>
      <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 4 }}>{label}</div>
      <pre
        style={{
          fontSize: 11,
          background: "var(--bg-elevated)",
          padding: 8,
          borderRadius: 4,
          maxHeight,
          overflow: "auto",
          whiteSpace: "pre-wrap",
          wordBreak: "break-all",
        }}
      >
        {body}
      </pre>
    </>
  );
}

function renderBlackboardSnapshot(raw: string) {
  let body = raw;
  try {
    const obj = JSON.parse(raw);
    if (obj && typeof obj === "object" && obj.reflection !== undefined) {
      body = JSON.stringify({ reflection: obj.reflection }, null, 2);
    } else {
      body = JSON.stringify(obj, null, 2);
    }
  } catch { /* keep raw */ }
  return (
    <>
      <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 8 }}>
        blackboardSnapshot (工作流原始结果):
      </div>
      <pre
        style={{
          fontSize: 11,
          background: "var(--bg-elevated)",
          padding: 8,
          borderRadius: 4,
          maxHeight: 400,
          overflow: "auto",
          whiteSpace: "pre-wrap",
          wordBreak: "break-all",
        }}
      >
        {body}
      </pre>
    </>
  );
}

function ExpandedReflectionRow({ row, t }: { row: ReflectionRow; t: (key: string, opts?: object) => string }) {
  const suggestions = parseParamsSuggestion(row);
  const [checkedParams, setCheckedParams] = useState<string[]>([]);
  const [applying, setApplying] = useState(false);

  // AI 修改建议状态
  const [aiModalOpen, setAiModalOpen] = useState(false);
  const [aiStreaming, setAiStreaming] = useState(false);
  const [aiContent, setAiContent] = useState("");
  const [aiActions, setAiActions] = useState<AiChatAction[]>([]);
  const [aiApplying, setAiApplying] = useState(false);
  const aiUnlistensRef = useRef<Array<() => void>>([]);

  const handleApply = async () => {
    const selected = suggestions.filter((s) => checkedParams.includes(s.param));
    if (selected.length === 0) { return; }
    setApplying(true);
    try {
      await invoke("apply_param_suggestions", {
        updates: selected.map((s) => ({ param: s.param, value: s.suggested_value })),
      });
      message.success(t("stockAnalysis.reflection.applySuccess"));
      setCheckedParams([]);
    } catch (e) {
      message.error(t("stockAnalysis.reflection.applyFailed", { error: String(e) }));
    } finally {
      setApplying(false);
    }
  };

  /**
   * 调 workflow_ai_chat_stream（直接走后端命令，不依赖 workflowEditorStore），
   * 把反思内容作为 user message 注入，让 AI LLM 基于上游纯净 system_prompt
   * 输出 5 种 v2.0 :::action 块，由前端解析后在弹窗内展示 diff preview。
   * 这条路径让 ReflectionPanel 单独完成"反思 → 修改闭环"，不依赖 workflow 页面。
   */
  const handleAiAssist = async () => {
    setAiModalOpen(true);
    setAiStreaming(true);
    setAiContent("");
    setAiActions([]);
    // 清理旧的 listener
    aiUnlistensRef.current.forEach((fn) => fn());
    aiUnlistensRef.current = [];

    const sessionId = `reflection-${row.id}-${Date.now()}`;
    const userMessage = buildAiAssistPrompt(row);

    let accumulated = "";
    try {
      const chunkUnlisten = await listen<{
        conversation_id: string;
        message_id: string;
        chunk: { content: string | null; done: boolean };
      }>("workflow-ai-chat-chunk", (event) => {
        if (event.payload.conversation_id !== sessionId) { return; }
        const c = event.payload.chunk;
        if (c.content) { accumulated += c.content; }
        if (c.done) {
          setAiContent(accumulated);
          setAiActions(parseActionsFromAccumulated(accumulated));
          setAiStreaming(false);
        } else {
          setAiContent(accumulated + "▍");
        }
      });
      const errorUnlisten = await listen<{ conversation_id: string; error: string }>(
        "workflow-ai-chat-error",
        (event) => {
          if (event.payload.conversation_id !== sessionId) { return; }
          setAiContent((prev) => prev + `\n\n❌ ${event.payload.error}`);
          setAiStreaming(false);
        },
      );
      aiUnlistensRef.current = [chunkUnlisten, errorUnlisten];

      await invoke("workflow_ai_chat_stream", {
        message: userMessage,
        history: [],
        currentNodes: undefined,
        currentEdges: undefined,
        sessionId,
      });
    } catch (e) {
      setAiContent((prev) => prev + `\n\n❌ ${String(e)}`);
      setAiStreaming(false);
    }
  };

  /**
   * 用户确认后调上游 `apply_diff_with_validation` 聚合器（workflow_ai_apply.rs:491）：
   * - actions 直接传 ChatAction[]（强类型 enum，与 AiChatAction discriminated union 1:1）
   * - validation: ValidationSpec（{ type, params }）
   * - rollback_on_failure: bool（默认 true，单条 action 失败 / validation 失败时倒序回滚）
   */
  const handleApplyAiActions = async () => {
    if (aiActions.length === 0) { return; }
    setAiApplying(true);
    try {
      const result = await invoke<{
        validation_passed: boolean;
        applied_count: number;
        applied: string[];
        validation_metrics: unknown;
        rolled_back: boolean;
        error: string | null;
      }>("apply_diff_with_validation", {
        actions: aiActions,
        validation: { type: "noop", params: {} },
        rollback_on_failure: true,
      });
      if (result.validation_passed) {
        message.success(
          t("stockAnalysis.reflection.aiApplySuccess", { count: result.applied_count }),
        );
        setAiModalOpen(false);
      } else {
        message.error(
          t("stockAnalysis.reflection.aiApplyFailed", {
            error: result.error ?? "未知",
            rollback: result.rolled_back ? t("stockAnalysis.reflection.rolledBack") : "",
          }),
        );
      }
    } catch (e) {
      message.error(t("stockAnalysis.reflection.aiApplyFailed", { error: String(e), rollback: "" }));
    } finally {
      setAiApplying(false);
    }
  };

  const handleAiModalClose = () => {
    setAiModalOpen(false);
    aiUnlistensRef.current.forEach((fn) => fn());
    aiUnlistensRef.current = [];
  };

  return (
    <div style={{ padding: "8px 0" }}>
      <Space direction="vertical" style={{ width: "100%" }}>
        <Space>
          <Tag color="red">{row.actualOutcome}</Tag>
          <Tag>
            {row.reflectionDepth === "deep"
              ? t("stockAnalysis.reflection.depthDeepLabel")
              : t("stockAnalysis.reflection.depthLightLabel")}
          </Tag>
          <Text type="secondary">
            {t("stockAnalysis.reflection.asOfLabel", {
              asOf: row.asOfDate,
              hindsight: row.hindsightDate,
            })}
          </Text>
        </Space>
        <div>
          <Text strong>{t("stockAnalysis.reflection.causeLabel")}</Text>
          <Text>{row.whatWentWrong || "-"}</Text>
        </div>
        <div>
          <Text strong>{t("stockAnalysis.reflection.signalsLabel")}</Text>
          <Text>{formatJson(row.missedSignals)}</Text>
        </div>
        <div>
          <Text strong>{t("stockAnalysis.reflection.improveLabel")}</Text>
          <Text>{row.fixForFuture || "-"}</Text>
        </div>
        {(row.decisionJson || row.blackboardSnapshot) && (
          <details style={{ marginTop: 4 }} open={!row.whatWentWrong && !row.fixForFuture}>
            <summary style={{ cursor: "pointer", color: "var(--muted)", fontSize: 12 }}>
              {t("stockAnalysis.reflection.rawOutputTitle")}
            </summary>
            {row.decisionJson && renderPrettyJson("decisionJson (LLM 提取后):", row.decisionJson, 200)}
            {row.blackboardSnapshot && renderBlackboardSnapshot(row.blackboardSnapshot)}
          </details>
        )}
        {suggestions.length > 0 && (
          <div>
            <Text strong style={{ fontSize: 13 }}>
              ⚙ {t("stockAnalysis.reflection.paramSuggestTitle")}
            </Text>
            <div style={{ marginTop: 8 }}>
              <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
                <thead>
                  <tr style={{ borderBottom: "1px solid #d9d9d9" }}>
                    <th style={{ padding: "4px 8px", textAlign: "left", width: 32 }} />
                    <th style={{ padding: "4px 8px", textAlign: "left" }}>
                      {t("stockAnalysis.reflection.paramCol")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "right", width: 80 }}>
                      {t("stockAnalysis.reflection.paramCurrent")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "right", width: 80 }}>
                      {t("stockAnalysis.reflection.paramSuggested")}
                    </th>
                    <th style={{ padding: "4px 8px", textAlign: "left" }}>
                      {t("stockAnalysis.reflection.paramReason")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {suggestions.map((s) => (
                    <tr key={s.param} style={{ borderBottom: "1px solid #f0f0f0" }}>
                      <td style={{ padding: "4px 8px" }}>
                        <Checkbox
                          checked={checkedParams.includes(s.param)}
                          onChange={(e) => {
                            setCheckedParams(
                              e.target.checked
                                ? [...checkedParams, s.param]
                                : checkedParams.filter((p) => p !== s.param),
                            );
                          }}
                        />
                      </td>
                      <td style={{ padding: "4px 8px", fontFamily: "monospace" }}>
                        {s.param}
                      </td>
                      <td style={{ padding: "4px 8px", textAlign: "right" }}>
                        {s.current_value}
                      </td>
                      <td style={{ padding: "4px 8px", textAlign: "right" }}>
                        <Text strong style={{ color: "#1890ff" }}>{s.suggested_value}</Text>
                      </td>
                      <td style={{ padding: "4px 8px" }}>
                        <Text type="secondary">{s.reason}</Text>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <Space style={{ marginTop: 8 }}>
                <Button
                  type="primary"
                  size="small"
                  loading={applying}
                  disabled={checkedParams.length === 0}
                  onClick={handleApply}
                >
                  {t("stockAnalysis.reflection.applyParamsBtn", { count: checkedParams.length })}
                </Button>
                <Button
                  size="small"
                  onClick={handleAiAssist}
                  disabled={aiStreaming}
                >
                  {aiStreaming
                    ? (
                      <>
                        <Spin size="small" /> {t("stockAnalysis.reflection.aiAssistLoading")}
                      </>
                    )
                    : t("stockAnalysis.reflection.aiAssistBtn")}
                </Button>
              </Space>
            </div>
          </div>
        )}
        {suggestions.length === 0 && (
          <div style={{ marginTop: 4 }}>
            <Button
              size="small"
              onClick={handleAiAssist}
              disabled={aiStreaming}
            >
              {aiStreaming
                ? (
                  <>
                    <Spin size="small" /> {t("stockAnalysis.reflection.aiAssistLoading")}
                  </>
                )
                : t("stockAnalysis.reflection.aiAssistBtn")}
            </Button>
          </div>
        )}
      </Space>

      {/* AI 修改建议弹窗：流式响应 + diff preview + 一键应用 */}
      <Modal
        title={t("stockAnalysis.reflection.aiAssistModalTitle")}
        open={aiModalOpen}
        onCancel={handleAiModalClose}
        width={760}
        footer={[
          <Button key="cancel" onClick={handleAiModalClose}>
            {t("common.cancel")}
          </Button>,
          <Button
            key="apply"
            type="primary"
            loading={aiApplying}
            disabled={aiActions.length === 0 || aiStreaming}
            onClick={handleApplyAiActions}
          >
            {t("stockAnalysis.reflection.aiApplyBtn", { count: aiActions.length })}
          </Button>,
        ]}
      >
        {aiStreaming && aiActions.length === 0 && (
          <div style={{ marginBottom: 12 }}>
            <Spin /> <Text type="secondary">{t("stockAnalysis.reflection.aiAssistThinking")}</Text>
          </div>
        )}
        {aiActions.length > 0 && (
          <div style={{ marginBottom: 12 }}>
            <Text strong style={{ fontSize: 13 }}>
              {t("stockAnalysis.reflection.aiDiffPreviewTitle", { count: aiActions.length })}
            </Text>
            <div style={{ marginTop: 8 }}>
              {aiActions.map((a, i) => (
                <div
                  key={i}
                  style={{
                    border: "1px solid var(--border-color, #d9d9d9)",
                    borderRadius: 4,
                    padding: 8,
                    marginBottom: 6,
                    fontSize: 12,
                  }}
                >
                  <div style={{ marginBottom: 4 }}>
                    <Tag color={ACTION_COLOR[a.action_type] ?? "default"}>
                      {a.action_type}
                    </Tag>
                  </div>
                  <pre
                    style={{
                      fontSize: 11,
                      background: "var(--bg-elevated, #fafafa)",
                      padding: 6,
                      borderRadius: 3,
                      margin: 0,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-all",
                      maxHeight: 180,
                      overflow: "auto",
                    }}
                  >
                    {JSON.stringify(a.data, null, 2)}
                  </pre>
                </div>
              ))}
            </div>
          </div>
        )}
        {aiContent && (
          <details>
            <summary style={{ cursor: "pointer", color: "var(--muted)", fontSize: 12 }}>
              {t("stockAnalysis.reflection.aiRawOutputTitle")}
            </summary>
            <pre
              style={{
                fontSize: 11,
                background: "var(--bg-elevated, #fafafa)",
                padding: 8,
                borderRadius: 4,
                marginTop: 4,
                whiteSpace: "pre-wrap",
                wordBreak: "break-all",
                maxHeight: 200,
                overflow: "auto",
              }}
            >
              {aiContent}
            </pre>
          </details>
        )}
      </Modal>
    </div>
  );
}

// =========================================================================
// AI 修改建议辅助函数
// 把反思上下文打包成 user message，调后端 workflow_ai_chat_stream。
// 让上游 LLM（基于纯净 system_prompt）输出 5 种 v2.0 :::action 块。
// =========================================================================

const REFLECTION_TEMPLATE_ID = "stock-analysis";

const ACTION_COLOR: Record<string, string> = {
  update_variable: "purple",
  rollback_to_version: "orange",
  update_input_mapping: "cyan",
  edit_asset_file: "magenta",
  apply_diff_with_validation: "green",
};

function buildAiAssistPrompt(row: ReflectionRow): string {
  // 解析 decisionJson 里的 params_suggestion / implementation_tier / code_diff_proposal
  let paramsText = "(无)";
  let tierText = "L1";
  let diffText = "(无)";
  try {
    const parsed = row.decisionJson ? JSON.parse(row.decisionJson) : null;
    const ref = parsed?.reflection ?? parsed;
    if (Array.isArray(ref?.params_suggestion) && ref.params_suggestion.length > 0) {
      paramsText = ref.params_suggestion
        .map((p: { param: string; suggested_value: unknown; reason: string }) =>
          `  - ${p.param} = ${JSON.stringify(p.suggested_value)}（${p.reason ?? ""}）`
        )
        .join("\n");
    }
    if (ref?.implementation_tier) {
      tierText = String(ref.implementation_tier);
    }
    if (ref?.code_diff_proposal) {
      diffText = String(ref.code_diff_proposal);
    }
  } catch {
    // 忽略解析错误，使用默认占位
  }

  return `你是工作流修改助手。请基于以下反思结论，输出 1-3 个 :::action 块，
让前端用户可以预览 + 一键应用。

**重要：所有 action payload 字段用 snake_case（template_id, anchor_line, rollback_on_failure）。
template_id 固定为 "${REFLECTION_TEMPLATE_ID}"。**

【反思上下文】
- 股票：${row.stockCode} ${row.stockName}
- as-of: ${row.asOfDate} → hindsight: ${row.hindsightDate}
- 实际结果：${row.actualOutcome}
- 反思深度：${row.reflectionDepth}

【根因】${row.whatWentWrong || "(空)"}

【遗漏信号】${row.missedSignals || "(空)"}

【改进】${row.fixForFuture || "(空)"}

【业务层参数建议（params_suggestion）】
${paramsText}

【实现层级（implementation_tier）】${tierText}
（说明：L1 = 改 workflow_template.variables；L2 = 改 .rhai 公式 / 算法约束；L3 = 改 .md 提示词）

【业务层代码 diff 提案（code_diff_proposal）】
${diffText}

【输出格式】
按以下 schema 输出 1-3 个 :::action 块（用 \`\`\`json 包裹也接受）。
前端会自动用 apply_diff_with_validation 聚合，所以你**不需要**输出 apply_diff_with_validation 自身。
edit_asset_file 的 description 必填（≤ 200 字）。

1. 如果是 L1（参数调整）：
   :::action
   {"action_type":"update_variable","data":{"template_id":"${REFLECTION_TEMPLATE_ID}","name":"<变量名>","value":<new_value>}}
   :::

2. 如果是 L2（改 .rhai 公式）：
   :::action
   {"action_type":"edit_asset_file","data":{"path":"src-tauri/src/commands/portfolio-mgr.rhai","operation":"insert_after|replace|delete","anchor_line":<int>,"code":"<新代码段>","description":"<≤200 字理由>"}}
   :::

3. 如果是 L3（改 .md 提示词）：
   :::action
   {"action_type":"edit_asset_file","data":{"path":"<md path>","operation":"insert_after|replace|delete","anchor_line":<int>,"code":"<新提示词段>","description":"<≤200 字理由>"}}
   :::

4. 如果是回滚：
   :::action
   {"action_type":"rollback_to_version","data":{"template_id":"${REFLECTION_TEMPLATE_ID}","target_version":<int>}}
   :::

5. 如果是改 sub-workflow input_mapping：
   :::action
   {"action_type":"update_input_mapping","data":{"node_id":"<nodeId>","mappings":[{"target":"<var>","source":"<var>"}]}}
   :::

不要输出除 :::action 块以外的其他内容（除简短中文解释外）。`;
}

function parseActionsFromAccumulated(content: string): AiChatAction[] {
  const regex = /:::action\s*\n([\s\S]*?)\n:::/g;
  const out: AiChatAction[] = [];
  let match;
  const known = new Set<AiChatAction["action_type"]>([
    "update_variable",
    "rollback_to_version",
    "update_input_mapping",
    "edit_asset_file",
    "apply_diff_with_validation",
  ]);
  while ((match = regex.exec(content)) !== null) {
    try {
      const parsed = JSON.parse(match[1].trim());
      const actionType = parsed.action_type;
      if (!known.has(actionType as AiChatAction["action_type"])) { continue; }
      // 防御：LLM 可能输出 camelCase（templateId/anchorLine/rollbackOnFailure），
      // 归一化为 snake_case 以匹配上游 ChatAction enum 序列化要求。
      const data = camelToSnakeKeys(parsed.data ?? {}) as AiChatAction["data"];
      out.push({ action_type: actionType, data } as AiChatAction);
    } catch {
      // skip invalid JSON
    }
  }
  return out;
}

/** 把对象所有顶层 key 从 camelCase 转为 snake_case（递归）。用于防御 LLM 输出不一致。 */
function camelToSnakeKeys<T = unknown>(v: T): T {
  if (Array.isArray(v)) {
    return v.map((x) => camelToSnakeKeys(x)) as unknown as T;
  }
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
      const snake = k.replace(/[A-Z]/g, (m) => "_" + m.toLowerCase());
      out[snake] = camelToSnakeKeys(val);
    }
    return out as T;
  }
  return v;
}
