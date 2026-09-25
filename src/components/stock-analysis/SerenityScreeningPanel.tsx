import { useStockJump } from "@/hooks/useStockJump";
import i18n from "@/i18n";
import { showBackendError, translateFailureText } from "@/lib/errorI18n";
import { invoke, listen, TimeoutError as InvokeTimeoutError } from "@/lib/invoke";
import {
  type SerenityCandidate,
  type SerenityChainId,
  type StepStage,
  type TrendInfo,
  useSerenityStore,
} from "@/stores/feature/serenityStore";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import {
  AlertOutlined,
  CheckCircleOutlined,
  ClockCircleOutlined,
  DeleteOutlined,
  DownOutlined,
  HistoryOutlined,
  LoadingOutlined,
  PlayCircleOutlined,
  ReloadOutlined,
  RightOutlined,
  StockOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Empty,
  InputNumber,
  Modal,
  Popconfirm,
  Progress,
  Select,
  Space,
  Spin,
  Table,
  Tag,
  Typography,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SerenityCandidateCard } from "./SerenityCandidateCard";

const { Text, Title } = Typography;

// ── invoke 返回类型 ──
interface SerenityResult {
  status?: string;
  candidates?: unknown;
  trends?: TrendInfo[];
  /**
   * 后端从 a-candidate-mapper 的 arguments.summary 透传出来的"为什么没有候选"。
   * 当上游三个瓶颈节点均返回 data_gaps=true 时，模型反幻觉拒绝编造并在此
   * 字段说明原因；前端在 candidates 为空时把它展示给用户。
   */
  emptyReason?: string | null;
}

/// get_reco_detail 返回的单条候选记录（serde camelCase）
interface RecoDetailItem {
  id: string;
  generatedAt: string;
  period: string;
  stockCode: string;
  stockName: string;
  style: string;
  confidence: number;
  synthetic: number;
  seedPoolJson?: string | null;
  pickData?: string | null;
  createdAt: string;
}

/// 从 reco_picks 落库数据还原 SerenityCandidate：
/// 优先解析 seed_pool_json（serenity-screening 工作流写的 candidate 对象），
/// 失败时用基础列构造兜底候选（智能荐股 bottleneck 行的 seed_pool_json 是
/// 推荐池 [[code,name]] 数组，无法解析为单个候选，必须走 fallback）。
function restoreCandidate(item: RecoDetailItem): SerenityCandidate | null {
  // 时间基线回填：行级 generated_at + pick_data.holdingDays（否则卡片
  // 时间基线兜底为渲染当日，历史记录显示的推荐日失真）
  const withBaseline = (c: SerenityCandidate): SerenityCandidate => {
    let holdingDays: number | undefined;
    if (item.pickData) {
      try {
        holdingDays = (JSON.parse(item.pickData) as { holdingDays?: number })?.holdingDays;
      } catch {
        // pick_data 损坏时缺省，卡片自行兜底 20 天
      }
    }
    return { ...c, generated_at: item.generatedAt, holding_days: holdingDays };
  };
  if (item.seedPoolJson) {
    try {
      const parsed = JSON.parse(item.seedPoolJson) as unknown;
      // 只接受单个对象（serenity workflow 写的 candidate）—— 数组（推荐池快照）
      // 和其它 shape 一律走 fallback，否则 SerenityCandidate 字段全是 undefined
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return withBaseline(parsed as SerenityCandidate);
      }
    } catch {
      // seed_pool_json 损坏时降级到基础字段
    }
  }
  if (!item.stockCode) { return null; }
  return withBaseline({
    stockCode: item.stockCode,
    stockName: item.stockName,
    confidence: item.confidence,
  });
}

/// 从多种可能的 candidates 结构中提取候选数组。
/// 支持的输入形态：
///   - 数组 [...]
///   - { candidates: [...] }
///   - { stocks: [...] } / { list: [...] } / { data: [...] }  常见字段名
///   - Agent 包装 { content: "...", params: { candidates: [...] } }
///   - Agent 包装 { content: "```json\\n{...}\\n```" } （markdown 代码块）
///   - Agent 包装 { content: "{ candidates: [...] }" } （content 是 JSON string）
///   - 任意对象中嵌套的 candidates/stocks 数组（深搜）
function extractCandidatesList(raw: unknown, depth = 0): SerenityCandidate[] {
  // 防御：LLM 可能返回多层 Agent 包装（content 套 content），限制递归深度避免栈溢出
  if (depth > 10) { return []; }
  if (raw == null) { return []; }
  if (Array.isArray(raw)) {
    return raw as SerenityCandidate[];
  }
  if (typeof raw === "object") {
    const obj = raw as Record<string, unknown>;
    // 常见容器字段
    for (const key of ["candidates", "stocks", "list", "data", "items", "results"]) {
      if (Array.isArray(obj[key])) {
        return obj[key] as SerenityCandidate[];
      }
    }
    // Agent 包装：{ params: { candidates: [...] } }
    if (obj.params && typeof obj.params === "object") {
      const params = obj.params as Record<string, unknown>;
      for (const key of ["candidates", "stocks", "list", "data"]) {
        if (Array.isArray(params[key])) {
          return params[key] as SerenityCandidate[];
        }
      }
      // params 整体就是数组
      if (Array.isArray(params)) {
        return params as unknown as SerenityCandidate[];
      }
    }
    // Agent 包装：{ content: "..." }（content 可能是 JSON string 或 markdown 块）
    if (typeof obj.content === "string") {
      const parsed = parseJsonFromContent(obj.content);
      if (parsed) {
        return extractCandidatesList(parsed, depth + 1);
      }
    }
    // 兜底：深度搜索任何属性里的数组，每个元素形如 { stock_code, ... }
    const fallback = findCandidatesDeep(obj);
    if (fallback.length > 0) {
      return fallback;
    }
  }
  return [];
}

/// 从 content 字符串提取 JSON（支持 markdown 代码块、纯 JSON、有无前缀）
function parseJsonFromContent(content: string): unknown | null {
  // 去除前后空白
  const trimmed = content.trim();
  if (!trimmed) { return null; }
  // 1) 直接是 JSON
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      return JSON.parse(trimmed);
    } catch {
      // 不是纯 JSON，继续尝试
    }
  }
  // 2) markdown ```json ... ``` 块
  const codeBlock = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/);
  if (codeBlock) {
    try {
      return JSON.parse(codeBlock[1].trim());
    } catch {
      // fall through
    }
  }
  // 3) 提取第一个 {...} 或 [...] 块
  const firstBrace = trimmed.indexOf("{");
  const firstBracket = trimmed.indexOf("[");
  const start = (() => {
    if (firstBrace === -1) { return firstBracket; }
    if (firstBracket === -1) { return firstBrace; }
    return Math.min(firstBrace, firstBracket);
  })();
  if (start < 0) { return null; }
  const openChar = trimmed[start];
  const closeChar = openChar === "{" ? "}" : "]";
  // 括号配对扫描（处理字符串内的括号）
  let depth = 0;
  let inStr = false;
  let escape = false;
  let end = -1;
  for (let i = start; i < trimmed.length; i++) {
    const c = trimmed[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (c === "\\") {
      escape = true;
      continue;
    }
    if (c === '"') {
      inStr = !inStr;
      continue;
    }
    if (inStr) { continue; }
    if (c === openChar) { depth++; }
    else if (c === closeChar) {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end > start) {
    const candidate = trimmed.slice(start, end + 1);
    try {
      return JSON.parse(candidate);
    } catch {
      return null;
    }
  }
  return null;
}

/// 深度搜索：返回 obj 任意层级第一个看起来像候选数组的数组
function findCandidatesDeep(obj: Record<string, unknown>, depth = 0): SerenityCandidate[] {
  if (depth > 4) { return []; }
  for (const v of Object.values(obj)) {
    if (Array.isArray(v)) {
      // 数组里第一个元素包含 stock_code 或 stock_name 字段就认为命中
      if (v.length > 0 && typeof v[0] === "object" && v[0] !== null) {
        const first = v[0] as Record<string, unknown>;
        if ("stock_code" in first || "stockCode" in first || "stock_name" in first || "stockName" in first) {
          return v as SerenityCandidate[];
        }
      }
    } else if (v && typeof v === "object") {
      const found = findCandidatesDeep(v as Record<string, unknown>, depth + 1);
      if (found.length > 0) { return found; }
    }
  }
  return [];
}

// ── 趋势智选两链的模板 id ──
// 与后端 `commands/stock_workflow/serenity.rs` 的 `SERENITY_TEMPLATE_IDS` 白名单
// 一一对应（两链共用 `run_serenity_screening` 命令，由本参数决定跑哪条）：
//   - `serenity-screening`      ：原链（12 个 Agent 腿，慢但覆盖全）
//   - `serenity-screening-fast` ：快速链（确定性简报 + 单 Agent，见 seed_serenity_fast.rs）
// 类型本身定义在 `serenityStore`（`runningChain` 是 store 状态，两处各写一份会分叉）。

// 链名显示：复用两个运行按钮的文案，不另建 i18n key —— 与 `ScheduledAnalysisTab.tsx`
// 的 `TREND_CHAINS` 同一约定（同一概念同一文案），避免两处链名分叉。
const CHAIN_LABEL_KEY: Record<SerenityChainId, string> = {
  "serenity-screening": "serenityPanel.run",
  "serenity-screening-fast": "serenityPanel.fastRun",
};

// ── 节点 ID → 阶段映射 ──
// 两链**共用本表**（H4：事件 `type` 同为 "serenity-screening"，共用监听器/store）：
//   原链 `serenity-screening`（v53，19 节点）
//   快速链 `serenity-screening-fast`（21 节点，见 seed_serenity_fast.rs）
// 模板每次升版后必须重跑双向 diff（DB nodes[].id 集合 vs 本表 key）：
//   模板有、本表缺 → `?? "loading"` 兜底让阶段文案倒退；本表有、模板无 → 僵尸残留。
// 历史坑：本表曾长期停留在 v4（33 条，含 t-baseline-*/t-signal-*/
// c-bottleneck-trend1~5/s-save-candidates 等已删除节点），执行到 c-scorer-trendN 时
// 因无映射回落 "loading"，阶段文案倒退、进度条语义错乱。
const NODE_STAGE_MAP: Record<string, StepStage> = {
  trigger: "loading",
  // Phase 0: 市场扫描
  "t-industry-rank": "scanning",
  "t-cls-flash": "scanning",
  "t-northbound": "scanning",
  "t-policy-news": "scanning",
  "a-trend-scanner": "scanning",
  // Phase 1: 产业链拆解（a-chain-trendN 与 c-scorer-trendN 交替执行）
  // 注：快速链无 a-chain-trendN（拆解与扫描合并进单个 a-trend-scanner Agent），
  // 由 c-trend-split（Code，把 Agent 输出切成 trend1..5）替代其承上启下位置。
  "a-chain-trend1": "decomposing",
  "a-chain-trend2": "decomposing",
  "a-chain-trend3": "decomposing",
  "a-chain-trend4": "decomposing",
  "a-chain-trend5": "decomposing",
  "c-trend-split": "decomposing",
  // Phase 2: 策略评分 + 一致性检查
  "c-scorer-trend1": "identifying",
  "c-scorer-trend2": "identifying",
  "c-scorer-trend3": "identifying",
  "c-scorer-trend4": "identifying",
  "c-scorer-trend5": "identifying",
  "c-consistency-check": "identifying",
  // Phase 3: 候选公司映射 + 财务数据验证
  // 注：v53 已无 s-save-candidates 节点 —— 候选落库由 run_serenity_screening 尾部
  // 的 Rust 代码完成（不产生节点事件），因此没有节点映射到 "saving" 阶段。
  "a-candidate-mapper": "mapping",
  "c-data-verifier": "mapping",
  // ── 快速链独有节点（原链无这些 id）──
  // 阶段取值遵循**执行顺序单调不回退**：c-scanner-brief / j-* / t-candidate-search /
  // c-candidate-pool 全部在 a-trend-scanner **之前**执行 ⇒ 归 "scanning"；
  // 若按业务语义归 "mapping" 会让阶段在 Agent 启动前先跳到末期、再由
  // a-trend-scanner 退回 "scanning"（正是本表历史坑的同一形态）。
  "c-scanner-brief": "scanning",
  "j-strategy-type": "scanning",
  "j-bottleneck": "scanning",
  "t-candidate-search": "scanning",
  "c-candidate-pool": "scanning",
  "j-candidate-pick": "scanning",
};

// ── 工作流事件监听（模块级单例）──
// 2026-09-11 修复：原实现在 handleRun 内 listen、组件 unmount 时 unlisten，而
// ScreenerPage 的 Tabs 使用 destroyOnHidden（切走即 unmount）→ 切回后没有监听，
// 进度冻结在切走那一帧，且 store.running 粘滞为 true（按钮永久禁用）。
// 现改为模块级注册一次、永不解绑：
//   1) 事件持续写入 store，面板重新挂载后直接读到最新进度/候选/步骤；
//   2) 按 runId 过滤：节点 ID 跨运行恒定且 addStep 按 nodeId upsert，不校验运行
//      归属则残留或并发运行的事件必然串台覆盖当前运行。
let listenersPromise: Promise<void> | null = null;
/** 当前运行 ID：面板发起运行时生成，随 invoke 传给后端，后端原样回灌到事件 payload */
let activeRunId: string | null = null;
/** completed/failed 事件是否已处理（避免 invoke 兜底路径重复设置或错误覆盖） */
let eventHandled = false;

/** 生成一次运行的唯一 ID（WebView2 支持 crypto.randomUUID，降级为时间戳+随机数） */
function newRunId(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c && typeof c.randomUUID === "function") { return c.randomUUID(); }
  return `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

/** 事件是否属于当前运行：后端未回灌 runId 时放行（兼容旧二进制/其它入口） */
function isEventOfActiveRun(payloadRunId: unknown): boolean {
  if (typeof payloadRunId !== "string" || payloadRunId.length === 0) { return true; }
  return activeRunId == null || payloadRunId === activeRunId;
}

/// 幂等注册工作流事件监听（模块级，跨组件挂载周期存活）
function ensureSerenityListeners(): Promise<void> {
  if (!listenersPromise) {
    listenersPromise = (async () => {
      try {
        await listen<{
          runId?: string;
          nodeId: string;
          status: string;
          totalNodes: number;
          completedNodes: number;
          output?: unknown;
          error?: string;
          /**
           * 节点失败的**结构化错误码**（后端收敛到本域值域，见 `stock_workflow/core.rs::node_error_code`）。
           * `null` = 后端明示「本事件无失败」，故判定须用 `typeof errorCode === "string"`。
           */
          errorCode?: string | null;
          elapsedMs?: number;
        }>("serenity-screening-step", (event) => {
          const p = event.payload;
          if (!isEventOfActiveRun(p.runId)) { return; }
          const store = useSerenityStore.getState();
          // 映射缺失时兜底 "loading" 已由 v53 全量映射消除（见 NODE_STAGE_MAP）
          store.setStage(NODE_STAGE_MAP[p.nodeId] ?? "loading");
          store.setTotalNodes(p.totalNodes ?? 0);
          store.setCompletedNodes(p.completedNodes ?? 0);
          store.setCurrentNode(p.nodeId);
          store.addStep({
            nodeId: p.nodeId,
            status: p.status,
            output: p.output,
            error: p.error,
            errorCode: p.errorCode,
            elapsedMs: p.elapsedMs,
            totalNodes: p.totalNodes,
            completedNodes: p.completedNodes,
            timestamp: Date.now(),
          });
        });

        await listen<{
          runId?: string;
          status: string;
          result?: unknown;
          candidates?: unknown[];
          trends?: TrendInfo[];
          error?: string;
          /**
           * 工作流级失败的**结构化码**（后端 `stock_workflow/core.rs::workflow_error_code`
           * 映射 `WorkflowError` 全部 10 个变体）。`undefined` = 旧载荷（无码）⇒ 展示回退 `error` 原文。
           */
          code?: string | null;
          emptyReason?: string | null;
          /**
           * 落库部分失败（`partial_failure`）的**结构化码**：`stock_workflow::PERSIST_FAILED`
           * （`error_code.rs`）。与 `code` 一样，`null` = 后端明示无失败，`undefined` = 旧载荷。
           */
          persistenceCode?: string | null;
          /** 首个写入失败的股票代码 —— 作为详情行的 `params`（主文案不含它）。 */
          persistenceStockCode?: string | null;
          /**
           * 落库失败的**技术详情**（DB 报错原文，后端已去掉中文前缀）。
           * ⚠ 旧载荷里这里是 `"写入 300567 失败: <db err>"` 整句 —— 故渲染层
           * **必须**先看 `persistenceCode` 是否存在，再决定要不要补股票代码行，
           * 否则旧载荷下会把代码重复渲染两遍。
           */
          persistenceError?: string | null;
        }>("serenity-screening-completed", (event) => {
          const p = event.payload;
          if (!isEventOfActiveRun(p.runId)) { return; }
          const store = useSerenityStore.getState();
          eventHandled = true;
          if (p.status === "failed") {
            // 本地化：有码取 11 语言译文，无码（旧载荷）回退原文。
            // `p.error` 形如 `"Serenity 筛选工作流失败: <WorkflowError::Display>"`，其中可能含中文
            // ——`WorkflowError` 的 `InvalidStateTransition` / `LifecycleHookFailed` 两个变体自带中文。
            const failureText = translateFailureText(p.error, p.code)
              || i18n.t("serenityPanel.errorUnknown");
            store.setError(failureText);
            store.setStage("error");
            store.setRunning(false);
            store.setCurrentNode(null);
            return;
          }
          // completed 与 partial_failure 都必须收尾：
          // partial_failure = 候选已产出但部分落库失败（serenity.rs persistence_success=false），
          // 旧实现只认 completed/failed → 该分支既不收尾也不清 running（面板卡在运行中）。
          if (p.status === "completed" || p.status === "partial_failure") {
            if (p.status === "partial_failure") {
              // 日志打结构化字段（码由 payload 承载）——此前打的是中文整句，非中文环境查日志同样难读。
              console.warn(
                "[Serenity] 持久化部分失败：",
                p.persistenceCode ?? "(no code)",
                p.persistenceStockCode,
                p.persistenceError,
              );
            }
            const directCandidates = Array.isArray(p.candidates)
              ? (p.candidates.filter((c: unknown) => c != null) as SerenityCandidate[])
              : null;
            const list = directCandidates && directCandidates.length > 0
              ? directCandidates
              : extractCandidatesList(p.result);
            if (list.length > 0) {
              store.setCandidates(list);
            } else {
              console.warn(
                "[Serenity] ⚠️ No candidates could be extracted! Full payload:",
                JSON.stringify(p).slice(0, 1000),
              );
            }
            if (Array.isArray(p.trends)) {
              store.setTrends(p.trends);
            }
            if (typeof p.emptyReason === "string" && p.emptyReason.trim().length > 0) {
              store.setEmptyReason(p.emptyReason.trim());
            }
            // 部分落库失败时把原因暴露给用户（候选仍正常展示，仅提示写入异常）。
            // 双层：主文案走**结构化码**取 11 语言译文（`translateFailureText`），
            // 技术详情（哪只 + DB 原文）另起一行保留 —— 本地化不以「丢原因」为代价。
            if (p.status === "partial_failure" && typeof p.persistenceError === "string") {
              const persistCode = typeof p.persistenceCode === "string" ? p.persistenceCode : null;
              // 有码 ⇒ `persistenceError` 已是纯 DB 原文，需补「哪只」才有排查价值；
              // 无码（旧载荷）⇒ 原文整句里本就带股票代码，再拼一次会把代码重复渲染两遍。
              const persistDetail = persistCode
                ? [p.persistenceStockCode, p.persistenceError]
                  .filter((x): x is string => typeof x === "string" && x.length > 0)
                  .join(": ")
                : null;
              store.setError(
                translateFailureText(p.persistenceError, persistCode),
                persistDetail || null,
              );
            }
            store.setStage("done");
            store.setRunning(false);
            store.setCurrentNode(null);
          }
        });
      } catch {
        // 非 Tauri 环境（浏览器 mock）listen 不可用：置空以允许下次重试
        listenersPromise = null;
      }
    })();
  }
  return listenersPromise;
}

/// 将节点 ID 映射为 i18n 标题 key
function nodeTitleKey(nodeId: string): string {
  return `serenityPanel.nodeTitles.${nodeId}`;
}

// ═══ 节点输出语义化渲染 ═══
// 引擎 executor 输出形态（已实锤）：
//   ToolNode  → { tool_name, result(JSON字符串), truncated, is_error, node_id }
//   CodeNode  → { status:"executed", language, result(JSON), params, input_params, node_id }
//   AgentNode → { content, thinking, tool_calls, usage, iterations, stopped_by_limit }

/// 常见数据字段 → 中文列名（数据 schema 翻译，非 UI 文案，不走 i18n）
const FIELD_LABEL_MAP: Record<string, string> = {
  // 通用
  stock_code: "serenityPanel.colStockCode",
  stockCode: "serenityPanel.colStockCode",
  stock_name: "serenityPanel.colName",
  stockName: "serenityPanel.colName",
  name: "serenityPanel.colName",
  code: "serenityPanel.colCode",
  price: "serenityPanel.colPrice",
  change_pct: "serenityPanel.colChangePct",
  changePct: "serenityPanel.colChangePct",
  pct_chg: "serenityPanel.colChangePct",
  status: "serenityPanel.colStatus",
  result: "serenityPanel.colResult",
  params: "serenityPanel.colParams",
  // 行业排名
  industry: "serenityPanel.colIndustry",
  industry_name: "serenityPanel.colIndustry",
  industryName: "serenityPanel.colIndustry",
  rank: "serenityPanel.colRank",
  change_pct_3m: "serenityPanel.colChangePct3m",
  changePct3m: "serenityPanel.colChangePct3m",
  change_pct_1m: "serenityPanel.colChangePct1m",
  changePct1m: "serenityPanel.colChangePct1m",
  leading_stocks: "serenityPanel.colLeadingStocks",
  leadingStocks: "serenityPanel.colLeadingStocks",
  turnover: "serenityPanel.colTurnover",
  volume: "serenityPanel.colVolume",
  avg_price: "serenityPanel.colAvgPrice",
  // 北向资金
  northbound_hold: "serenityPanel.colNorthboundHold",
  northboundHold: "serenityPanel.colNorthboundHold",
  hold_value: "serenityPanel.colHoldValue",
  holdValue: "serenityPanel.colHoldValue",
  // 趋势/信号
  trend_name: "serenityPanel.colTrend",
  trendName: "serenityPanel.colTrend",
  confidence: "serenityPanel.colConfidence",
  score: "serenityPanel.colScore",
  total_score: "serenityPanel.colTotalScore",
  totalScore: "serenityPanel.colTotalScore",
  // 财报
  revenue: "serenityPanel.colRevenue",
  net_profit: "serenityPanel.colNetProfit",
  netProfit: "serenityPanel.colNetProfit",
  gross_margin: "serenityPanel.colGrossMargin",
  grossMargin: "serenityPanel.colGrossMargin",
  pe: "PE",
  pb: "PB",
  roe: "ROE",
};

/** 节点输出分析结果：语义化渲染所需的最小结构化数据 */
interface NodeOutputView {
  kind: "tool" | "code" | "agent" | "json" | "text" | "empty";
  /** 数组数据（表格渲染） */
  table?: { columns: string[]; rows: Array<Record<string, unknown>> };
  /** 展开态完整展示文本（JSON 美化或原文） */
  jsonText: string;
  /** 文本预览（agent 节点取 content 摘要） */
  textPreview?: string;
  /** 数组条数（折叠态摘要用） */
  count?: number;
  /** 对象有值字段数（折叠态摘要用） */
  fieldCount?: number;
}

/** 宽松解析：字符串 → JSON；非 JSON 字符串或解析失败返回原文 */
function looseParse(v: unknown): unknown {
  if (typeof v !== "string") { return v; }
  const t = v.trim();
  if (!t.startsWith("{") && !t.startsWith("[")) { return v; }
  try {
    return JSON.parse(t);
  } catch {
    return v;
  }
}

/** 提取数组行的列：保持首元素字段顺序，补充后续行的新字段 */
function collectColumns(rows: Array<Record<string, unknown>>): string[] {
  const cols: string[] = [];
  const seen = new Set<string>();
  for (const r of rows) {
    for (const k of Object.keys(r)) {
      if (!seen.has(k)) {
        seen.add(k);
        cols.push(k);
      }
    }
  }
  return cols;
}

/** 单元格清洗：null → "—"；number → 2 位小数；对象/数组 → 精简 JSON */
function cellText(v: unknown, translate?: (key: string) => string): string {
  if (v == null) { return "—"; }
  if (typeof v === "number") {
    return Number.isFinite(v) ? (Math.round(v * 100) / 100).toString() : String(v);
  }
  if (typeof v === "boolean") {
    return translate
      ? (v ? translate("serenityPanel.boolYes") : translate("serenityPanel.boolNo"))
      : (v ? "Yes" : "No");
  }
  if (typeof v === "string") { return v.length > 60 ? v.slice(0, 60) + "…" : v; }
  const s = JSON.stringify(v);
  return s && s.length > 60 ? s.slice(0, 60) + "…" : (s ?? "—");
}

/** 分析节点输出 → 语义化视图（纯函数，不含 i18n 文案，文案在渲染处拼装） */
function buildNodeOutputView(_nodeId: string, output: unknown): NodeOutputView {
  const empty: NodeOutputView = { kind: "empty", jsonText: "" };
  if (output == null) { return empty; }
  if (typeof output === "string" && output.trim().length === 0) { return empty; }

  // 1. 按包装结构解包 → payload + 节点类型
  let kind: NodeOutputView["kind"] = "text";
  let payload: unknown = output;
  if (typeof output === "object" && !Array.isArray(output)) {
    const o = output as Record<string, unknown>;
    if (typeof o.tool_name === "string" && "result" in o) {
      // ToolNode：result 可能是 ToolResult 形态 {content, truncated, is_error, ...}，
      // content 才是工具数据（JSON 字符串）。再解一层。
      kind = "tool";
      let r = o.result;
      if (r && typeof r === "object" && !Array.isArray(r)) {
        const rr = r as Record<string, unknown>;
        if (typeof rr.content === "string") { r = rr.content; }
      }
      payload = looseParse(r);
    } else if (o.status === "executed" && "result" in o) {
      // CodeNode：result 为 Rhai 脚本返回值
      kind = "code";
      payload = looseParse(o.result);
    } else if (typeof o.content === "string") {
      // AgentNode：content 为 LLM 输出（可能为 JSON 字符串）
      kind = "agent";
      payload = looseParse(o.content);
    } else if ("result" in o) {
      payload = looseParse(o.result);
    }
  } else if (typeof output === "string") {
    const parsed = looseParse(output);
    if (parsed !== output) {
      kind = "json";
      payload = parsed;
    }
  }

  // 1.5 解包后为 null（如北向资金返回 "null"）→ 空数据语义
  if (payload === null) {
    return { kind, jsonText: "null", count: 0 };
  }

  // 2. 数组 → 表格（元素为对象时）
  if (Array.isArray(payload)) {
    const rows = payload.filter(
      (r): r is Record<string, unknown> => !!r && typeof r === "object" && !Array.isArray(r),
    );
    if (rows.length > 0) {
      return {
        kind,
        table: { columns: collectColumns(rows), rows },
        jsonText: JSON.stringify(payload, null, 2),
        count: payload.length,
      };
    }
    return { kind, jsonText: JSON.stringify(payload, null, 2), count: payload.length };
  }

  // 3. 对象 → 键值/JSON 展示
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>;
    const keys = Object.keys(obj).filter((k) => {
      const v = obj[k];
      return v != null && !(typeof v === "string" && v.trim().length === 0);
    });
    // Agent 节点常输出 { summary, trends, ... }：摘要直接展示 summary 结论文本
    let preview: string | undefined;
    if (typeof obj.summary === "string" && obj.summary.trim().length > 0) {
      preview = obj.summary.trim();
    }
    return {
      kind,
      jsonText: JSON.stringify(payload, null, 2),
      fieldCount: keys.length,
      textPreview: preview,
    };
  }

  // 4. 纯文本
  const text = String(payload).trim();
  if (text.length === 0) { return empty; }
  return { kind, jsonText: text, textPreview: text };
}

/** 截断辅助 */
function truncateText(s: string, n: number): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}

export function SerenityScreeningPanel() {
  const { message: messageApi } = App.useApp();
  const {
    running,
    setRunning,
    candidates,
    setCandidates,
    trends,
    setTrends,
    error,
    errorDetail,
    setError,
    stage,
    setStage,
    completedNodes,
    setCompletedNodes,
    totalNodes,
    setTotalNodes,
    steps,
    currentNodeId,
    setCurrentNode,
    clearSteps,
    emptyReason,
    setEmptyReason,
    runningChain,
    setRunningChain,
  } = useSerenityStore();
  const { t } = useTranslation();
  // 跳转统一走 useStockJump（与智能荐股 / 筛选结果同一条链，避免参数名分叉）
  const jumpToStock = useStockJump();

  // 事件监听已上移到模块级（ensureSerenityListeners）：跨 tab 切换存活，
  // 不再使用组件内 ref 保存 unlisten，也不再随 unmount 解绑。
  const [expandedSteps, setExpandedSteps] = useState<Set<number>>(new Set());
  // 本次运行跑的是哪条链（null = 未运行/未知）。存在 store 里而非组件 state：
  // ScreenerPage 的 Tabs 是 destroyOnHidden，切走即 unmount，组件 state 会丢而
  // store.running 仍为 true ⇒ 两按钮都 disabled、都不转圈，进度卡也说不清是哪条链。
  // 两个运行按钮各自据此显示 loading，避免点快速链却让原链按钮也在转（running 由
  // store 统管、无法区分来源）。
  // 回馈闭环状态
  const [feedbackData, setFeedbackData] = useState<
    {
      total: number;
      profitable_count: number;
      win_rate: number;
      avg_return_pct: number;
      performances: Array<{
        id: string;
        stock_code: string;
        stock_name: string;
        return_pct: number;
        is_profitable: boolean;
        recommend_date: string;
        catalysts: { total: number; verified: number };
      }>;
    } | null
  >(null);
  const [feedbackLoading, setFeedbackLoading] = useState(false);

  // 挂载时恢复上次运行候选的加载状态（避免 Empty 闪烁）
  const [lastRunLoading, setLastRunLoading] = useState(false);

  // 瓶颈掘金历史（多选 + 批量删除）
  const [serenityHistoryOpen, setSerenityHistoryOpen] = useState(false);
  const [serenityHistory, setSerenityHistory] = useState<
    Array<{
      generatedAt: string;
      stockCount: number;
      createdAt: string;
      // 该条记录实际包含的风格（后端 GROUP_CONCAT DISTINCT style），
      // 详情查询用它对齐列表口径，避免列表认两种风格而详情只认一种导致数量对不上
      styles: string;
    }>
  >([]);
  const [serenityHistoryLoading, setSerenityHistoryLoading] = useState(false);
  const [serenitySelected, setSerenitySelected] = useState<string[]>([]);
  const [serenityDeleting, setSerenityDeleting] = useState(false);

  // 瓶颈掘金历史详情
  const [serenityDetailOpen, setSerenityDetailOpen] = useState(false);
  const [serenityDetailLoading, setSerenityDetailLoading] = useState(false);
  const [serenityDetailItems, setSerenityDetailItems] = useState<
    Array<{
      stockCode: string;
      stockName: string;
      confidence: number;
      generatedAt: string;
    }>
  >([]);
  const [serenityDetailRow, setSerenityDetailRow] = useState<
    {
      generatedAt: string;
      stockCount: number;
      createdAt: string;
      styles: string;
    } | null
  >(null);

  // ── 主题输入（对话式主题荐股 v47）──
  const [themeTags, setThemeTags] = useState<string[]>([]);

  // ── 估值过滤设置 ──
  const [serenitySettingsOpen, setSerenitySettingsOpen] = useState(false);
  const [serenityVars, setSerenityVars] = useState<Record<string, number>>({});
  // 2026-09-11 修复：原实现写死 get_template_by_version({ version: 6 }) —— 读的是
  // 历史快照（该函数按版本精确匹配），而运行时执行与写入（apply_update_variable）
  // 都走主表当前版本 → 改完设置关闭再打开会「回弹」旧值（快照里还留着已删除的
  // ref_*_code 变量）。改为按 id 读主表（当前版本），与写侧同源。
  // 读侧仍以原链模板为**权威源**（两链的变量语义完全相同，设置面板是共享设置）。
  useEffect(() => {
    invoke<{ variables: Array<{ name: string; value: unknown }> }>(
      "get_workflow_template",
      { id: "serenity-screening" },
    ).then((tpl) => {
      if (!tpl) { return; }
      const map: Record<string, number> = {};
      for (const v of tpl.variables) {
        if (v.name.startsWith("serenity_")) {
          map[v.name.replace("serenity_", "")] = Number(v.value) || 0;
        }
      }
      setSerenityVars(map);
    }).catch(() => {});
  }, []);
  // 写侧必须**双写两链**：H2 只约束脚本单源，而变量是「用户共享设置」。
  // 快速链的模板行在建链时复制了一份默认值（seed_serenity_fast.rs 复用
  // build_serenity_variables），若只写原链，用户在设置面板改完过滤参数后，
  // 快速链会静默沿用种子时的旧默认值，两链结果不一致且无任何提示。
  // 用 allSettled 语义逐项 catch：另一链尚未种子（首次启动竞态）时只丢该项，
  // 不影响原链写入 —— 原链是权威源，绝不能被快速链的失败带崩。
  const handleSerenityVarChange = useCallback(async (key: string, value: number) => {
    setSerenityVars((prev) => ({ ...prev, [key]: value }));
    const chainIds: SerenityChainId[] = ["serenity-screening", "serenity-screening-fast"];
    await Promise.all(
      chainIds.map((templateId) =>
        invoke("apply_update_variable", {
          templateId,
          name: `serenity_${key}`,
          value,
        }).catch(() => {
          /* ignore：另一链未种子时该项失败即可 */
        })
      ),
    );
  }, []);

  // 挂载即确保事件监听已注册（模块级单例，注册后永不解绑）。
  // 不放 handleRun 内：监听随组件卸载解绑正是「切 tab 后进度冻结 + running 粘滞」
  // 的根因；模块级注册后，重新挂载的面板可直接读到 store 中的最新进度。
  useEffect(() => {
    void ensureSerenityListeners();
  }, []);

  // ── 挂载时恢复最近一次工作流运行产生的候选 ──
  // tab 打开（destroyOnHidden 下每次切换都会重新 mount）默认展示上一次
  // 趋势智选产物。
  // ⚠ 2026-09-18 修复：恢复查询**优先只认 style='serenity'**（serenity-screening
  // 工作流产物，seed_pool_json 是完整候选对象，含 serenity_score/催化剂/风险等）；
  // 仅当历史中完全没有 serenity 记录时，才回退到 style='bottleneck'
  // （智能荐股内置 SerenityStrategy 产物，业务上也属"趋势智选"）。
  // 此前直接认 "serenity,bottleneck"：若最近一次是智能荐股，其 seed_pool_json
  // 是推荐池快照（数组），restoreCandidate 走 fallback 只剩
  // {stockCode, stockName, confidence} —— 趋势智选卡片评分恒为 0、
  // 催化剂/风险/退出信号/关注度全部缺失（格式与信息均不正确）。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      // 若工作流正在运行，不打扰运行态（completed 事件会覆盖结果）
      if (useSerenityStore.getState().running) { return; }
      setLastRunLoading(true);
      try {
        // 先查最近一次真正的趋势智选（serenity）记录
        let list = await invoke<Array<{ generatedAt: string; stockCount: number; createdAt: string }>>(
          "list_reco_history",
          { styleFilter: "serenity", limit: 1 },
        );
        let restoreStyle = "serenity";
        // 无 serenity 历史 → 回退到智能荐股 SerenityStrategy（bottleneck）记录
        if (!list || list.length === 0) {
          list = await invoke<Array<{ generatedAt: string; stockCount: number; createdAt: string }>>(
            "list_reco_history",
            { styleFilter: "bottleneck", limit: 1 },
          );
          restoreStyle = "bottleneck";
        }
        if (cancelled || !list || list.length === 0) { return; }
        const detail = await invoke<RecoDetailItem[]>("get_reco_detail", {
          generatedAt: list[0].generatedAt,
          styleFilter: restoreStyle,
        });
        if (cancelled) { return; }
        const restored = (detail ?? [])
          .map(restoreCandidate)
          .filter((c): c is SerenityCandidate => c != null);
        if (restored.length > 0) {
          setCandidates(restored);
        }
      } catch (e) {
        // 历史为空/查询失败不阻塞面板，保持默认空状态
        console.error("[Serenity] Failed to load last run candidates", e);
      } finally {
        if (!cancelled) { setLastRunLoading(false); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [setCandidates]);

  const toggleStep = useCallback((idx: number) => {
    setExpandedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) {
        next.delete(idx);
      } else {
        next.add(idx);
      }
      return next;
    });
  }, []);

  const handleRun = useCallback(async (chainId: SerenityChainId) => {
    // 清理上一次结果
    clearSteps();
    setCandidates([]);
    setTrends([]);
    setError(null);
    setEmptyReason(null);
    setStage("loading");
    setCompletedNodes(0);
    setTotalNodes(0);
    setExpandedSteps(new Set());
    setRunningChain(chainId);

    // 事件监听是模块级单例（ensureSerenityListeners）：这里只确保已注册，
    // 并在启动前锁定本次运行的 runId —— 监听器据此丢弃其它运行的事件。
    // 必须先 await 再 invoke，避免漏掉早期节点事件。
    activeRunId = newRunId();
    eventHandled = false;
    await ensureSerenityListeners();

    setRunning(true);
    try {
      // 读取时间旅行上下文
      const anchorState = useTimeAnchorStore.getState();
      const asOfDate = anchorState.mode === "replay" || anchorState.mode === "backtest_sweep"
        ? anchorState.asOfDate
        : null;
      // invoke 作为兜底：如果 completed 事件已设置结果，这里的重复 set 是无害的；
      // 如果事件未到达（如非 Tauri 环境），invoke 返回值是唯一来源。
      // Serenity 筛选涉及多个 LLM 调用，超时时间设为 30 分钟
      const SERENITY_TIMEOUT_MS = 30 * 60 * 1000;
      const r = await invoke<SerenityResult>(
        "run_serenity_screening",
        {
          asOfDate,
          themes: themeTags.length > 0 ? themeTags : null,
          // 运行 ID：后端把它原样回灌到 step/completed 事件 payload，
          // 前端监听器据此过滤掉其它运行（并发/残留）的事件，避免串台。
          runId: activeRunId,
          // 跑哪条链：由按钮决定（原链 / 快速链），后端按白名单校验后
          // 决定 `load_and_inject_template` 读哪个模板行。
          templateId: chainId,
        },
        SERENITY_TIMEOUT_MS,
      );
      // 如果事件已经处理过（覆盖了 candidates/trends），这里不要重复 set
      // 但仍要确保 running 状态被关闭
      if (!eventHandled) {
        const list = extractCandidatesList(r?.candidates);
        if (list.length > 0) {
          setCandidates(list);
        }
        if (Array.isArray(r?.trends) && r.trends.length > 0) {
          setTrends(r.trends);
        }
        if (typeof r?.emptyReason === "string" && r.emptyReason.trim().length > 0) {
          setEmptyReason(r.emptyReason.trim());
        }
        setStage("done");
      }
    } catch (err: unknown) {
      // 仅在 completed 事件未已经处理时才显示错误
      if (!eventHandled) {
        // 超时错误特殊处理：如果后端仍在运行，给用户友好提示
        if (err instanceof InvokeTimeoutError) {
          console.warn(
            `[Serenity] invoke timed out (${
              (err.timeoutMs / 1000).toFixed(0)
            }s), backend workflow may still be running...`,
          );
          setError(
            t("serenityPanel.timeoutHint", {
              seconds: (err.timeoutMs / 1000).toFixed(0),
            }),
          );
        } else {
          setError(err instanceof Error ? err.message : String(err));
        }
        setStage("error");
      }
    } finally {
      setRunning(false);
      setCurrentNode(null);
      setRunningChain(null);
    }
  }, [
    clearSteps,
    setCandidates,
    setEmptyReason,
    setError,
    setRunning,
    setStage,
    setTrends,
    setCompletedNodes,
    setTotalNodes,
    setCurrentNode,
    themeTags,
    t,
  ]);

  /** 打开瓶颈掘金历史详情 */
  const openSerenityDetail = useCallback(
    async (row: { generatedAt: string; stockCount: number; createdAt: string; styles: string }) => {
      setSerenityDetailRow(row);
      setSerenityDetailOpen(true);
      setSerenityDetailLoading(true);
      try {
        // ⚠ 2026-09-18 修复：详情查询用该记录自身的 styles（后端 list 返回的
        // GROUP_CONCAT DISTINCT style），与列表口径一致。此前硬编码 "serenity"：
        // 列表用 "serenity,bottleneck" 展示合并数量（如 10），详情却只查 serenity，
        // 若该轮候选来自智能荐股（bottleneck）则详情恒空，显示"无候选股票数据"。
        const styleFilter = row.styles || "serenity";
        const items = await invoke<
          Array<{ stockCode: string; stockName: string; confidence: number; generatedAt: string }>
        >("get_reco_detail", {
          generatedAt: row.generatedAt,
          styleFilter,
        });
        setSerenityDetailItems(items ?? []);
      } catch (e) {
        console.error("Failed to load bottleneck detail", e);
        setSerenityDetailItems([]);
      }
      setSerenityDetailLoading(false);
    },
    [],
  );

  /** 删除单条历史记录（复用 batch_delete_reco_history，传单元素数组） */
  const handleDeleteOne = useCallback(
    async (row: { generatedAt: string; stockCount: number; createdAt: string; styles: string }) => {
      setSerenityDeleting(true);
      try {
        await invoke("batch_delete_reco_history", { generatedAts: [row.generatedAt] });
        messageApi.success(
          t("serenityPanel.serenityHistory.deleteSuccess", { count: 1 }),
        );
        // 若当前详情正是被删这条，同步关闭
        if (serenityDetailRow?.generatedAt === row.generatedAt) {
          setSerenityDetailOpen(false);
          setSerenityDetailItems([]);
          setSerenityDetailRow(null);
        }
        setSerenityHistory((prev) => prev.filter((r) => r.generatedAt !== row.generatedAt));
        setSerenitySelected((prev) => prev.filter((g) => g !== row.generatedAt));
      } catch (e) {
        showBackendError(messageApi, e);
      } finally {
        setSerenityDeleting(false);
      }
    },
    [serenityDetailRow, messageApi, t],
  );

  // 当前阶段文案
  const stageLabel = (() => {
    if (!running && stage === "done") { return t("serenityPanel.stage_done"); }
    if (!running && stage === "error") { return t("serenityPanel.stage_error"); }
    switch (stage) {
      case "loading":
        return t("serenityPanel.stage_loading");
      case "scanning":
        return t("serenityPanel.stage_scanning");
      case "decomposing":
        return t("serenityPanel.stage_decomposing");
      case "identifying":
        return t("serenityPanel.stage_identifying");
      case "mapping":
        return t("serenityPanel.stage_mapping");
      case "saving":
        return t("serenityPanel.stage_saving");
      default:
        return t("serenityPanel.running");
    }
  })();

  const progressPct = totalNodes > 0 ? Math.round((completedNodes / totalNodes) * 100) : 0;

  return (
    <div className="flex flex-col gap-3">
      {/* v47: 主题输入区（对话式主题荐股） */}
      <div className="flex items-center gap-2">
        <Text type="secondary" className="text-xs whitespace-nowrap">
          {t("serenityPanel.themeInput")}
        </Text>
        <Select
          mode="tags"
          style={{ flex: 1 }}
          placeholder={t("serenityPanel.themePlaceholder")}
          value={themeTags}
          onChange={setThemeTags as (val: string[]) => void}
          disabled={running}
          tokenSeparators={[",", "，"]}
          open={false}
        />
        {themeTags.length > 0 && (
          <Tag color="blue">
            {t("serenityPanel.sourceUser")}: {themeTags.join(", ")}
          </Tag>
        )}
      </div>

      {/* 操作栏 */}
      <div className="flex items-center justify-between">
        <Text type="secondary" className="text-xs">
          {t("serenityPanel.desc")}
        </Text>
        <div className="flex items-center gap-2">
          <Button
            size="small"
            icon={<HistoryOutlined />}
            onClick={async () => {
              setSerenityHistoryOpen(true);
              setSerenityHistoryLoading(true);
              try {
                // 同时认 'serenity'（serenity-screening 工作流）和 'bottleneck'
                // （智能荐股内置 SerenityStrategy）——业务上都是"趋势智选"。
                const list = await invoke<typeof serenityHistory>("list_reco_history", {
                  styleFilter: "serenity,bottleneck",
                  limit: 50,
                });
                console.log("[SerenityHistory] list_reco_history returned:", list?.length, list);
                setSerenityHistory(list ?? []);
                if (!list || list.length === 0) {
                  messageApi.warning(
                    t("serenityPanel.emptyHistoryWarning"),
                  );
                }
              } catch (e) {
                console.error("[SerenityHistory] list_reco_history call failed", e);
                messageApi.error(t("serenityPanel.backendCallFailed", { error: String(e) }));
              } finally {
                setSerenityHistoryLoading(false);
              }
            }}
          >
            {t("serenityPanel.serenityHistory.viewHistory")}
          </Button>
          <Button
            size="small"
            icon={<AlertOutlined />}
            loading={feedbackLoading}
            onClick={async () => {
              setFeedbackLoading(true);
              try {
                const anchorState = useTimeAnchorStore.getState();
                const asOfDate = anchorState.mode === "replay"
                  ? anchorState.asOfDate
                  : null;
                const r = await invoke<typeof feedbackData>(
                  "refresh_serenity_feedback",
                  { asOfDate },
                );
                setFeedbackData(r);
              } catch (e) {
                console.error("Feedback loop analysis failed", e);
              } finally {
                setFeedbackLoading(false);
              }
            }}
          >
            {t("serenityPanel.feedbackButton")}
          </Button>
          <Button
            type="primary"
            icon={runningChain === "serenity-screening"
              ? <ReloadOutlined spin />
              : <PlayCircleOutlined />}
            loading={runningChain === "serenity-screening"}
            disabled={running && runningChain !== "serenity-screening"}
            onClick={() => handleRun("serenity-screening")}
          >
            {runningChain === "serenity-screening"
              ? t("serenityPanel.running")
              : t("serenityPanel.run")}
          </Button>
          <Button
            type="primary"
            ghost
            icon={runningChain === "serenity-screening-fast"
              ? <ReloadOutlined spin />
              : <ThunderboltOutlined />}
            loading={runningChain === "serenity-screening-fast"}
            disabled={running && runningChain !== "serenity-screening-fast"}
            title={t("serenityPanel.fastRunTip")}
            onClick={() => handleRun("serenity-screening-fast")}
          >
            {runningChain === "serenity-screening-fast"
              ? t("serenityPanel.running")
              : t("serenityPanel.fastRun")}
          </Button>
        </div>
      </div>

      {/* 过滤参数设置 */}
      <Card
        size="small"
        className="w-full"
        title={
          <div
            className="flex items-center gap-2 cursor-pointer text-sm"
            onClick={() => setSerenitySettingsOpen(!serenitySettingsOpen)}
          >
            <span>{serenitySettingsOpen ? "▼" : "▶"} {t("serenityPanel.settings")}</span>
          </div>
        }
      >
        {serenitySettingsOpen && (
          <div className="grid grid-cols-2 gap-3">
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filterPeUpper")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={1000}
                value={serenityVars.max_pe ?? 100}
                onChange={(v) => handleSerenityVarChange("max_pe", v ?? 100)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixMultiplier")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filterPbUpper")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={100}
                value={serenityVars.max_pb ?? 10}
                onChange={(v) => handleSerenityVarChange("max_pb", v ?? 10)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixMultiplier")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filter3mGainUpper")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={500}
                value={serenityVars.max_3m_gain_pct ?? 30}
                onChange={(v) => handleSerenityVarChange("max_3m_gain_pct", v ?? 30)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixPercent")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filter12mGainUpper")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={500}
                value={serenityVars.max_12m_gain_pct ?? 100}
                onChange={(v) => handleSerenityVarChange("max_12m_gain_pct", v ?? 100)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixPercent")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filterGrossMarginLower")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={100}
                value={serenityVars.min_gross_margin ?? 25}
                onChange={(v) => handleSerenityVarChange("min_gross_margin", v ?? 25)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixPercent")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filterDebtRatioUpper")}</Text>
              <InputNumber
                size="small"
                min={0}
                max={100}
                value={serenityVars.max_debt_ratio ?? 60}
                onChange={(v) => handleSerenityVarChange("max_debt_ratio", v ?? 60)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixPercent")}
              />
            </div>
            <div className="flex items-center justify-between text-xs">
              <Text type="secondary">{t("serenityPanel.filterGrowthExemptThreshold")}</Text>
              <InputNumber
                size="small"
                min={10}
                max={200}
                value={serenityVars.growth_exempt_pct ?? 50}
                onChange={(v) => handleSerenityVarChange("growth_exempt_pct", v ?? 50)}
                className="w-24"
                suffix={t("serenityPanel.filterSuffixPercent")}
              />
            </div>
            <div className="col-span-2 text-xs text-gray-400 mt-1">
              {t("serenityPanel.filterGrowthExemptHint")}
            </div>
          </div>
        )}
      </Card>

      {/* 进度状态 */}
      {running && (
        <Card size="small" className="w-full">
          <div className="flex flex-col gap-2">
            <div className="flex items-center gap-2 text-sm">
              <Spin indicator={<LoadingOutlined spin />} size="small" />
              {runningChain && (
                <Tag color="blue" className="text-xs shrink-0">
                  {t(CHAIN_LABEL_KEY[runningChain])}
                </Tag>
              )}
              <span className="font-medium">{stageLabel}</span>
              {currentNodeId && (
                <Text type="secondary" className="text-xs">
                  {t(nodeTitleKey(currentNodeId))}
                </Text>
              )}
            </div>
            {totalNodes > 0 && (
              <div className="flex items-center gap-2">
                <Progress
                  percent={progressPct}
                  size="small"
                  className="flex-1"
                  format={() => `${completedNodes}/${totalNodes}`}
                />
              </div>
            )}
          </div>
        </Card>
      )}

      {/* 执行日志 */}
      {steps.length > 0 && (
        <Card
          size="small"
          title={
            <div className="flex items-center gap-2 text-sm">
              <ClockCircleOutlined />
              <span>{t("serenityPanel.stepLogTitle")}</span>
              {runningChain && (
                <Tag color="blue" className="text-xs">
                  {t(CHAIN_LABEL_KEY[runningChain])}
                </Tag>
              )}
              <Tag className="text-xs">{steps.length}</Tag>
            </div>
          }
          className="w-full"
          styles={{ body: { maxHeight: 360, overflowY: "auto" } }}
        >
          <Space orientation="vertical" className="w-full" size={4}>
            {steps.map((s, i) => {
              const isExpanded = expandedSteps.has(i);
              // timeout 与 failed 同属失败类（此前 timeout 被渲染成蓝色 loading 图标）
              const isFailed = s.status === "failed" || s.status === "timeout";
              const statusColor = s.status === "completed"
                ? "green"
                : isFailed
                ? "red"
                : "blue";
              const statusIcon = s.status === "completed"
                ? <CheckCircleOutlined style={{ color: "#52c41a" }} />
                : isFailed
                ? <span style={{ color: "#ff4d4f" }}>✕</span>
                : <LoadingOutlined style={{ color: "#1677ff" }} />;
              // 节点输出语义化分析（ToolNode 表格 / CodeNode 计算 / AgentNode 文本）
              const view = buildNodeOutputView(s.nodeId, s.output);
              // 失败节点的展示文案。`s.error` 是后端 `NodeError::Display` 的**自由文本**
              // （可能含中文，如 "EXECUTION_CANCELLED: 节点执行已取消"）⇒ 只作技术详情；
              // 主文案优先用 `errorCode` 取 11 语言译文，无码/未收录时自动回退原文。
              const failureText = isFailed ? translateFailureText(s.error, s.errorCode) : "";
              // 折叠态单行摘要：失败 → 错误信息；成功 → "类型 · 数据规模"
              let summary = "";
              if (isFailed) {
                summary = failureText ? truncateText(failureText, 60) : "";
              } else if (s.status === "completed" && view.kind !== "empty") {
                const typeLabel = t(`serenityPanel.stepLogType.${view.kind}`);
                // 摘要优先级：结论文本（summary）> 数组条数（空→"空数据"）> 字段数
                const detail = view.textPreview
                  ? truncateText(view.textPreview, 42)
                  : view.count != null
                  ? (view.count === 0
                    ? t("serenityPanel.stepLogCountEmpty")
                    : t("serenityPanel.stepLogCountSuffix", { count: view.count }))
                  : view.fieldCount != null
                  ? t("serenityPanel.stepLogFieldSuffix", { count: view.fieldCount })
                  : "";
                summary = detail ? `${typeLabel} · ${detail}` : typeLabel;
              }
              return (
                <div
                  key={`${s.nodeId}-${i}`}
                  className="rounded border border-gray-100 px-2 py-1 text-xs"
                >
                  <div
                    className="flex items-center gap-2 cursor-pointer"
                    onClick={() => toggleStep(i)}
                  >
                    {statusIcon}
                    <Text strong className="text-xs shrink-0">
                      {t(nodeTitleKey(s.nodeId))}
                    </Text>
                    <Tag color={statusColor} className="text-xs shrink-0">
                      {s.status}
                    </Tag>
                    {s.elapsedMs != null && (
                      <Text type="secondary" className="text-xs shrink-0">
                        {(s.elapsedMs / 1000).toFixed(1)}s
                      </Text>
                    )}
                    {summary && (
                      <Text
                        type={isFailed ? "danger" : "secondary"}
                        className="text-xs truncate flex-1 min-w-0"
                        title={summary}
                      >
                        {summary}
                      </Text>
                    )}
                    <div className="flex-1" />
                    <Text
                      type={summary ? "secondary" : undefined}
                      className="text-xs cursor-pointer shrink-0"
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleStep(i);
                      }}
                    >
                      {isExpanded
                        ? <DownOutlined />
                        : <RightOutlined />}
                    </Text>
                  </div>
                  {isExpanded && (
                    isFailed && s.error
                      ? (
                        <div className="mt-1 text-xs text-red-500 whitespace-pre-wrap break-all">
                          {
                            /*
                            展开态 = 后端**原文**（`NodeError::Display`）全文。
                            刻意不在这里重复主文案：本地化主文案已由上面的折叠摘要承担，
                            展开的意义是「看完整原文」（摘要会截断 60 字符）。

                            原文可能含中文（如 "EXECUTION_CANCELLED: 节点执行已取消"）—— 这是
                            刻意的取舍：detail 承载具体原因（LLM 报错正文 / IO 详情），没有对应
                            译文，抹掉它会让失败无从排查。**主文案本地化 + 详情保留原文**。
                          */
                          }
                          {s.error}
                        </div>
                      )
                      : view.kind === "empty"
                      ? (
                        <div className="mt-1 text-xs text-gray-400 italic">
                          {s.status === "completed"
                            ? t("serenityPanel.stepLogCompletedNoOutput")
                            : s.status === "running"
                            ? t("serenityPanel.stepLogRunning")
                            : t("serenityPanel.stepLogNoDetail")}
                        </div>
                      )
                      : view.table
                      ? (
                        <div
                          className="mt-1 max-h-64 overflow-auto rounded p-1"
                          style={{
                            backgroundColor: "rgba(255,255,255,0.04)",
                            border: "1px solid rgba(255,255,255,0.08)",
                          }}
                        >
                          <Table
                            size="small"
                            pagination={false}
                            rowKey={(_, idx) => String(idx ?? 0)}
                            scroll={{ x: "max-content" }}
                            dataSource={view.table.rows}
                            columns={view.table.columns.map((c) => ({
                              title: (() => {
                                const label = FIELD_LABEL_MAP[c] ?? c;
                                return label.startsWith("serenityPanel.") ? t(label) : label;
                              })(),
                              dataIndex: c,
                              key: c,
                              ellipsis: true,
                              render: (v: unknown) => (
                                <span title={typeof v === "string" ? v : undefined}>{cellText(v, t)}</span>
                              ),
                            }))}
                          />
                        </div>
                      )
                      : (
                        <pre
                          className="mt-1 max-h-48 overflow-auto rounded p-2 text-xs whitespace-pre-wrap break-all"
                          style={{
                            backgroundColor: "rgba(255,255,255,0.04)",
                            border: "1px solid rgba(255,255,255,0.08)",
                            color: "rgba(230,230,230,0.9)",
                          }}
                        >
                        {view.jsonText.length > 2000
                          ? view.jsonText.slice(0, 2000) + "..."
                          : view.jsonText}
                        </pre>
                      )
                  )}
                </div>
              );
            })}
          </Space>
        </Card>
      )}

      {/* 错误 */}
      {error && !running && (
        <div
          className="rounded border border-red-500/30 p-2 text-sm text-red-400"
          style={{ backgroundColor: "rgba(255,77,79,0.08)" }}
        >
          <div>{error}</div>
          {
            /* 技术详情行：未本地化的原文（DB 报错 / 节点 `NodeError` 自由文本），仅供排查。
              与主文案相同则不渲染 —— 旧载荷下 `errorDetail` 会被置 null，
              但这里再挡一次，避免调用方误传同一串导致同一句话出现两行。 */
          }
          {errorDetail && errorDetail !== error && (
            <div className="mt-1 font-mono text-xs break-all opacity-80">{errorDetail}</div>
          )}
        </div>
      )}

      {/* 趋势摘要 */}
      {trends.length > 0 && !running && (
        <Card size="small" title={t("serenityPanel.trendTitle")} className="w-full">
          <Space orientation="vertical" className="w-full">
            {trends.map((tr, i) => (
              <div key={i} className="flex items-center gap-2 text-sm">
                <Tag color="purple">{tr.confidence ?? "?"}%</Tag>
                <Text strong>{tr.trend_name ?? tr.trendName}</Text>
                {tr.bottleneck_candidate && (
                  <Text type="secondary" className="text-xs">
                    {t("serenityPanel.bottleneckLink")}
                    {tr.bottleneck_candidate}
                  </Text>
                )}
              </div>
            ))}
          </Space>
        </Card>
      )}

      {/* 挂载恢复上次候选的加载态 */}
      {lastRunLoading && (
        <Card size="small" className="w-full">
          <div className="py-6 flex items-center justify-center gap-2 text-sm text-gray-400">
            <Spin size="small" />
            <span>{t("common.loading")}</span>
          </div>
        </Card>
      )}

      {/* 候选股列表 */}
      {candidates.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <Title level={5} className="m-0">
              {t("serenityPanel.candidateTitle")} ({candidates.length})
            </Title>
            <Button
              size="small"
              icon={<AlertOutlined />}
              onClick={async () => {
                try {
                  const anchorState = useTimeAnchorStore.getState();
                  const asOfDate = anchorState.mode === "replay" || anchorState.mode === "backtest_sweep"
                    ? anchorState.asOfDate
                    : null;
                  const r = await invoke<{
                    status: string;
                    checked_count: number;
                    exit_now_count: number;
                    caution_count: number;
                    candidates: Array<{
                      stock_code: string;
                      stock_name: string;
                      exit_urgency: string;
                      has_disruption_news: boolean;
                      margin_declining: boolean;
                    }>;
                  }>("refresh_serenity_exit_signals", { asOfDate });
                  if (r.exit_now_count > 0 || r.caution_count > 0) {
                    const msg = t("serenityPanel.exitSignalAlert", {
                      checked: r.checked_count,
                      exitNow: r.exit_now_count,
                      caution: r.caution_count,
                    });
                    alert(msg);
                  } else {
                    alert(t("serenityPanel.exitSignalAlertNone", { checked: r.checked_count }));
                  }
                } catch (e) {
                  console.error("Failed to refresh exit signal", e);
                }
              }}
            >
              {t("serenityPanel.refreshExitButton")}
            </Button>
          </div>
          {candidates.map((c, i) => {
            const code = c.stock_code ?? c.stockCode ?? "";
            return (
              <SerenityCandidateCard
                key={`${code}-${i}`}
                candidate={c}
              />
            );
          })}
        </div>
      )}

      {/* 空状态 / 解释无候选原因 */}
      {!running && !error && !lastRunLoading && candidates.length === 0 && trends.length === 0 && (
        emptyReason
          ? (
            <Alert
              type="info"
              showIcon
              title={t("serenityPanel.noCandidateTitle")}
              description={emptyReason}
              className="w-full"
            />
          )
          : (
            <Empty
              image={<StockOutlined style={{ fontSize: 48, opacity: 0.3 }} />}
              description={t("serenityPanel.emptyHint")}
            />
          )
      )}
      {!running && !error && !lastRunLoading && candidates.length === 0 && trends.length > 0 && emptyReason && (
        <Alert
          type="info"
          showIcon
          title={t("serenityPanel.noCandidateTitle")}
          description={emptyReason}
          className="w-full"
        />
      )}

      {/* 回馈闭环结果 */}
      {feedbackLoading && (
        <Card size="small" className="w-full">
          <div className="flex gap-4 mb-3">
            <div
              className="text-center flex-1 rounded p-3 animate-pulse"
              style={{ backgroundColor: "rgba(255,255,255,0.04)" }}
            />
            <div
              className="text-center flex-1 rounded p-3 animate-pulse"
              style={{ backgroundColor: "rgba(255,255,255,0.04)" }}
            />
            <div
              className="text-center flex-1 rounded p-3 animate-pulse"
              style={{ backgroundColor: "rgba(255,255,255,0.04)" }}
            />
          </div>
        </Card>
      )}
      {feedbackData && (
        <Card
          size="small"
          title={t("serenityPanel.feedbackTitle", { count: feedbackData.total })}
          className="w-full"
          extra={
            <Text type="secondary" className="text-xs">
              {new Date().toLocaleDateString()}
            </Text>
          }
        >
          <div className="flex gap-4 mb-3">
            <div className="text-center flex-1 rounded p-2" style={{ backgroundColor: "rgba(255,255,255,0.04)" }}>
              <div
                className="text-lg font-bold"
                style={{ color: feedbackData.win_rate >= 0.5 ? "#52c41a" : "#ff4d4f" }}
              >
                {(feedbackData.win_rate * 100).toFixed(0)}%
              </div>
              <div className="text-xs opacity-50">{t("serenityPanel.feedbackWinRate")}</div>
            </div>
            <div className="text-center flex-1 rounded p-2" style={{ backgroundColor: "rgba(255,255,255,0.04)" }}>
              <div
                className="text-lg font-bold"
                style={{ color: feedbackData.avg_return_pct >= 0 ? "#52c41a" : "#ff4d4f" }}
              >
                {feedbackData.avg_return_pct.toFixed(1)}%
              </div>
              <div className="text-xs opacity-50">{t("serenityPanel.feedbackAvgReturn")}</div>
            </div>
            <div className="text-center flex-1 rounded p-2" style={{ backgroundColor: "rgba(255,255,255,0.04)" }}>
              <div className="text-lg font-bold">{feedbackData.profitable_count}/{feedbackData.total}</div>
              <div className="text-xs opacity-50">{t("serenityPanel.feedbackProfitable")}</div>
            </div>
          </div>
          {/* 个股表现列表 */}
          <div className="max-h-48 overflow-y-auto">
            {feedbackData.performances.slice(0, 50).map((p, i) => (
              <div
                key={p.id ?? i}
                className="flex items-center justify-between py-1 text-xs border-b border-white/5 last:border-0"
              >
                <div className="flex items-center gap-2">
                  <span className="font-mono">{p.stock_code}</span>
                  <span className="text-gray-500">{p.stock_name}</span>
                  <span className="text-gray-400 text-2xs">{p.recommend_date}</span>
                </div>
                <div className="flex items-center gap-3">
                  {p.catalysts && (
                    <Tag className="text-2xs" color={p.catalysts.verified > 0 ? "green" : "default"}>
                      {t("serenityPanel.feedbackCatalyst", {
                        verified: p.catalysts.verified,
                        total: p.catalysts.total,
                      })}
                    </Tag>
                  )}
                  <span style={{ color: p.return_pct >= 0 ? "#52c41a" : "#ff4d4f" }}>
                    {p.return_pct > 0 ? "+" : ""}
                    {p.return_pct.toFixed(1)}%
                  </span>
                  <Button
                    type="text"
                    size="small"
                    danger
                    className="text-xs opacity-40 hover:opacity-100"
                    onClick={async (e) => {
                      e.stopPropagation();
                      try {
                        await invoke("delete_serenity_pick", { id: p.id });
                        setFeedbackData((prev) => {
                          if (!prev) { return prev; }
                          const perf = prev.performances.filter((x) => x.id !== p.id);
                          const total = perf.length;
                          const profitable = perf.filter((x) => x.is_profitable).length;
                          const avg_return = total > 0
                            ? perf.reduce((s, x) => s + x.return_pct, 0) / total
                            : 0;
                          return {
                            ...prev,
                            total,
                            profitable_count: profitable,
                            win_rate: total > 0 ? profitable / total : 0,
                            avg_return_pct: Number(avg_return.toFixed(2)),
                            performances: perf,
                          };
                        });
                      } catch (e) {
                        console.error("Delete failed", e);
                      }
                    }}
                  >
                    ✕
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {/* 瓶颈掘金历史（多选 + 批量删除） */}
      <Modal
        title={t("serenityPanel.serenityHistory.title")}
        open={serenityHistoryOpen}
        onCancel={() => {
          setSerenityHistoryOpen(false);
          setSerenitySelected([]);
        }}
        footer={serenitySelected.length > 0
          ? (
            <div className="flex items-center gap-2">
              <span className="text-xs text-gray-400">
                {t("serenityPanel.serenityHistory.selectedCount", { count: serenitySelected.length })}
              </span>
              <Button size="small" onClick={() => setSerenitySelected([])}>
                {t("serenityPanel.serenityHistory.exitSelect")}
              </Button>
              <Button
                size="small"
                danger
                loading={serenityDeleting}
                onClick={async () => {
                  setSerenityDeleting(true);
                  try {
                    await invoke("batch_delete_reco_history", { generatedAts: serenitySelected });
                    messageApi.success(
                      t("serenityPanel.serenityHistory.deleteSuccess", { count: serenitySelected.length }),
                    );
                    setSerenityHistory((prev) => prev.filter((r) => !serenitySelected.includes(r.generatedAt)));
                    setSerenitySelected([]);
                  } catch (e) {
                    showBackendError(messageApi, e);
                  }
                  setSerenityDeleting(false);
                }}
              >
                {t("serenityPanel.serenityHistory.batchDelete", { count: serenitySelected.length })}
              </Button>
            </div>
          )
          : null}
        width={620}
      >
        <Table
          size="small"
          loading={serenityHistoryLoading}
          dataSource={serenityHistory}
          rowKey="generatedAt"
          pagination={false}
          onRow={(record) => ({
            className: "cursor-pointer",
            onClick: () => openSerenityDetail(record),
          })}
          columns={[
            {
              title: (
                <Checkbox
                  checked={serenityHistory.length > 0 && serenitySelected.length === serenityHistory.length}
                  indeterminate={serenitySelected.length > 0 && serenitySelected.length < serenityHistory.length}
                  onChange={(e) => {
                    setSerenitySelected(e.target.checked ? serenityHistory.map((r) => r.generatedAt) : []);
                  }}
                />
              ),
              key: "select",
              width: 40,
              render: (_, r) => (
                <Checkbox
                  checked={serenitySelected.includes(r.generatedAt)}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => {
                    setSerenitySelected(
                      e.target.checked
                        ? [...serenitySelected, r.generatedAt]
                        : serenitySelected.filter((g) => g !== r.generatedAt),
                    );
                  }}
                />
              ),
            },
            {
              title: t("serenityPanel.serenityHistory.generatedAt"),
              dataIndex: "generatedAt",
              key: "generatedAt",
              render: (v: string) => (
                <span className="text-xs font-mono">
                  {new Date(v).toLocaleString()}
                </span>
              ),
            },
            {
              title: t("serenityPanel.serenityHistory.candidateCount"),
              dataIndex: "stockCount",
              key: "stockCount",
              render: (v: number) => <span className="text-xs">{v}{t("serenityPanel.filterSuffixCount")}</span>,
            },
            {
              title: t("serenityPanel.serenityHistory.actions"),
              key: "actions",
              width: 64,
              render: (_, r: { generatedAt: string; stockCount: number; createdAt: string; styles: string }) => (
                <Popconfirm
                  title={t("serenityPanel.serenityHistory.deleteOneConfirm")}
                  onConfirm={() => handleDeleteOne(r)}
                >
                  <Button
                    type="text"
                    size="small"
                    danger
                    icon={<DeleteOutlined />}
                    loading={serenityDeleting}
                    onClick={(e) => e.stopPropagation()}
                  />
                </Popconfirm>
              ),
            },
          ]}
        />
      </Modal>

      {/* 瓶颈掘金历史详情 */}
      <Modal
        title={serenityDetailRow
          ? `${t("serenityPanel.serenityHistory.title")} — ${new Date(serenityDetailRow.generatedAt).toLocaleString()}`
          : ""}
        open={serenityDetailOpen}
        onCancel={() => {
          setSerenityDetailOpen(false);
          setSerenityDetailItems([]);
          setSerenityDetailRow(null);
        }}
        footer={null}
        width={600}
      >
        {serenityDetailLoading
          ? (
            <div className="py-8 text-center text-sm text-gray-400">
              {t("common.loading")}
            </div>
          )
          : serenityDetailItems.length === 0
          ? <Empty description={t("serenityPanel.serenityHistory.detailEmpty")} />
          : (
            <div className="flex flex-col gap-2">
              <div className="text-xs text-gray-500 mb-1">
                {t("serenityPanel.serenityHistory.candidateCount")}: {serenityDetailItems.length}
                {t("serenityPanel.filterSuffixCount")}
              </div>
              {serenityDetailItems.map((item, i) => (
                <Card
                  key={`${item.stockCode}-${i}`}
                  size="small"
                  hoverable
                  className="w-full"
                  onClick={() => {
                    setSerenityDetailOpen(false);
                    jumpToStock({ code: item.stockCode, name: item.stockName });
                  }}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Text strong className="text-sm">{item.stockName}</Text>
                      <Text type="secondary" className="text-xs font-mono">{item.stockCode}</Text>
                    </div>
                    <Tag color="purple" className="text-xs font-bold">
                      {t("serenityPanel.confidencePrefix")} {item.confidence}
                    </Tag>
                  </div>
                  <div className="mt-1 text-[10px] text-gray-500">
                    {new Date(item.generatedAt).toLocaleString()}
                  </div>
                </Card>
              ))}
            </div>
          )}
      </Modal>
    </div>
  );
}
