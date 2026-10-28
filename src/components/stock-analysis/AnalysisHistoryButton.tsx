import { invoke } from "@/lib/invoke";
import {
  actionSourceLabelKey,
  confidenceSourceLabelKey,
  FAST_TEMPLATE_ID,
  filterAnalysisHistory,
  getActionTagStyle,
  getActionTKey,
  groupAnalysisHistory,
  HORIZON_T_SUFFIX,
  horizonSourceLabelKey,
  mergeAnalysisHistoryPages,
  provenanceOfRecord,
  readDecisionProvenance,
  readHorizonActions,
  resolveRecordDisplayAction,
  StockAction,
  UNKNOWN_HORIZON,
} from "@/lib/stock-analysis-utils";
import { App, Button, Dropdown, Input, Tag } from "antd";
import { Check, History, Pencil, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

interface AnalysisRecord {
  id: string;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  /** 决策动作（后端直返，如 BUY/SELL/HOLD/WAIT/UNCERTAIN） */
  decisionAction: string | null;
  /** 决策仓位百分比（后端直返，0-100） */
  decisionPositionPct: number | null;
  /** 决策持仓状态轴（后端直返，v228）；null = 记录早于 v228，非 EMPTY */
  decisionPositionState: string | null;
  /** 完整决策 JSON（含 confidence 等，部分旧数据可能为 null） */
  decisionJson: string | null;
  /** 列表场景不返回，详情页通过 get_stock_analysis 单独获取 */
  blackboardSnapshot?: string | null;
  createdAt: number;
  updatedAt?: number;
  status: string;
  /** 版本化分析：指向原始记录 ID，null 表示首次分析 */
  parentAnalysisId: string | null;
  /**
   * 工作流模板 id：`"stock-analysis-fast"` = 快速 JEV 链。
   * `undefined` / `null` = 未知（本列引入前的记录或非模板产出）—— 不打标识。
   */
  templateId?: string | null;
  /**
   * 主结论所属档位（`ultra_short` / `short` / `mid` / `long`）；null = 本列引入前的记录，
   * 按「未知周期」显示，不得推断成某档。
   */
  decisionTimeHorizon?: string | null;
  /** 档位来源：`formula` = 本地公式定档；`model` = 采信模型自报；null = 未知。 */
  decisionHorizonSource?: string | null;
  /**
   * 该记录由**哪一代工作流图**跑出来的快照（写侧在建点当时取 `workflow_templates.version`）。
   * `null` / `undefined` = 本列引入前的记录或非模板产出 ⇒ 显示「版本未知」，
   * **不得**回退成 0 或当代（那会把「不知道」伪装成「就是最新判据」）。
   */
  templateVersion?: number | null;
  /**
   * 记录来历（后端 `stock_analyses.analysis_kind`，NOT NULL）：`"live"` = 实况分析，
   * `"replay"` = as-of 回放（写侧判据是 `asof_ctx.is_some()`，见 `stock_workflow/core.rs`）。
   * ⚠ 与 `useTimeAnchorStore.anchorMode === "replay"` **不是一回事** —— 后者是「查看者当前的
   * 时间锚」，前者是「这条记录当时怎么跑出来的」；同词不同源，混用会把「我在看哪天」当成「这条是回放」。
   */
  analysisKind?: string | null;
  /**
   * 回放样本的**决策日**（代际裁定 2026-10-13：同表 + `as_of_date` 列）。
   * 现网实测 283 行**全部非空** ⇒ 它不能用来区分 live / 回放（那是 `analysisKind` 的活），
   * 但它与 `analysisDate` 可以不同（87 条回放行里 37 条不同），不同才是有信息量的那部分。
   */
  asOfDate?: string | null;
}

/** `get_stock_template_version_status` 的返回形状（后端 `StockTemplateVersionStatus`，camelCase）。 */
interface TemplateVersionStatus {
  /** 代码里的权威代（下次播种会写入的值）。 */
  codeVersion: number;
  /** 库里已播种到的代；`null` = 库里没有这张图的行（尚未播种）。 */
  dbVersion: number | null;
}

// 代际对照的四种状态各占一个文案键 —— 刻意不合并成「一致 / 不一致」两态：
// 「库里高于代码」正是本仓真实事故（版本门 `existing >= TEMPLATE_VERSION ⇒ 跳过`，
// 三批改动一字不落库），把它和「落后」并成一句就看不出方向了。
export function versionStatusKey(v: TemplateVersionStatus): string {
  if (v.dbVersion === null) { return "stockAnalysis.templateVersion.notSeeded"; }
  if (v.dbVersion === v.codeVersion) { return "stockAnalysis.templateVersion.matched"; }
  return v.dbVersion < v.codeVersion
    ? "stockAnalysis.templateVersion.behind"
    : "stockAnalysis.templateVersion.ahead";
}

// 与「快速链」「档位」两枚标签同尺寸的紧凑形态（新标签复用，不再抄第四份内联样式）。
const COMPACT_TAG_STYLE = {
  margin: 0,
  fontSize: 10,
  lineHeight: "16px",
  padding: "0 4px",
  border: "1px solid var(--muted, #888)",
  color: "var(--muted, #888)",
  background: "transparent",
} as const;

/**
 * 单页取数量。后端 `list_stock_analyses` 的 `MAX_LIMIT` 是 1000（超出会被截断并 warn），
 * 取 500 留一半余量；现网读数 360 条 ⇒ 通常一次就取完。
 */
const PAGE_SIZE = 500;

/**
 * 一次「加载全部」最多翻多少页（500 × 20 = 1 万条）。
 *
 * 有这个封顶就必须让它**可见**：撞到上限时界面要说「可能还有更早的记录」并给「继续加载」，
 * 否则就是拿一个新的静默截断顶掉旧的 `limit: 30`（那条正是本次要消除的缺陷）。
 */
const MAX_PAGES_PER_ROUND = 20;

/**
 * 状态筛选的选项值域 = 后端 `stock_analyses.status` 的全部取值
 * （`completed` + `recordStatusKey` 认得的另外五个）。
 * 值域权威在 `crates/entities/src/stock_analyses.rs` 的列注释，新增态时这里要同步加。
 */
const STATUS_OPTIONS: readonly string[] = [
  "completed",
  "running",
  "interrupted",
  "failed",
  "timeout",
  "cancelled",
];

/** 档位筛选的选项值域：四档取自 `HORIZON_T_SUFFIX`（不复制第二份档位清单）+ 「未知周期」。 */
const HORIZON_OPTIONS: readonly string[] = [...Object.keys(HORIZON_T_SUFFIX), UNKNOWN_HORIZON];

/**
 * 结论筛选的选项值域：八档全部取自 `StockAction`（值域权威 = 后端 `ActionKind`，
 * 由 `scripts/check-decision-action-single-source.mjs` 五面对账），不另加自造档位。
 *
 * 2026-10-22 裁定后撤掉了此前追加的「无结论」哨兵格 —— 它与权威档「数据缺失」重叠：
 * 后端对空 `decision_action` 的既定口径是下发 `UNAVAILABLE` 哨兵（列表路径本轮已补齐），
 * 「还没有决策」的只剩 `running` 行，那一格由状态维度表达。
 */
const ACTION_OPTIONS: readonly string[] = Object.values(StockAction);

/** 结论选项的展示键：八档复用 Tag 那批 `stockAnalysis.action*` 键。 */
export function actionOptionLabelKey(option: string): string {
  return getActionTKey(option);
}

/** 状态值的展示键：`completed` 用既有独立键，其余走 `recordStatus.*`；不认识的态退回原始串。 */
export function statusLabelKey(status: string): string | null {
  return status === "completed" ? "stockAnalysis.completed" : recordStatusKey(status);
}

/**
 * 筛选 chip —— 刻意用普通 button 而不是 antd Select：Select 的下拉面板是 portal，
 * 点选项会被外层 Dropdown 判成「点击外部」而把整个历史弹窗关掉。
 */
function FilterChip({ label, active, onClick }: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      style={{
        cursor: "pointer",
        fontSize: 10,
        lineHeight: "16px",
        padding: "1px 6px",
        borderRadius: 8,
        border: `1px solid ${active ? "var(--accent, #7c3aed)" : "var(--muted, #888)"}`,
        background: active ? "var(--accent-bg, rgba(124,58,237,0.18))" : "transparent",
        color: active ? "var(--accent, #7c3aed)" : "var(--muted, #888)",
      }}
    >
      {label}
    </button>
  );
}

/**
 * 记录状态 → i18n key。返回 `null` = 该值不在已知值域内（调用方退回显示原始字符串，
 * **不吞信息**）。
 *
 * 修复（2026-10-17）：此前这里把 `r.status` **原样打印**，界面出现的是后端字面量
 * 英文小写串（`running` / `interrupted`）；读者既看不懂，也无从分辨
 * 「仍在跑」与「已中断」—— 用户据此报障「挂着好多过期记录状态为 running」。
 * 后端值域见 `src-tauri/crates/entities/src/stock_analyses.rs` 的 `status` 列注释。
 */
export function recordStatusKey(status: string): string | null {
  if (status === "running") { return "stockAnalysis.recordStatus.running"; }
  if (status === "interrupted") { return "stockAnalysis.recordStatus.interrupted"; }
  if (status === "failed") { return "stockAnalysis.recordStatus.failed"; }
  if (status === "timeout") { return "stockAnalysis.recordStatus.timeout"; }
  if (status === "cancelled") { return "stockAnalysis.recordStatus.cancelled"; }
  return null;
}

/** 个股分析页搜索框下方的历史分析快捷按钮 */
export function AnalysisHistoryButton() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [open, setOpen] = useState(false);
  // 全量加载的三种未完成状态各自独立，且**都必须被界面说出来**（缺料不得伪装成「就这么多了」）：
  // `loading` 还在翻页 / `capped` 撞到单次上限 / `loadFailed` 取数失败。三者皆假 = 已取全。
  const [loading, setLoading] = useState(false);
  const [capped, setCapped] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [keyword, setKeyword] = useState("");
  const [statusFilter, setStatusFilter] = useState<string | undefined>(undefined);
  const [horizonFilter, setHorizonFilter] = useState<string | undefined>(undefined);
  const [actionFilter, setActionFilter] = useState<string | undefined>(undefined);
  // 位置式翻页游标：与「已到手条数」分家，避免并发插入造成的重复页把游标顶歪。
  const offsetRef = useRef(0);
  // 每轮加载自增；回调回来时不等于当前轮次即丢弃（弹窗已重开，这一轮的结果属于上一轮）。
  const roundRef = useRef(0);
  // 代码版本 vs 库里已播种版本的对照（弹窗顶部一行）。`null` = 还没取到 / 取失败 ⇒ 不显示该行，
  // 但**不得**显示「一致」—— 没取到就是没取到（结构性缺口不得在 UI 造成歧义）。
  const [versionStatus, setVersionStatus] = useState<TemplateVersionStatus | null>(null);
  // editingId !== null 表示正在重命名该记录
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  /**
   * 从当前游标起连续翻页，直到某页不满（取完）或撞到单次上限。
   *
   * @param restart `true` = 重新打开弹窗，游标归零从头取（已到手的数据先留着显示，不闪空）。
   */
  const loadPages = useCallback(async (restart: boolean) => {
    const round = ++roundRef.current;
    if (restart) { offsetRef.current = 0; }
    setLoading(true);
    setLoadFailed(false);
    setCapped(false);
    let reachedEnd = false;
    // 只有本轮**第一页**做整体替换（重开弹窗时把上一轮的陈旧数据换掉），后续页一律追加。
    let replaces = restart;
    for (let page = 0; page < MAX_PAGES_PER_ROUND; page++) {
      let rows: AnalysisRecord[];
      try {
        rows = await invoke<AnalysisRecord[]>("list_stock_analyses", {
          limit: PAGE_SIZE,
          offset: offsetRef.current,
        });
      } catch {
        if (round === roundRef.current) {
          setLoadFailed(true);
          setLoading(false);
        }
        return;
      }
      // 等待期间弹窗被重开 ⇒ 这一轮不再属于当前视图，整轮丢弃
      if (round !== roundRef.current) { return; }
      const list = Array.isArray(rows) ? rows : [];
      offsetRef.current += list.length;
      setRecords((prev) =>
        replaces
          ? mergeAnalysisHistoryPages([], list)
          : mergeAnalysisHistoryPages(prev, list)
      );
      replaces = false;
      if (list.length < PAGE_SIZE) {
        reachedEnd = true;
        break;
      }
    }
    if (round !== roundRef.current) { return; }
    setLoading(false);
    setCapped(!reachedEnd);
  }, []);

  useEffect(() => {
    if (!open) { return; }
    let cancelled = false;
    void loadPages(true);
    invoke<TemplateVersionStatus>("get_stock_template_version_status")
      .then((v) => {
        if (!cancelled) { setVersionStatus(v); }
      })
      .catch(() => {
        if (!cancelled) { setVersionStatus(null); }
      });
    return () => {
      cancelled = true;
    };
  }, [open, loadPages]);

  useEffect(() => {
    if (editingId && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editingId]);

  const handleDelete = useCallback((e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    void invoke("delete_stock_analysis", { analysisId: id })
      .then(() => {
        setRecords((prev) => prev.filter((r) => r.id !== id));
        message.success(t("stockAnalysis.deleteSuccess"));
      })
      .catch(() => {
        message.error(t("stockAnalysis.deleteFailed"));
      });
  }, [t, message]);

  const startRename = useCallback((e: React.MouseEvent, r: AnalysisRecord) => {
    e.stopPropagation();
    setEditingId(r.id);
    setEditValue(r.stockName);
  }, []);

  const confirmRename = useCallback((e: React.MouseEvent | KeyboardEvent, id: string) => {
    e.stopPropagation?.();
    const newName = editValue.trim();
    if (!newName) { return; }
    void invoke("rename_stock_analysis", { analysisId: id, newName })
      .then(() => {
        setRecords((prev) => prev.map((r) => r.id === id ? { ...r, stockName: newName } : r));
        setEditingId(null);
        message.success(t("stockAnalysis.renameSuccess"));
      })
      .catch(() => {
        message.error(t("stockAnalysis.renameFailed"));
      });
  }, [editValue, t, message]);

  const cancelRename = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    setEditingId(null);
  }, []);

  // 按 stockCode 分组（与详情页历史列表共用 groupAnalysisHistory）：
  // 组名取组内第一条非空名称，一条空名新记录不会把整组标题抹成只剩代码。
  const filtered = useMemo(
    () =>
      filterAnalysisHistory(records, {
        keyword,
        status: statusFilter,
        horizon: horizonFilter,
        action: actionFilter,
      }),
    [records, keyword, statusFilter, horizonFilter, actionFilter],
  );
  const grouped = useMemo(() => groupAnalysisHistory(filtered), [filtered]);
  const filterActive = Boolean(keyword.trim() || statusFilter || horizonFilter || actionFilter);

  // 记录名：日期（精确到日期），重跑版本加 ↻ 标记
  const dateLabel = (r: AnalysisRecord) => {
    const date = r.analysisDate || (r.createdAt ? new Date(r.createdAt).toLocaleDateString() : "");
    return r.parentAnalysisId ? `${date} ↻` : date;
  };

  // 分析时间：精确到分钟（HH:mm），用于区分同日多次分析
  const timeLabel = (r: AnalysisRecord) => {
    if (!r.createdAt) { return ""; }
    return new Date(r.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  };

  // 解析决策信息：结论档走共用判据 `resolveRecordDisplayAction`（与「结论」筛选同一个权威 ⇒
  // 筛出来的必然等于 Tag 上那枚），decisionJson 只用来补 confidence / positionPct（兼容旧数据）。
  // 2026-09-22: 展示档 = 方向档（`resolveDisplayAction` 已恒等，**不再按仓位派生**）——
  //   历史上 P1-2(2026-09-14)/V76 曾按 positionState/positionPct 派生，因「用本次决策算出的
  //   建议仓位反推本次展示名」属循环判据而被废除（LLM 措辞抖动能经「试探仓 → positionPct →
  //   positionState」把「观望」翻成「持有」）—— 见 `AUDIT-300642-run-variance-2026-09-22.md`。
  const decisionInfo = (r: AnalysisRecord): {
    action: string;
    positionPct?: number;
    confidence?: number;
  } | null => {
    const action = resolveRecordDisplayAction(r);
    if (!action) { return null; }
    let positionPct = r.decisionPositionPct ?? undefined;
    let confidence: number | undefined;
    if (r.decisionJson) {
      try {
        const d = JSON.parse(r.decisionJson) as Record<string, unknown>;
        if (d && typeof d === "object") {
          if (positionPct == null && typeof d.positionPct === "number") { positionPct = d.positionPct; }
          if (typeof d.confidence === "number") { confidence = d.confidence; }
        }
      } catch { /* 坏 JSON ⇒ 只有 Tag，不编造仓位与置信度 */ }
    }
    return { action, positionPct, confidence };
  };

  return (
    <Dropdown
      open={open}
      onOpenChange={setOpen}
      trigger={["click"]}
      popupRender={() => (
        <div
          style={{
            width: 520,
            maxHeight: 520,
            overflowY: "auto",
            background: "var(--surface, #1a1a2e)",
            border: "1px solid var(--color-border, #333)",
            borderRadius: 8,
            padding: 4,
          }}
        >
          {versionStatus && (
            <div
              style={{
                padding: "4px 10px",
                fontSize: 10,
                color: "var(--muted, #888)",
                borderBottom: "1px solid var(--color-border, #333)",
              }}
            >
              {t(versionStatusKey(versionStatus), {
                code: versionStatus.codeVersion,
                db: versionStatus.dbVersion ?? "-",
                gap: versionStatus.dbVersion === null
                  ? 0
                  : Math.abs(versionStatus.codeVersion - versionStatus.dbVersion),
              })}
            </div>
          )}
          {
            /* 筛选条 + 完整性那一行：sticky 让「筛了什么 / 取全了没有」在滚动时始终可见，
              组标题（zIndex 1）滚到它下面即被盖住，不会两行叠成一行看不清。 */
          }
          <div
            style={{
              position: "sticky",
              top: -4,
              zIndex: 3,
              background: "var(--surface, #1a1a2e)",
              padding: "4px 10px 6px",
              borderBottom: "1px solid var(--color-border, #333)",
              display: "flex",
              flexDirection: "column",
              gap: 4,
            }}
          >
            <Input
              size="small"
              allowClear
              value={keyword}
              onChange={(e) => setKeyword(e.target.value)}
              placeholder={t("stockAnalysis.historyFilter.keywordPlaceholder")}
            />
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4, alignItems: "center" }}>
              <span style={{ fontSize: 10, color: "var(--muted, #888)", marginRight: 2 }}>
                {t("stockAnalysis.historyFilter.statusLabel")}
              </span>
              <FilterChip
                label={t("stockAnalysis.historyFilter.all")}
                active={!statusFilter}
                onClick={() => setStatusFilter(undefined)}
              />
              {STATUS_OPTIONS.map((s) => {
                const key = statusLabelKey(s);
                return (
                  <FilterChip
                    key={s}
                    label={key ? t(key) : s}
                    active={statusFilter === s}
                    onClick={() => setStatusFilter(s)}
                  />
                );
              })}
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4, alignItems: "center" }}>
              <span style={{ fontSize: 10, color: "var(--muted, #888)", marginRight: 2 }}>
                {t("stockAnalysis.timeHorizon")}
              </span>
              <FilterChip
                label={t("stockAnalysis.historyFilter.all")}
                active={!horizonFilter}
                onClick={() => setHorizonFilter(undefined)}
              />
              {HORIZON_OPTIONS.map((h) => (
                <FilterChip
                  key={h}
                  label={h === UNKNOWN_HORIZON
                    ? t("stockAnalysis.reflection.horizonUnknown")
                    : t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[h]}`)}
                  active={horizonFilter === h}
                  onClick={() => setHorizonFilter(h)}
                />
              ))}
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4, alignItems: "center" }}>
              <span style={{ fontSize: 10, color: "var(--muted, #888)", marginRight: 2 }}>
                {t("stockAnalysis.historyFilter.actionLabel")}
              </span>
              <FilterChip
                label={t("stockAnalysis.historyFilter.all")}
                active={!actionFilter}
                onClick={() => setActionFilter(undefined)}
              />
              {ACTION_OPTIONS.map((a) => (
                <FilterChip
                  key={a}
                  label={t(actionOptionLabelKey(a))}
                  active={actionFilter === a}
                  onClick={() => setActionFilter(a)}
                />
              ))}
            </div>
            {
              /* 完整性那一行：取数失败 / 还在翻页 / 撞到单次上限 / 已取全（筛选态另给分子），
                四种各占一句，缺料绝不显示成「共 N 条」这种齐备态读数。 */
            }
            {!loading && !loadFailed && !capped && records.length === 0
              ? null
              : (
                <div
                  data-testid="history-completeness"
                  style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 10, color: "var(--muted, #888)" }}
                >
                  <span>
                    {loadFailed
                      ? t("stockAnalysis.historyFilter.loadFailed")
                      : loading
                      ? t("stockAnalysis.historyFilter.loadedPartial", { count: records.length })
                      : capped
                      ? t("stockAnalysis.historyFilter.capped", { count: records.length })
                      : filterActive
                      ? t("stockAnalysis.historyFilter.showingTotal", {
                        shown: filtered.length,
                        total: records.length,
                      })
                      : t("stockAnalysis.historyFilter.loadedTotal", { total: records.length })}
                  </span>
                  {(loadFailed || capped) && (
                    <Button
                      size="small"
                      type="link"
                      style={{ fontSize: 10, padding: 0, height: "auto" }}
                      onClick={() => void loadPages(loadFailed)}
                    >
                      {loadFailed ? t("stockAnalysis.retry") : t("stockAnalysis.historyFilter.loadMore")}
                    </Button>
                  )}
                </div>
              )}
          </div>
          {loadFailed && records.length === 0
            ? null
            : records.length === 0
            ? (
              <div style={{ padding: "12px 8px", color: "var(--muted, #888)", textAlign: "center", fontSize: 12 }}>
                {t("stockAnalysis.noHistory")}
              </div>
            )
            : filtered.length === 0
            ? (
              // 一条都不剩时要把「库里有多少」一起说，否则「筛空了」和「库里没记录」在界面上同形
              <div style={{ padding: "12px 8px", color: "var(--muted, #888)", textAlign: "center", fontSize: 12 }}>
                {t("stockAnalysis.historyFilter.noMatch", { total: records.length })}
              </div>
            )
            : grouped.map((g) => (
              <div key={g.stockCode}>
                {/* 组标题：股票名称(股票代码) - 名称用主题色，代码用次要色 */}
                <div
                  style={{
                    padding: "4px 10px",
                    fontSize: 11,
                    fontWeight: 600,
                    borderBottom: "1px solid var(--color-border, #333)",
                    marginBottom: 2,
                    position: "sticky",
                    top: 0,
                    background: "var(--surface, #1a1a2e)",
                    zIndex: 1,
                    display: "flex",
                    alignItems: "center",
                    gap: 6,
                  }}
                >
                  {g.stockName
                    ? (
                      <>
                        <span style={{ color: "var(--accent, #7c3aed)" }}>{g.stockName}</span>
                        <span style={{ color: "var(--muted, #888)", fontSize: 10 }}>
                          ({g.stockCode})
                        </span>
                      </>
                    )
                    : <span style={{ color: "var(--accent, #7c3aed)" }}>{g.stockCode}</span>}
                </div>
                {/* 组内记录：日期为记录名 */}
                {g.items.map((r) => (
                  <div
                    key={r.id}
                    style={{
                      padding: "6px 10px 6px 20px",
                      cursor: "pointer",
                      borderRadius: 4,
                      fontSize: 13,
                      display: "flex",
                      alignItems: "center",
                      gap: 6,
                    }}
                    onClick={() => {
                      if (editingId) { return; }
                      setOpen(false);
                      // 2026-10-28 起 `/stock-analysis/:id` 已删除，记录详情并入投资工作区
                      // （`?tab=workspace&view=analysis`，`analysisId` 由 StockAnalysisPage 消费）。
                      const qs = new URLSearchParams({
                        tab: "workspace",
                        view: "analysis",
                        stockCode: r.stockCode,
                        analysisId: r.id,
                      });
                      if (r.stockName) { qs.set("stockName", r.stockName); }
                      navigate(`/invest?${qs.toString()}`);
                    }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLElement).style.background =
                        "color-mix(in srgb, var(--fg) 6%, transparent)";
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLElement).style.background = "transparent";
                    }}
                  >
                    {/* 名称：编辑模式 vs 显示模式 */}
                    {editingId === r.id
                      ? (
                        <div
                          style={{ flex: 1, display: "flex", alignItems: "center", gap: 4 }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <Input
                            // eslint-disable-next-line @typescript-eslint/no-explicit-any
                            ref={inputRef as any}
                            size="small"
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            onKeyDown={(e) => {
                              // eslint-disable-next-line @typescript-eslint/no-explicit-any
                              if (e.key === "Enter") { confirmRename(e as any, r.id); }
                              if (e.key === "Escape") { setEditingId(null); }
                            }}
                            style={{ width: 200, height: 26, fontSize: 12 }}
                          />
                          <span
                            style={{ cursor: "pointer", display: "inline-flex", color: "var(--green)" }}
                            onClick={(e) => confirmRename(e, r.id)}
                          >
                            <Check size={14} />
                          </span>
                          <span
                            style={{ cursor: "pointer", display: "inline-flex", color: "var(--muted, #888)" }}
                            onClick={cancelRename}
                          >
                            <X size={14} />
                          </span>
                        </div>
                      )
                      : (
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div className="flex items-center gap-1">
                            <span
                              style={{
                                fontWeight: 500,
                                whiteSpace: "nowrap",
                                overflow: "hidden",
                                textOverflow: "ellipsis",
                              }}
                            >
                              {dateLabel(r)}
                            </span>
                            {
                              /* 链路标识（2026-09-24）：快速 JEV 链与完整链的记录在
                                本列表里形态一致（analysisKind 同为 live），不打标识时
                                用户看到的只是「同一只股票两条互相矛盾的结论」。
                                templateId 为 null（本列引入前 / 非模板产出）时不打。 */
                            }
                            {r.templateId === FAST_TEMPLATE_ID && (
                              <Tag
                                style={{
                                  margin: 0,
                                  fontSize: 10,
                                  lineHeight: "16px",
                                  padding: "0 4px",
                                  border: "1px solid var(--muted, #888)",
                                  color: "var(--muted, #888)",
                                  background: "transparent",
                                }}
                              >
                                {t("stockAnalysis.fastAnalysis")}
                              </Tag>
                            )}
                            {
                              /* 图代标识：这条结论由哪一代工作流图跑出（本仓版本门频繁换代，
                              同一判据的两代可能给出不同结论；无该列的记录显式标「版本未知」。
                              ⚠ 快速链**不打**这一枚：它的 `template_version` 是另一套计数
                              （`FAST_TEMPLATE_VERSION` 恒 1 = 「派生资产第 1 版」，不是图代），
                              与主图的 v143 并排显示会伪造可比性；是哪条链由上面「快速分析」标签说。 */
                            }
                            {r.templateId !== FAST_TEMPLATE_ID && (
                              <Tag style={COMPACT_TAG_STYLE}>
                                {r.templateVersion == null
                                  ? t("stockAnalysis.templateVersion.unknown")
                                  : `v${r.templateVersion}`}
                              </Tag>
                            )}
                            {
                              /* 来历标识（2026-10-22）：实况**不打标** —— `analysis_kind` 是 NOT NULL 且
                                只有 live / replay 两态，第三种值另打「来源未知」⇒「无标」可被证明就是实况。
                                回放打「回放」，决策日与列表日期不同时带上决策日（现网 87 条回放里 37 条不同）。
                                ⚠ 别拿 `asOfDate` 是否为空来分来历：库里 283 行**全部非空**。 */
                            }
                            {(() => {
                              const provenance = provenanceOfRecord(r);
                              if (provenance === "live") { return null; }
                              if (provenance === "unknown") {
                                return (
                                  <Tag style={COMPACT_TAG_STYLE} data-testid="provenance-tag">
                                    {t("stockAnalysis.provenanceUnknown")}
                                  </Tag>
                                );
                              }
                              const diffDay = !!r.asOfDate && r.asOfDate !== r.analysisDate;
                              return (
                                <Tag style={COMPACT_TAG_STYLE} data-testid="provenance-tag">
                                  {diffDay
                                    ? t("stockAnalysis.provenanceReplayWithDate", { date: r.asOfDate })
                                    : t("stockAnalysis.provenanceReplay")}
                                </Tag>
                              );
                            })()}
                            {/* 决策结论 Tag */}
                            {(() => {
                              const info = decisionInfo(r);
                              if (!info) { return null; }
                              return (
                                <Tag
                                  style={getActionTagStyle(info.action)}
                                  data-testid="action-tag"
                                >
                                  {t(getActionTKey(info.action))}
                                </Tag>
                              );
                            })()}
                            <Tag
                              style={{
                                margin: 0,
                                fontSize: 10,
                                lineHeight: "16px",
                                padding: "0 4px",
                                border: "1px solid var(--muted, #888)",
                                color: "var(--muted, #888)",
                                background: "transparent",
                              }}
                            >
                              {r.decisionTimeHorizon && HORIZON_T_SUFFIX[r.decisionTimeHorizon]
                                ? t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[r.decisionTimeHorizon]}`)
                                : t("stockAnalysis.reflection.horizonUnknown")}
                              {(() => {
                                const srcKey = horizonSourceLabelKey(r.decisionHorizonSource);
                                return srcKey ? ` · ${t(srcKey)}` : "";
                              })()}
                            </Tag>
                          </div>
                          {timeLabel(r) && (
                            <div style={{ fontSize: 10, color: "var(--muted, #888)", lineHeight: 1.2 }}>
                              {t("stockAnalysis.analysisTime")} {timeLabel(r)}
                            </div>
                          )}
                          {/* 重要要素：仓位 + 置信度 */}
                          {(() => {
                            const info = decisionInfo(r);
                            if (!info) { return null; }
                            const parts: string[] = [];
                            if (info.positionPct != null) {
                              parts.push(`${t("stockAnalysis.decision.positionPct")}${info.positionPct}%`);
                            }
                            if (info.confidence != null) {
                              parts.push(`${t("stockAnalysis.decision.confidence")}${info.confidence.toFixed(0)}%`);
                            }
                            if (parts.length === 0) { return null; }
                            return (
                              <div style={{ fontSize: 10, color: "var(--muted, #888)", lineHeight: 1.2 }}>
                                {parts.join(" · ")}
                              </div>
                            );
                          })()}

                          {
                            /* 主档来历（§五十三 ①，v127）：与 HistoricalAnalysisPanel 同一判据 ——
                              主档可能是「所选档被后置规则单向降级」的结果，不成句说明就会与
                              四档 chips 同屏互斥。v127 之前的记录无此字段 ⇒ 不渲染、不编造。 */
                          }
                          {(() => {
                            const prov = readDecisionProvenance(r.decisionJson);
                            if (!prov) { return null; }
                            const srcLabel = actionSourceLabelKey(prov.actionSource);
                            if (!srcLabel) { return null; }
                            const horizonName = prov.horizon && HORIZON_T_SUFFIX[prov.horizon]
                              ? t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[prov.horizon]}`)
                              : "";
                            const actionText = t(getActionTKey(decisionInfo(r)?.action ?? ""));
                            const body = prov.kind === "downgraded"
                              ? t("stockAnalysis.decisionProvenanceDowngraded", {
                                horizon: horizonName,
                                branchAction: t(getActionTKey(prov.branchAction ?? "")),
                                reason: t(srcLabel),
                                action: actionText,
                              })
                              : prov.kind === "direct"
                              ? t("stockAnalysis.decisionProvenanceDirect", {
                                horizon: horizonName,
                                action: actionText,
                              })
                              : t(srcLabel);
                            const confSrcKey = prov.confidenceSource === "main_chain_posterior"
                              ? confidenceSourceLabelKey(prov.confidenceSource)
                              : null;
                            return (
                              <div style={{ fontSize: 10, color: "var(--muted, #888)", lineHeight: 1.2 }}>
                                {body}
                                {confSrcKey ? ` · ${t(confSrcKey)}` : ""}
                              </div>
                            );
                          })()}

                          {
                            /* 四档 Action（2026-09-29）：一次分析产四档决策，历史行原先只显示主档
                            那条 ⇒ 四档明细虽在 decisionJson 里下发却无人渲染。旧记录无该结构时
                            不渲染（缺席不伪造）。 */
                          }
                          {(() => {
                            const tiers = readHorizonActions(r.decisionJson);
                            if (tiers.length === 0) { return null; }
                            return (
                              <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                                {tiers.map((x) => (
                                  <Tag
                                    key={x.key}
                                    style={{
                                      ...getActionTagStyle(x.action),
                                      margin: 0,
                                      fontSize: 10,
                                      lineHeight: "14px",
                                      padding: "0 3px",
                                    }}
                                  >
                                    {t(`stockAnalysis.timeHorizon${HORIZON_T_SUFFIX[x.key]}`)}
                                    {": "}
                                    {t(getActionTKey(x.action))}
                                  </Tag>
                                ))}
                              </div>
                            );
                          })()}
                        </div>
                      )}

                    {(() => {
                      // 完成态沿用 ✓；其余走 i18n 标签，未知值退回原始串（不吞信息）。
                      if (r.status === "completed") {
                        return <span style={{ color: "var(--green)", fontSize: 10, marginRight: 4 }}>✓</span>;
                      }
                      const key = recordStatusKey(r.status);
                      // `running` 是**进行中**、不是错误 ⇒ 用 amber，不再一律标红
                      // （红字配上「已中断」文案会让人以为分析失败，而真相是进程消失了）。
                      const color = r.status === "running"
                        ? "var(--amber)"
                        : "var(--red)";
                      return (
                        <span title={r.status} style={{ color, fontSize: 10, marginRight: 4 }}>
                          {key ? t(key) : r.status}
                        </span>
                      );
                    })()}
                    {/* 重命名按钮 */}
                    <span
                      title={t("stockAnalysis.rename")}
                      onClick={(e) => startRename(e, r)}
                      style={{
                        padding: "2px 4px",
                        borderRadius: 4,
                        cursor: "pointer",
                        color: "var(--muted, #888)",
                        display: "inline-flex",
                        alignItems: "center",
                      }}
                      onMouseEnter={(e) => {
                        (e.currentTarget as HTMLElement).style.color = "var(--accent, #7c3aed)";
                      }}
                      onMouseLeave={(e) => {
                        (e.currentTarget as HTMLElement).style.color = "var(--muted, #888)";
                      }}
                      onMouseDown={(e) => e.stopPropagation()}
                    >
                      <Pencil size={12} />
                    </span>
                    {/* 删除按钮 */}
                    <span
                      title={t("stockAnalysis.deleteAnalysis")}
                      onClick={(e) => handleDelete(e, r.id)}
                      style={{
                        padding: "2px 4px",
                        borderRadius: 4,
                        cursor: "pointer",
                        color: "var(--muted, #888)",
                        display: "inline-flex",
                        alignItems: "center",
                      }}
                      onMouseEnter={(e) => {
                        (e.currentTarget as HTMLElement).style.color = "var(--red)";
                      }}
                      onMouseLeave={(e) => {
                        (e.currentTarget as HTMLElement).style.color = "var(--muted, #888)";
                      }}
                      onMouseDown={(e) => e.stopPropagation()}
                    >
                      <Trash2 size={12} />
                    </span>
                  </div>
                ))}
              </div>
            ))}
        </div>
      )}
    >
      <Button size="small" type="text" style={{ fontSize: 12, color: "var(--muted, #888)" }}>
        <History size={13} style={{ marginRight: 4 }} />
        {t("stockAnalysis.history")}
        {records.length > 0 && <span style={{ marginLeft: 4, fontSize: 11, opacity: 0.6 }}>({records.length})</span>}
      </Button>
    </Dropdown>
  );
}
