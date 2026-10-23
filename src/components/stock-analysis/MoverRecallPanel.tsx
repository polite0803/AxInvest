import { invoke } from "@/lib/invoke";
import { horizonSuffix } from "@/lib/stock-analysis-utils";
import type {
  CloseSweepReport,
  HistoryBackfillReport,
  MarketCoverage,
  MoverRecallEventReview,
  MoverRecallReviewReflection,
  MoverRecallRunMeta,
  MoverRecallRunView,
  MoverRecallView,
  UniverseScanStats,
} from "@/types/stock-analysis";
import { Button, Card, message, Select, Space, Spin, Tag } from "antd";
import { Fragment, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";

/** 轮次下拉里「现场重算」的取值（走 `analyze_mover_recall`，不落库）。 */
const LIVE_SOURCE = "";
/** 一次取回多少轮（核查是周频 ⇒ 20 轮约 5 个月）。 */
const RUN_LIST_LIMIT = 20;

/**
 * 窗口涨幅达标漏检核查面板（`PLAN-mover-recall-attribution.md` Phase 5 + D-5 轮次读回）。
 *
 * 两种来源必须各自成句、不得互相冒充（D-5，2026-10-20 实机反馈「点了立即执行看不到结果」）：
 * - **已落库轮次**：读 `get_mover_recall_run_view`，是调度器（含任务中心「立即执行」）那一次的
 *   落库快照，面板不重算；
 * - **现场重算**：读 `analyze_mover_recall`，只出当前视图、不落库，因此它的明细行没有轮次归属。
 *
 * 四件事在 UI 上必须各自成句、不得互相冒充（「结构性缺口不得在 UI 造成歧义」）：
 * ① 不可运行（`market_daily_close` 未采集/取数失败）—— `failed` 支；
 * ② 某档判据不成立（阈值变量非正数）—— `rulesMissing` 支；
 * ③ 区间内无达标事件 —— `empty` 支；**仅当全部档位窗口已满**时才算结论，
 *    未满时以 `emptyNotYet` 说明「尚不可判定」并逐档标注（`windowOpen`）。
 * ④ 返回值形状不符（IPC 契约漂移 / mock 兜底返回 `{}`）—— 同走 `failed` 支：
 *    既不静默空白，也不让 `data.rates` 这类取值把整棵调用树带崩。
 * 另有两种「库里没有」：还没有任何已落库轮次（`runsEmpty` 支）与深链指定的轮次不在列表内
 * （`runGone` 支）—— 都不是「没有达标事件」。
 *
 * 口径边界：判据是**绝对涨幅**，不含板块涨停语义 ⇒ 本组件与 i18n 一律不得出现「涨停」字样。
 */
export function MoverRecallPanel() {
  const { t } = useTranslation();
  const [params, setSearchParams] = useSearchParams();
  const [data, setData] = useState<MoverRecallView | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [listNonce, setListNonce] = useState(0);
  const [payloadNonce, setPayloadNonce] = useState(0);
  // D-5：来源。`selectedId === null` = 轮次列表还没取回来；`""` = 现场重算；其余 = 该轮已落库结果。
  const [runs, setRuns] = useState<MoverRecallRunMeta[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [currentRun, setCurrentRun] = useState<MoverRecallRunMeta | null>(null);
  // A-2：现场重算一次拿回**两份**视图（荐股 / 智选各一），当前展示哪一条由 chain 决定。
  const [chains, setChains] = useState<MoverRecallView[]>([]);
  const [chain, setChain] = useState<string>("");
  // 深链指定的轮次不在已落库列表里 ⇒ 单独成句，不能悄悄退回重算
  const [runGone, setRunGone] = useState<string | null>(null);
  /** D-3：补分析批次的三态 —— 从未发起 / 最近一批 / 读不到（第三态不得冒充前两者）。 */
  /** D-4：回灌清单三态（无候选 / 样本不足保持 shadow / 可转正）。 */
  const [reflow, setReflow] = useState<
    { items: number; codes: string[]; settled: number; required: number; ready: boolean } | null
  >(null);
  const [batchInfo, setBatchInfo] = useState<
    { kind: "never" } | { kind: "latest"; status: string } | { kind: "unknown" } | null
  >(null);
  // 每次「刷新」只真取一次列表，避免 selectedId 变化把列表重复拉一遍
  const listFetchedFor = useRef(-1);
  // P3：漏检明细行展开 → 懒加载补分析报告 + 逐档反思。key = 三键（票×右端日×档）。
  const [expandedKeys, setExpandedKeys] = useState<ReadonlySet<string>>(new Set());
  const [reviews, setReviews] = useState<
    Record<string, { loading: boolean; error: string | null; data: MoverRecallEventReview | null }>
  >({});
  /** 阶段 0：全 A 股样本覆盖（申报总数 / 已枚举 / 最新收盘）。`null` = 取不到（不是 0）。 */
  const [coverage, setCoverage] = useState<MarketCoverage | null>(null);
  const [sampleAction, setSampleAction] = useState<"scan" | "sweep" | "history" | null>(null);

  const loadCoverage = async () => {
    try {
      const c = await invoke<unknown>("get_market_coverage", {});
      // 形状守卫：mock 兜底/契约漂移时不能让「拿不到」被读成 0 只，也不能让缺 closeDates 崩渲染
      setCoverage(isCoverage(c) ? c : null);
    } catch {
      setCoverage(null);
    }
  };

  const runSampleAction = async (
    kind: "scan" | "sweep" | "history",
    run: () => Promise<string>,
  ) => {
    setSampleAction(kind);
    try {
      message.success(await run());
      await loadCoverage();
      // 清单/收盘变了 ⇒ 轮次列表与现场视图都要重取
      setListNonce((n) => n + 1);
      setPayloadNonce((n) => n + 1);
    } catch (e) {
      message.error(detailOf(e));
    } finally {
      setSampleAction(null);
    }
  };

  const fullMarketScan = () =>
    runSampleAction("scan", async () => {
      const r = await invoke<UniverseScanStats>("run_universe_code_scan", { shards: 1 });
      return t("stockAnalysis.moverRecall.fullScanDone", {
        registered: r?.registered ?? 0,
        codes: r?.codesTried ?? 0,
      });
    });

  const sweepLatestClose = () =>
    runSampleAction("sweep", async () => {
      const r = await invoke<CloseSweepReport>("sweep_market_close", {});
      return t("stockAnalysis.moverRecall.sweepDone", {
        written: r?.written ?? 0,
        total: r?.universeSize ?? 0,
        missing: r?.missingQuotes ?? 0,
      });
    });

  const backfillHistory = () =>
    runSampleAction("history", async () => {
      const r = await invoke<HistoryBackfillReport>("backfill_market_close_history", {
        limit: 500,
      });
      // 全失败不能伪装成「补齐」：把首条原因带回给用户（500/0/500 那次就是被这里吞掉的）
      if ((r?.failed ?? 0) > 0) {
        throw new Error(r?.error ?? `backfill failed: ${r?.failed ?? 0}`);
      }
      return (r?.requested ?? 0) === 0
        ? t("stockAnalysis.moverRecall.backfillNone")
        : t("stockAnalysis.moverRecall.backfillDone", {
          codes: r?.codes ?? 0,
          rows: r?.rows ?? 0,
          failed: r?.failed ?? 0,
        });
    });

  const missKey = (stockCode: string, anchorDate: string, period: string): string =>
    `${stockCode}-${anchorDate}-${period}`;

  const toggleReview = async (key: string, stockCode: string, anchorDate: string, period: string) => {
    if (expandedKeys.has(key)) {
      setExpandedKeys((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
      return;
    }
    setExpandedKeys((prev) => new Set(prev).add(key));
    // 已加载过（含 null/失败留痕）不重复请求；失败后想重试靠面板顶部刷新整轮。
    if (reviews[key] != null) {
      return;
    }
    setReviews((prev) => ({ ...prev, [key]: { loading: true, error: null, data: null } }));
    try {
      // 局部变量别再叫 data：它会遮住同名状态，初始化里引用自己即 TDZ 报错
      const review = await invoke<MoverRecallEventReview | null>("get_mover_recall_event_review", {
        stockCode,
        anchorDate,
        period,
        // 当前视图所属链；未分链的旧轮次由后端自行回退，前端不抄一份值域判断
        chain: data?.chain ?? null,
      });
      setReviews((prev) => ({ ...prev, [key]: { loading: false, error: null, data: review ?? null } }));
    } catch (e) {
      setReviews((prev) => ({
        ...prev,
        [key]: { loading: false, error: detailOf(e), data: null },
      }));
    }
  };

  /** 形状守卫 + 落状态：两种来源共用同一条判据（缺一半载荷即走 failed 支，不猜空集）。 */
  const applyPayload = (raw: unknown, run: MoverRecallRunMeta | null) => {
    const missing = missingFields(raw);
    if (missing.length > 0) {
      setData(null);
      setCurrentRun(null);
      setFailed(t("stockAnalysis.moverRecall.malformed", { fields: missing.join(", ") }));
      return;
    }
    setData(raw as MoverRecallView);
    setCurrentRun(run);
  };

  useEffect(() => {
    if (listFetchedFor.current === listNonce) {
      return;
    }
    listFetchedFor.current = listNonce;
    let cancelled = false;
    void (async () => {
      try {
        const listed = await invoke<MoverRecallRunMeta[]>("list_mover_recall_runs", {
          limit: RUN_LIST_LIMIT,
        });
        if (cancelled) { return; }
        // mock 兜底 / 契约漂移时可能是 undefined 或 {} ⇒ 归一成空列表走现场重算支，
        // 而不是让 `runs.map` 把整棵调用树带崩（同形状守卫的动机）。
        const list = Array.isArray(listed) ? listed.filter((r) => typeof r?.id === "string") : [];
        setRuns(list);
        const urlRun = params.get("moverRun");
        if (urlRun) {
          // 深链只消费一次：用完就从 URL 摘掉。留着会让用户之后改选「现场重算」、
          // 离开再回到本页时被重新钉回那一轮（URL 是入口提示，不是状态权威）。
          const next = new URLSearchParams(params);
          next.delete("moverRun");
          setSearchParams(next, { replace: true });
        }
        if (selectedId === null) {
          // 缺省来源：深链指定的轮次 > 最近一轮已落库 > 现场重算
          if (urlRun && !list.some((r) => r.id === urlRun)) {
            setRunGone(urlRun);
            setSelectedId(LIVE_SOURCE);
          } else {
            setRunGone(null);
            setSelectedId(urlRun ?? list[0]?.id ?? LIVE_SOURCE);
          }
        } else if (selectedId !== LIVE_SOURCE && !list.some((r) => r.id === selectedId)) {
          // 已选轮次从库里消失（换库/截断）⇒ 成句说明并退回现场重算，不得静默继续显示旧载荷
          setRunGone(selectedId);
          setSelectedId(LIVE_SOURCE);
        }
      } catch {
        if (!cancelled && selectedId === null) {
          setSelectedId(LIVE_SOURCE);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [listNonce, selectedId, params, setSearchParams]);

  // D-3：补分析批次摘要与核查轮次分开取 —— 批次不按链/轮次分区，读失败也只影响这一行。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await invoke<unknown[]>("list_mover_recall_analysis_batches", { limit: 1 });
        if (cancelled) { return; }
        const ok = Array.isArray(rows)
          && rows.every((r) => typeof (r as { batchId?: unknown } | null)?.batchId === "string");
        if (!ok) {
          setBatchInfo({ kind: "unknown" });
          return;
        }
        setBatchInfo(
          rows.length === 0
            ? { kind: "never" }
            : { kind: "latest", status: String((rows[0] as { status?: unknown }).status ?? "") },
        );
      } catch {
        if (!cancelled) { setBatchInfo({ kind: "unknown" }); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [listNonce]);

  // D-4：回灌清单与核查轮次独立取 —— 它是跨轮次的，且读失败只影响这一行。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const v = await invoke<
          {
            items?: { stockCode?: string }[];
            settled?: number;
            required?: number;
            ready?: boolean;
          } | null
        >("list_mover_reflow_candidates", {});
        if (cancelled) { return; }
        if (!v || !Array.isArray(v.items) || typeof v.settled !== "number") {
          setReflow(null);
          return;
        }
        setReflow({
          items: v.items.length,
          codes: v.items.slice(0, 5).map((i) => String(i?.stockCode ?? "")).filter(Boolean),
          settled: v.settled,
          required: v.required ?? 0,
          ready: v.ready === true,
        });
      } catch {
        if (!cancelled) { setReflow(null); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [listNonce]);

  // 阶段 0：全 A 股样本覆盖（申报总数/已枚举/最新收盘）——独立取，读失败只影响这一行。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const c = await invoke<unknown>("get_market_coverage", {});
        if (!cancelled) {
          setCoverage(isCoverage(c) ? c : null);
        }
      } catch {
        if (!cancelled) { setCoverage(null); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [listNonce]);

  useEffect(() => {
    if (selectedId === null) {
      return;
    }
    let cancelled = false;
    void (async () => {
      setLoading(true);
      setFailed(null);
      try {
        if (selectedId === LIVE_SOURCE) {
          const raw = await invoke<MoverRecallView[]>("analyze_mover_recall", {});
          if (cancelled) { return; }
          // A-2：每链一份视图。数组本身就是契约的一部分 —— 拿到 undefined / {}（mock 兜底）
          // 或空数组都不许读成「核查过但没有漏检」，交给形状守卫点名缺什么。
          if (!Array.isArray(raw) || raw.length === 0) {
            // 一份旧形状（单对象）或空列表都不能被当成「核查过但没有漏检」—— 那份账连属于
            // 哪条链都判不出来。点名 `chain[]` 走同一条 malformed 句，不静默按单份渲染。
            setChains([]);
            setChain("");
            setData(null);
            setCurrentRun(null);
            setFailed(t("stockAnalysis.moverRecall.malformed", { fields: "chain[]" }));
            return;
          }
          setChains(raw);
          const keep = raw.find((v) => v?.chain === chain) ?? raw[0];
          setChain(keep.chain ?? "");
          applyPayload(keep, null);
          return;
        } else {
          const r = await invoke<MoverRecallRunView | null>("get_mover_recall_run_view", {
            runId: selectedId,
          });
          if (cancelled) { return; }
          if (r == null) {
            // 「轮次不在库内」不是执行失败 ⇒ 与「列表里没有它」同处理：单独成句 + 退回现场重算，
            // 不占用 failed 支（把缺席说成故障，与把缺席说成没有事件是同一类歧义）。
            setRunGone(selectedId);
            setData(null);
            setCurrentRun(null);
            setSelectedId(LIVE_SOURCE);
            return;
          }
          applyPayload(r, r.run ?? null);
        }
      } catch (e) {
        if (!cancelled) {
          setData(null);
          setCurrentRun(null);
          setFailed(detailOf(e));
        }
      } finally {
        if (!cancelled) { setLoading(false); }
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, payloadNonce]);

  const tierName = (period: string): string => {
    const suffix = horizonSuffix(period);
    return suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : period;
  };

  const layerName = (layer: string): string => t(`stockAnalysis.moverRecall.layer.${layer}`, { defaultValue: layer });

  const marketName = (type: string): string =>
    t(`stockAnalysis.moverRecall.marketType.${type}`, { defaultValue: type });

  /** 换链只换内存里那一份视图，不重发命令 —— 同一次核查的两链视图是同一次算出来的。 */
  const switchChain = (next: string) => {
    const hit = chains.find((v) => v.chain === next);
    if (hit) {
      setChain(hit.chain);
      applyPayload(hit, null);
    }
  };

  /** 链名目一律走 `chain_<值>` 键；后端 `CHAIN_UNLABELED` 那一档另有说明句。 */
  const chainName = (value?: string | null): string =>
    value ? t(`stockAnalysis.moverRecall.chain_${value}`, { defaultValue: value }) : "-";

  /** 逐档「有可判票」：旧轮次没有逐档可判数（null）⇒ 不阻断旧口径的「真无达标」结论。 */
  const judgeableAllPositive = data?.judgeableCodesByTier == null
    || (data?.rules.every((r) => (data?.judgeableCodesByTier?.[r.period] ?? 0) > 0) ?? false);
  const allWindowsFull = data?.rules.every((r) => (data?.collectedDays ?? 0) >= r.windowDays)
    ?? false;

  const runOptions = [
    ...runs.map((r) => ({
      value: r.id,
      label: t("stockAnalysis.moverRecall.runOption", {
        from: r.dateFrom,
        to: r.dateTo,
        chain: chainName(r.chain ?? "unlabeled"),
        events: r.eventsTotal,
        misses: r.missCount,
      }),
    })),
    { value: LIVE_SOURCE, label: t("stockAnalysis.moverRecall.runOptionLive") },
  ];

  return (
    <Card
      size="small"
      title={t("stockAnalysis.moverRecall.title")}
      extra={
        <Space size={6}>
          {chains.length > 1
            ? (
              <Select
                size="small"
                style={{ minWidth: 120 }}
                aria-label={t("stockAnalysis.moverRecall.chainSelector")}
                data-testid="mover-recall-chain-select"
                value={chain}
                options={chains.map((v) => ({ value: v.chain, label: chainName(v.chain) }))}
                onChange={(v: string) => switchChain(v)}
              />
            )
            : null}
          <Select
            size="small"
            style={{ minWidth: 220 }}
            aria-label={t("stockAnalysis.moverRecall.runSelector")}
            data-testid="mover-recall-run-select"
            value={selectedId ?? LIVE_SOURCE}
            options={runOptions}
            onChange={(v: string) => {
              setRunGone(null);
              setSelectedId(v);
            }}
          />
          <Button
            size="small"
            loading={loading}
            onClick={() => {
              setListNonce((n) => n + 1);
              setPayloadNonce((n) => n + 1);
            }}
          >
            {t("stockAnalysis.moverRecall.refresh")}
          </Button>
        </Space>
      }
      styles={{ body: { padding: "8px 10px", fontSize: 12 } }}
    >
      {/* D-5：还没有任何已落库轮次 ≠ 核查过但没有漏检，两句必须分开 */}
      {runs.length === 0
        ? (
          <div
            style={{ color: "var(--color-text-tertiary)", marginBottom: 6 }}
            data-testid="mover-recall-runs-empty"
          >
            {t("stockAnalysis.moverRecall.runsEmpty")}
          </div>
        )
        : null}

      {/* 指定的轮次不在库内（换库 / 截断 / 深链过期）⇒ 单独成句，不混进 failed */}
      {runGone
        ? (
          <div
            style={{ color: "var(--sa-warning, #faad14)", marginBottom: 6 }}
            data-testid="mover-recall-run-gone"
          >
            {t("stockAnalysis.moverRecall.runGone", { id: runGone })}
          </div>
        )
        : null}

      {/* D-4：回灌清单三态 —— 样本不足只能说 shadow，不能说「可转正」 */}
      {reflow
        ? (
          <div
            style={{ color: "var(--color-text-tertiary)", marginBottom: 6 }}
            data-testid={`mover-reflow-${reflow.items === 0 ? "none" : reflow.ready ? "ready" : "pending"}`}
          >
            {reflow.items === 0
              ? t("stockAnalysis.moverRecall.reflowNone")
              : reflow.ready
              ? t("stockAnalysis.moverRecall.reflowReady", { count: reflow.items })
              : t("stockAnalysis.moverRecall.reflowPending", {
                count: reflow.items,
                need: Math.max(0, reflow.required - reflow.settled),
              })}
            {reflow.items > 0 && reflow.codes.length > 0 ? ` · ${reflow.codes.join(", ")}` : ""}
          </div>
        )
        : null}

      {/* D-3：0 批补分析不得显示成「没有待补」—— 那是「从没跑过」，不是「都补过了」 */}
      {batchInfo
        ? (
          <div
            style={{ color: "var(--color-text-tertiary)", marginBottom: 6 }}
            data-testid={`mover-batches-${batchInfo.kind}`}
          >
            {batchInfo.kind === "never"
              ? t("stockAnalysis.moverRecall.batchesNever")
              : batchInfo.kind === "latest"
              ? t("stockAnalysis.moverRecall.batchesLatest", { status: batchInfo.status })
              : t("stockAnalysis.moverRecall.batchesUnknown")}
          </div>
        )
        : null}

      {/* 阶段 0：全 A 股样本与采集动作。四个计数域分开写，不并排成可比。 */}
      <div style={{ marginBottom: 8 }} data-testid="mover-sample">
        <div style={{ color: "var(--color-text-secondary)" }}>
          <strong>{t("stockAnalysis.moverRecall.sampleTitle")}</strong>
          {" · "}
          {coverage
            ? t("stockAnalysis.moverRecall.sampleCounts", {
              total: coverage.reportedTotal > 0 ? coverage.reportedTotal : "-",
              enumerated: coverage.universeSize,
              confirmed: coverage.confirmedByClist,
              close: coverage.latestCloseCodes,
              date: coverage.closeDates.length > 0
                ? coverage.closeDates[coverage.closeDates.length - 1]
                : "-",
            })
            : t("stockAnalysis.moverRecall.sampleCoverageUnknown")}
        </div>
        <Space size={6} wrap style={{ marginTop: 4 }}>
          <Button
            size="small"
            loading={sampleAction === "scan"}
            disabled={sampleAction !== null && sampleAction !== "scan"}
            onClick={() => void fullMarketScan()}
            title={t("stockAnalysis.moverRecall.fullScanHint")}
          >
            {t("stockAnalysis.moverRecall.fullScan")}
          </Button>
          <Button
            size="small"
            loading={sampleAction === "sweep"}
            disabled={sampleAction !== null && sampleAction !== "sweep"}
            onClick={() => void sweepLatestClose()}
            title={t("stockAnalysis.moverRecall.sweepCloseHint")}
          >
            {t("stockAnalysis.moverRecall.sweepClose")}
          </Button>
          <Button
            size="small"
            loading={sampleAction === "history"}
            disabled={sampleAction !== null && sampleAction !== "history"}
            onClick={() => void backfillHistory()}
            title={t("stockAnalysis.moverRecall.backfillHistoryHint")}
          >
            {t("stockAnalysis.moverRecall.backfillHistory")}
          </Button>
        </Space>
        <div
          style={{ color: "var(--color-text-tertiary)", fontSize: 11, marginTop: 2 }}
          data-testid="mover-sample-hints"
        >
          {t("stockAnalysis.moverRecall.fullScanHint")}
          {" · "}
          {t("stockAnalysis.moverRecall.sweepCloseHint")}
          {" · "}
          {t("stockAnalysis.moverRecall.backfillHistoryHint")}
        </div>
      </div>

      {/* A-2：本轮账属于哪条链。混池轮次（分链改造之前落的）单独成句，不冒充某一链。 */}
      {data
        ? (
          <div style={{ marginBottom: 6 }} data-testid="mover-recall-chain">
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.moverRecall.chainLabel")}
              {": "}
              <strong>{chainName(data.chain)}</strong>
            </span>
            {data.chain === "unlabeled"
              ? (
                <span
                  style={{ color: "var(--sa-warning, #faad14)", marginLeft: 6 }}
                  data-testid="mover-recall-chain-unlabeled"
                >
                  {t("stockAnalysis.moverRecall.chainUnlabeledNote")}
                </span>
              )
              : null}
          </div>
        )
        : null}

      {/* 本轮来源自报：已落库轮次（含落库时刻）还是现场重算 */}
      {data
        ? (
          <div
            style={{ color: "var(--color-text-secondary)", marginBottom: 6 }}
            data-testid="mover-recall-source"
          >
            {currentRun
              ? t("stockAnalysis.moverRecall.sourceStored", {
                id: currentRun.id,
                at: new Date(currentRun.createdAtMs).toLocaleString(),
                from: currentRun.dateFrom,
                to: currentRun.dateTo,
                events: currentRun.eventsTotal,
                misses: currentRun.missCount,
              })
              : t("stockAnalysis.moverRecall.sourceLive")}
          </div>
        )
        : null}
      {loading && !data
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : null}

      {failed
        ? (
          <div style={{ color: "var(--sa-warning, #faad14)" }} data-testid="mover-recall-failed">
            {t("stockAnalysis.moverRecall.failed", { detail: failed })}
          </div>
        )
        : null}

      {data
        ? (
          <>
            {/* ① 数据边界显式声明：起点之前未采集，不做回填推断 */}
            <div
              style={{ color: "var(--color-text-secondary)", marginBottom: 4 }}
              data-testid="mover-recall-boundary"
            >
              {t("stockAnalysis.moverRecall.boundary", {
                from: data.from,
                to: data.to,
                since: data.dataSince,
              })}
            </div>

            {/* 枚举域：确认度不足时显式声明「池外大涨股可能不在样本内」 */}
            <div style={{ marginBottom: 6 }} data-testid="mover-recall-universe">
              {data.universeConfirmed < data.universeSize
                ? t("stockAnalysis.moverRecall.universeDegraded", {
                  size: data.universeSize,
                  confirmed: data.universeConfirmed,
                })
                : t("stockAnalysis.moverRecall.universe", {
                  size: data.universeSize,
                  confirmed: data.universeConfirmed,
                })}
            </div>

            {/* 事件域 ≠ 清单域：清单里没采到收盘的票不可能产生事件；逐档再按历史长度判可判票 */}
            <div
              style={{ marginBottom: 6, fontSize: 12, color: "var(--color-text-secondary)" }}
              data-testid="mover-recall-event-domain"
            >
              {data.codesWithClose == null || data.judgeableCodesByTier == null
                ? t("stockAnalysis.moverRecall.eventDomainUnknown")
                : t("stockAnalysis.moverRecall.eventDomain", {
                  close: data.codesWithClose,
                  tiers: data.rules
                    .map((r) => `${tierName(r.period)} ${data.judgeableCodesByTier?.[r.period] ?? 0}`)
                    .join(" / "),
                })}
            </div>

            <div
              style={{ marginBottom: 6, fontSize: 11, color: "var(--color-text-tertiary)" }}
              data-testid="mover-recall-gate-version"
            >
              {data.gateVersion
                ? t("stockAnalysis.moverRecall.gateVersion", { version: data.gateVersion })
                : t("stockAnalysis.moverRecall.gateVersionUnknown")}
            </div>

            {/* 三个率：null 显示「样本不足，算不出」，绝不显示成 0% */}
            <div style={{ display: "flex", gap: 24, flexWrap: "wrap", margin: "10px 0" }}>
              <RateStat
                label={t("stockAnalysis.moverRecall.rateReachability")}
                value={data.rates.reachability}
              />
              <RateStat
                label={t("stockAnalysis.moverRecall.rateCoverage")}
                value={data.rates.coverage}
              />
              <RateStat
                label={t("stockAnalysis.moverRecall.rateUnexplained")}
                value={data.rates.unexplainedShare}
              />
              <div
                style={{
                  fontSize: 11,
                  color: "var(--color-text-tertiary)",
                  alignSelf: "flex-end",
                }}
              >
                {data.rates.judgeableEvents == null
                  ? t("stockAnalysis.moverRecall.events", {
                    events: data.rates.events,
                    misses: data.rates.misses,
                  })
                  : t("stockAnalysis.moverRecall.eventsWithJudgeable", {
                    events: data.rates.events,
                    misses: data.rates.misses,
                    judgeable: data.rates.judgeableEvents,
                  })}
              </div>
            </div>

            {data.rates.events === 0
              ? (
                <div
                  style={{ color: "var(--color-text-tertiary)" }}
                  data-testid="mover-recall-empty"
                >
                  {/* C-2 三态：全满 ⇒ 结论；全未满 ⇒ 尚不可判定；部分满 ⇒ 点名未满档 */}
                  {allWindowsFull && !judgeableAllPositive
                    ? t("stockAnalysis.moverRecall.emptyNoJudgeable")
                    : allWindowsFull
                    ? t("stockAnalysis.moverRecall.empty")
                    : data.rules.every((r) => data.collectedDays < r.windowDays)
                    ? t("stockAnalysis.moverRecall.emptyNotYet", { have: data.collectedDays })
                    : t("stockAnalysis.moverRecall.emptyNotYetTiers", {
                      have: data.collectedDays,
                      tiers: data.rules
                        .filter((r) => data.collectedDays < r.windowDays)
                        .map((r) => tierName(r.period))
                        .join(" / "),
                    })}
                </div>
              )
              : null}

            {/* ② 四档判据：阈值来源（模板变量名）逐档点名 */}
            <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.rulesTitle")}
            </div>
            <ul style={{ margin: "2px 0 8px", paddingLeft: 18 }}>
              {data.rules.map((r) => (
                <li key={r.period} data-testid={`mover-rule-${r.period}`}>
                  {t("stockAnalysis.moverRecall.rule", {
                    tier: tierName(r.period),
                    gain: r.gainPct,
                    days: r.windowDays,
                    var: r.varName,
                  })}
                  {data.collectedDays < r.windowDays
                    ? (
                      <span
                        style={{ color: "var(--sa-warning, #faad14)", marginLeft: 6 }}
                        data-testid={`mover-window-open-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.windowOpen", {
                          have: data.collectedDays,
                          need: r.windowDays,
                        })}
                      </span>
                    )
                    : data.eventsByTier == null
                    ? (
                      <span
                        style={{ color: "var(--color-text-tertiary)", marginLeft: 6 }}
                        data-testid={`mover-tier-count-missing-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.tierCountMissing")}
                      </span>
                    )
                    : (data.eventsByTier[r.period] ?? 0) === 0
                    ? (
                      <span
                        style={{ color: "var(--color-text-tertiary)", marginLeft: 6 }}
                        data-testid={`mover-tier-no-events-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.tierNoEvents")}
                      </span>
                    )
                    : (
                      <span
                        style={{ color: "var(--color-text-tertiary)", marginLeft: 6 }}
                        data-testid={`mover-tier-events-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.tierEvents", {
                          count: data.eventsByTier[r.period],
                        })}
                      </span>
                    )}
                  {data.codesWithClose != null
                      && data.judgeableCodesByTier != null
                      && (data.judgeableCodesByTier[r.period] ?? 0) < data.codesWithClose
                    ? (
                      <span
                        style={{ color: "var(--sa-warning, #faad14)", marginLeft: 6 }}
                        data-testid={`mover-tier-judgeable-${r.period}`}
                      >
                        {t("stockAnalysis.moverRecall.tierJudgeable", {
                          judgeable: data.judgeableCodesByTier[r.period] ?? 0,
                          close: data.codesWithClose,
                        })}
                      </span>
                    )
                    : null}
                </li>
              ))}
            </ul>
            {data.rules.length < 4
              ? (
                <div
                  style={{ color: "var(--sa-warning, #faad14)", marginBottom: 6 }}
                  data-testid="mover-recall-rules-missing"
                >
                  {t("stockAnalysis.moverRecall.rulesMissing", { count: 4 - data.rules.length })}
                </div>
              )
              : null}

            {/* 归因分层：每层计数 + 按板块分组（避免混池假象） */}
            <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.layersTitle")}
            </div>
            <table style={{ width: "100%", borderCollapse: "collapse", marginBottom: 8 }}>
              <tbody>
                {data.layers.map((row) => (
                  <tr
                    key={row.layer}
                    style={{ borderBottom: "0.5px solid var(--color-border-tertiary)" }}
                  >
                    <td style={{ padding: "3px 0" }}>{layerName(row.layer)}</td>
                    <td style={{ padding: "3px 0", textAlign: "right", fontWeight: 500 }}>
                      {row.count}
                    </td>
                    <td
                      style={{
                        padding: "3px 0 3px 12px",
                        fontSize: 11,
                        color: "var(--color-text-tertiary)",
                      }}
                    >
                      {row.byMarketType
                        .map(([mt, n]) => `${marketName(mt)} ${n}`)
                        .join(" · ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            {/* 漏检明细（有留痕的才可归因；截断留痕风格逐条点名） */}
            {data.misses.length > 0
              ? (
                <>
                  <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>
                    {t("stockAnalysis.moverRecall.missesTitle")}
                  </div>
                  <div style={{ maxHeight: 260, overflowY: "auto" }}>
                    <table
                      style={{ width: "100%", borderCollapse: "collapse", fontSize: 11 }}
                      data-testid="mover-miss-table"
                    >
                      <thead>
                        <tr style={{ color: "var(--color-text-tertiary)" }}>
                          <th style={{ textAlign: "left", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colName")}
                          </th>
                          <th style={{ textAlign: "center", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colTier")}
                          </th>
                          <th style={{ textAlign: "right", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colCum")}
                          </th>
                          <th style={{ textAlign: "right", padding: "3px 0" }}>
                            {t("stockAnalysis.moverRecall.colMaxDaily")}
                          </th>
                          <th style={{ textAlign: "left", padding: "3px 0 3px 10px" }}>
                            {t("stockAnalysis.moverRecall.colLayer")}
                          </th>
                          <th style={{ width: 40 }} />
                        </tr>
                      </thead>
                      <tbody>
                        {data.misses.map((m) => {
                          const key = missKey(m.event.stockCode, m.event.anchorDate, m.event.period);
                          const expanded = expandedKeys.has(key);
                          const reviewState = reviews[key];
                          return (
                            <Fragment key={key}>
                              <tr>
                                <td style={{ padding: "3px 0" }}>
                                  {`${m.event.stockName} ${m.event.stockCode}`}
                                </td>
                                <td style={{ textAlign: "center", padding: "3px 0" }}>
                                  {tierName(m.event.period)}
                                </td>
                                <td
                                  style={{
                                    textAlign: "right",
                                    padding: "3px 0",
                                    color: "var(--sa-red)",
                                  }}
                                >
                                  {`+${m.event.cumGainPct.toFixed(1)}%`}
                                </td>
                                <td style={{ textAlign: "right", padding: "3px 0" }}>
                                  {`+${m.event.maxDailyPct.toFixed(1)}%`}
                                </td>
                                <td style={{ padding: "3px 0 3px 10px" }}>
                                  {layerName(m.layer)}
                                  {m.scoredOutStyles.length > 0
                                    ? (
                                      <div
                                        style={{ fontSize: 10, color: "var(--color-text-tertiary)" }}
                                        data-testid="mover-scored-out"
                                      >
                                        {t("stockAnalysis.moverRecall.scoredOut", {
                                          styles: m.scoredOutStyles.join("、"),
                                        })}
                                      </div>
                                    )
                                    : null}
                                </td>
                                <td style={{ textAlign: "center", padding: "3px 0" }}>
                                  <Button
                                    size="small"
                                    type="text"
                                    aria-label={t("stockAnalysis.moverRecall.review.toggle")}
                                    data-testid={`mover-review-toggle-${key}`}
                                    onClick={() =>
                                      void toggleReview(
                                        key,
                                        m.event.stockCode,
                                        m.event.anchorDate,
                                        m.event.period,
                                      )}
                                  >
                                    {expanded ? "−" : "+"}
                                  </Button>
                                </td>
                              </tr>
                              {expanded
                                ? (
                                  <tr key={`${key}-detail`}>
                                    <td colSpan={6} style={{ padding: "6px 8px 10px" }}>
                                      <MoverReviewDetail
                                        state={reviewState ?? { loading: true, error: null, data: null }}
                                        tierName={tierName}
                                        layerName={layerName}
                                      />
                                    </td>
                                  </tr>
                                )
                                : null}
                            </Fragment>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                </>
              )
              : null}

            {/* 说明段：三种「非算法缺陷」的口径边界，逐条成句 */}
            <div
              style={{
                marginTop: 8,
                fontSize: 10,
                color: "var(--color-text-tertiary)",
                lineHeight: 1.7,
              }}
            >
              <div>{t("stockAnalysis.moverRecall.noteByDesign")}</div>
              <div>{t("stockAnalysis.moverRecall.notePoolDegraded")}</div>
              <div>{t("stockAnalysis.moverRecall.noteShadow")}</div>
            </div>
          </>
        )
        : null}
    </Card>
  );
}

/** 一块率读数：`null` = 分母为 0（算不出）≠ 0%。 */
function RateStat({ label, value }: { label: string; value: number | null }) {
  const { t } = useTranslation();
  return (
    <div style={{ minWidth: 130 }}>
      <div style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>{label}</div>
      <div
        style={{
          fontSize: 16,
          fontWeight: 500,
          color: value == null ? "var(--color-text-tertiary)" : undefined,
        }}
      >
        {value == null
          ? t("stockAnalysis.moverRecall.rateUnavailable")
          : `${value.toFixed(1)}%`}
      </div>
    </div>
  );
}

/** 面板渲染所必需的顶层标量字段（`null` 视同缺）。 */
const REQUIRED_SCALARS = ["from", "to", "dataSince", "collectedDays", "rates"] as const;
/** 面板要逐行遍历的顶层数组字段。 */
const REQUIRED_ARRAYS = ["rules", "layers", "misses"] as const;

/**
 * 形状守卫：返回缺失/类型不符的顶层字段名（空数组 ⇒ 形状可用，交由 TS 的 `MoverRecallView` 断言）。
 *
 * 为什么需要它：`invoke<T>` 的 `T` 只是编译期断言，管不住后端实际返回了什么；
 * 浏览器 mock 的 default 兜底与测试里的通用 `mockResolvedValue` 都会给出 `{}` / `[]`，
 * 此时若直接把载荷当视图渲染，`data.rates.reachability` 会把整棵调用树带崩（矩阵页白屏）。
 */
function missingFields(r: unknown): string[] {
  if (r == null || typeof r !== "object") {
    return [...REQUIRED_SCALARS, ...REQUIRED_ARRAYS];
  }
  const o = r as Record<string, unknown>;
  const missing: string[] = REQUIRED_SCALARS.filter((k) => o[k] == null);
  for (const k of REQUIRED_ARRAYS) {
    if (!Array.isArray(o[k])) { missing.push(k); }
  }
  return missing;
}

/** `get_market_coverage` 的形状守卫：缺计数/closeDates 即视为「取不到」，不渲染成 0。 */
function isCoverage(c: unknown): c is MarketCoverage {
  if (c == null || typeof c !== "object") {
    return false;
  }
  const o = c as Record<string, unknown>;
  return typeof o.universeSize === "number"
    && typeof o.confirmedByClist === "number"
    && typeof o.reportedTotal === "number"
    && typeof o.latestCloseCodes === "number"
    && Array.isArray(o.closeDates);
}

/** 后端错误按 `detail` 成句呈现（已是完整说明），非 JSON 时退回消息原文。 */
function detailOf(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  try {
    const parsed = JSON.parse(msg) as { code?: unknown; detail?: unknown };
    if (typeof parsed.detail === "string" && parsed.detail.length > 0) { return parsed.detail; }
    if (typeof parsed.code === "string") { return parsed.code; }
  } catch { /* 非 JSON ⇒ 原文 */ }
  return msg;
}

// ── P3：漏检行展开详情（补分析报告 + 逐档反思，懒加载自 `get_mover_recall_event_review`）──

type ReviewState = {
  loading: boolean;
  error: string | null;
  data: MoverRecallEventReview | null;
};

/** 反思卡固定档位顺序（后端按字母序返回，前端重排为超短→长）。 */
const REVIEW_HORIZON_ORDER = ["ultra_short", "short", "mid", "long"];

const EVENT_STATUS_COLOR: Record<string, string> = {
  pending: "default",
  excluded: "default",
  running: "processing",
  analyzed: "blue",
  reflected: "green",
  failed: "error",
};

const VERDICT_COLOR: Record<string, string> = {
  correct: "green",
  partial: "gold",
  wrong: "red",
};

/** 展开区：加载/错误/未落库/事件四态 + 分析摘要与逐档反思卡。 */
function MoverReviewDetail({
  state,
  tierName,
  layerName,
}: {
  state: ReviewState;
  tierName: (code: string) => string;
  layerName: (code: string) => string;
}) {
  const { t } = useTranslation();

  if (state.loading) {
    return (
      <div style={{ display: "flex", gap: 8, alignItems: "center" }} data-testid="mover-review-loading">
        <Spin size="small" />
        <span style={{ fontSize: 12 }}>{t("stockAnalysis.moverRecall.review.loading")}</span>
      </div>
    );
  }
  if (state.error != null) {
    return (
      <div className="sa-failed" style={{ fontSize: 12 }} data-testid="mover-review-error">
        {t("stockAnalysis.moverRecall.review.loadError", { detail: state.error })}
      </div>
    );
  }
  const r = state.data;
  // 实时核查视图的 misses 不落库；命令按三键查不到事件 ⇒ 提示而非留白。
  if (r == null) {
    return (
      <div style={{ fontSize: 12, color: "var(--color-text-tertiary)" }} data-testid="mover-review-not-tracked">
        {t("stockAnalysis.moverRecall.review.notTracked")}
      </div>
    );
  }

  const statusColor = EVENT_STATUS_COLOR[r.status] ?? "default";
  const sortedReflections = [...r.reflections].sort((a, b) => {
    const ia = REVIEW_HORIZON_ORDER.indexOf(a.horizon ?? "");
    const ib = REVIEW_HORIZON_ORDER.indexOf(b.horizon ?? "");
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
  const showAnalysisBlock = r.status === "analyzed" || r.status === "reflected";
  // 补分析整行是 Model 的 serde 直出（camelCase），宽松取值；缺字段不渲染该行。
  const a = r.analysis;
  const analysisDate = a == null
    ? null
    : (typeof a.asOfDate === "string" ? a.asOfDate : null)
      ?? (typeof a.analysisDate === "string" ? a.analysisDate : null);
  const decisionAction = a == null ? null : typeof a.decisionAction === "string" ? a.decisionAction : null;
  const decisionReasoning = a == null
    ? null
    : typeof a.decisionReasoning === "string"
    ? a.decisionReasoning
    : null;

  return (
    <div
      data-testid="mover-review-detail"
      style={{
        background: "var(--color-bg-secondary, rgba(127,127,127,0.06))",
        border: "1px solid var(--color-border-secondary, rgba(127,127,127,0.2))",
        borderRadius: 6,
        padding: "8px 10px",
        fontSize: 12,
      }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <Tag color={statusColor}>{t(`stockAnalysis.moverRecall.review.status_${r.status}`)}</Tag>
        {r.missLayer
          ? (
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.moverRecall.review.layerHint", { layer: layerName(r.missLayer) })}
            </span>
          )
          : null}
        <span style={{ color: "var(--color-text-tertiary)" }}>
          {t("stockAnalysis.moverRecall.review.windowFacts", {
            start: r.windowStart,
            anchor: r.anchorDate,
            days: r.windowDays,
            gain: r.cumGainPct.toFixed(1),
            max: r.maxDailyPct.toFixed(1),
          })}
        </span>
      </div>

      {r.status === "failed"
        ? (
          <div className="sa-failed" style={{ marginTop: 6 }} data-testid="mover-review-failed">
            {t("stockAnalysis.moverRecall.review.errorDetail", { detail: r.errorDetail ?? "-" })}
          </div>
        )
        : null}
      {(r.status === "pending" || r.status === "excluded" || r.status === "running") && r.errorDetail
        ? (
          <div style={{ marginTop: 6, color: "var(--color-text-tertiary)" }}>
            {t("stockAnalysis.moverRecall.review.errorDetail", { detail: r.errorDetail })}
          </div>
        )
        : null}

      {showAnalysisBlock
        ? (
          <div style={{ marginTop: 8 }} data-testid="mover-review-analysis">
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {analysisDate
                ? t("stockAnalysis.moverRecall.review.analysisHeading", { date: analysisDate })
                : t("stockAnalysis.moverRecall.review.analysisHeading", { date: "-" })}
            </div>
            {a == null
              ? (
                <div style={{ color: "var(--color-text-tertiary)" }}>
                  {t("stockAnalysis.moverRecall.review.analysisMissing")}
                </div>
              )
              : (
                <>
                  {decisionAction
                    ? (
                      <div>
                        <strong>
                          {t("stockAnalysis.moverRecall.review.decisionLabel", { action: decisionAction })}
                        </strong>
                      </div>
                    )
                    : null}
                  {decisionReasoning
                    ? (
                      <div
                        style={{
                          marginTop: 2,
                          color: "var(--color-text-secondary)",
                          display: "-webkit-box",
                          WebkitLineClamp: 4,
                          WebkitBoxOrient: "vertical",
                          overflow: "hidden",
                        }}
                        title={decisionReasoning}
                      >
                        {decisionReasoning}
                      </div>
                    )
                    : null}
                </>
              )}
          </div>
        )
        : null}

      {showAnalysisBlock
        ? (
          <div style={{ marginTop: 8 }} data-testid="mover-review-reflections">
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {t("stockAnalysis.moverRecall.review.reflectionHeading")}
            </div>
            {sortedReflections.length === 0
              ? (
                <div style={{ color: "var(--color-text-tertiary)" }}>
                  {t("stockAnalysis.moverRecall.review.noReflection")}
                </div>
              )
              : (
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {sortedReflections.map((rf) => <ReflectionCard key={rf.id} rf={rf} tierName={tierName} />)}
                </div>
              )}
          </div>
        )
        : null}
    </div>
  );
}

/** 单档反思卡：completed 展示裁决 + 三问 + 收益；pending/running/failed 只出状态。 */
function ReflectionCard({
  rf,
  tierName,
}: {
  rf: MoverRecallReviewReflection;
  tierName: (code: string) => string;
}) {
  const { t } = useTranslation();

  const horizonLabel = rf.horizon ? tierName(rf.horizon) : "-";
  // status 失败行落库形如 "failed: <reason>"。
  const failedMatch = /^failed(?::\s?(.*))?$/.exec(rf.status);
  const signals = parseSignals(rf.missedSignals);

  return (
    <div
      data-testid={`mover-review-rf-${rf.horizon ?? "unknown"}`}
      style={{
        border: "1px solid var(--color-border-secondary, rgba(127,127,127,0.2))",
        borderRadius: 4,
        padding: "6px 8px",
      }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <strong>{horizonLabel}</strong>
        {rf.status === "completed" && rf.verdict
          ? (
            <Tag color={VERDICT_COLOR[rf.verdict] ?? "default"}>
              {t("stockAnalysis.moverRecall.review.verdictLabel")}
              {": "}
              {t(`stockAnalysis.moverRecall.review.verdict_${rf.verdict}`)}
            </Tag>
          )
          : null}
        {rf.status === "completed" && rf.rawReturn != null
          ? (
            <span style={{ color: "var(--color-text-tertiary)" }}>
              {t("stockAnalysis.moverRecall.review.rawReturn")}
              {": "}
              {`${rf.rawReturn >= 0 ? "+" : ""}${rf.rawReturn.toFixed(1)}%`}
            </span>
          )
          : null}
        {rf.status === "pending"
          ? <Tag>{t("stockAnalysis.moverRecall.review.rfPending")}</Tag>
          : null}
        {rf.status === "running"
          ? <Tag color="processing">{t("stockAnalysis.moverRecall.review.rfRunning")}</Tag>
          : null}
        {failedMatch
          ? (
            <Tag color="error">
              {t("stockAnalysis.moverRecall.review.rfFailed", {
                detail: failedMatch[1] || "-",
              })}
            </Tag>
          )
          : null}
      </div>

      {rf.status === "completed"
        ? (
          <div style={{ marginTop: 4, display: "flex", flexDirection: "column", gap: 3 }}>
            {rf.lessonSummary
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.lessonSummary")}</strong>
                  {": "}
                  {rf.lessonSummary}
                </div>
              )
              : null}
            {signals.length > 0
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.missedSignals")}</strong>
                  {": "}
                  <ul style={{ margin: "2px 0 0 18px", padding: 0 }}>
                    {signals.map((s, i) => <li key={i}>{s}</li>)}
                  </ul>
                </div>
              )
              : null}
            {rf.whatWentWrong
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.whatWentWrong")}</strong>
                  {": "}
                  {rf.whatWentWrong}
                </div>
              )
              : null}
            {rf.fixForFuture
              ? (
                <div>
                  <strong>{t("stockAnalysis.moverRecall.review.fixForFuture")}</strong>
                  {": "}
                  {rf.fixForFuture}
                </div>
              )
              : null}
          </div>
        )
        : null}
    </div>
  );
}

/** missed_signals 落库为 JSON 数组字符串；非数组/解析失败时按单条原文展示（不丢信息）。 */
function parseSignals(raw: string | null): string[] {
  if (raw == null || raw.trim().length === 0) { return []; }
  try {
    const v: unknown = JSON.parse(raw);
    if (Array.isArray(v) && v.every((x) => typeof x === "string")) { return v as string[]; }
  } catch { /* 非 JSON ⇒ 原文 */ }
  return [raw];
}
