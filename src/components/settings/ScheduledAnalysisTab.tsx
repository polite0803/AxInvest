import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import { App, Button, Card, Empty, Form, Input, InputNumber, Popconfirm, Select, Spin, Switch, Table, Tag } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface CronJobRow {
  id: string;
  name: string;
  description: string;
  schedule: string;
  status: string;
  /** 关联工作流模板 ID（`null` = 走 task_type 专用分支，不跑工作流）。
   *  趋势智选两条链共用一个 task_type，只能靠本字段把列表项归链。 */
  workflowId: string | null;
  runCount: number;
  lastRunAt: number | null;
}

function getCronPresets(t: (k: string) => string) {
  return [
    { label: t("stockAnalysis.scheduledAnalysis.cron.daily9am"), value: "0 9 * * *" },
    { label: t("stockAnalysis.scheduledAnalysis.cron.daily1530"), value: "30 15 * * *" },
    { label: t("stockAnalysis.scheduledAnalysis.cron.weeklyMon"), value: "0 9 * * 1" },
    { label: t("stockAnalysis.scheduledAnalysis.cron.hourly"), value: "0 * * * *" },
  ];
}

/**
 * 趋势智选的两条链 —— 定时任务按 `workflowId` 归链。
 * `workflowId` 必须与后端 `recommendation_cron.rs` 的
 * `TREND_SCREENING_WORKFLOW_IDS` 白名单逐字一致（后端不认的 id 会直接报错）。
 * 链名复用趋势智选面板的两个按钮文案，不另建 key（同一概念同一文案）。
 */
const TREND_CHAINS = [
  { id: "serenity-screening", labelKey: "serenityPanel.run" },
  { id: "serenity-screening-fast", labelKey: "serenityPanel.fastRun" },
] as const;

type TrendChainId = (typeof TREND_CHAINS)[number]["id"];

/**
 * 决策回测的 T+N 验证窗口候选（与后端 `RunDecisionBacktestRequest` 默认值
 * `[5, 20, 60]` 一致：短/中/长三个持有窗口）。
 */
const BT_WINDOWS = [5, 20, 60];

export function ScheduledAnalysisTab() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const [jobs, setJobs] = useState<CronJobRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [form] = Form.useForm();

  // 自选股自动扫描
  const [wlScanJobs, setWlScanJobs] = useState<CronJobRow[]>([]);
  const [wlCron, setWlCron] = useState("0 21 * * 1-5");

  // 候选池逐只分析（task_type = pool-scan）
  // 与「自选股扫描」的区别：数据源是 reco_picks 候选池（荐股/趋势智选的产物），
  // 而非 watchlist_items。没有它，候选池里的股票永远不会被自动分析。
  const [poolJobs, setPoolJobs] = useState<CronJobRow[]>([]);
  const [poolCron, setPoolCron] = useState("0 17 * * 1-5");

  // 趋势智选定时筛选（task_type = trend-screening，两链共用该标记，靠 workflowId 归链）
  // 产物写入 reco_picks（style='serenity'），与智能荐股结果共同构成候选池。
  // 两链的节奏诉求不同（原链 12 个 Agent 腿、慢但覆盖全；快速链确定性简报 + 单 Agent），
  // 故各自独立开关与独立 cron —— 合并成一个开关就会让「只跑快速链」无法表达。
  const [trendJobs, setTrendJobs] = useState<CronJobRow[]>([]);
  const [trendCrons, setTrendCrons] = useState<Record<string, string>>({
    "serenity-screening": "0 16 * * 1-5",
    "serenity-screening-fast": "0 16 * * 1-5",
  });

  // 4 周期反思（task_type = batch-reflection）
  // 每个档位一个任务：超短线 2 天 / 短线 5 天 / 中线 28 天 / 长线 90 天
  const [reflJobs, setReflJobs] = useState<CronJobRow[]>([]);
  const [reflCron, setReflCron] = useState("0 18 * * *");
  const [reflPeriod, setReflPeriod] = useState<string>("short");

  // 决策回测（task_type = decision-backtest）
  // 回放 reco_picks 历史荐股 → 拉 T+N 窗口真实 K 线 → 写 decision_validations
  // → 回写 stock_analyses.outcome。是「决策可采信度」唯一的量化入口。
  //
  // 默认 07:00：**晚于**反思族的 06:00、**早于**收市。T+N 验证只读历史 K 线，
  // 早跑不会拿到不完整数据；与 pool-scan(17:00)/trend-screening(16:00) 错开，
  // 避免同一时段抢 astock 供应商配额。
  const [btJobs, setBtJobs] = useState<CronJobRow[]>([]);
  const [btCron, setBtCron] = useState("0 7 * * *");
  const [btWindows, setBtWindows] = useState<number[]>([5, 20, 60]);
  const [btMaxPicks, setBtMaxPicks] = useState(200);
  const [btPeriod, setBtPeriod] = useState<string>("all");

  const loadPool = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_pool_scan_crons");
      if (Array.isArray(list)) { setPoolJobs(list); }
    } catch { /* backend not running */ }
  };

  const loadRefl = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_batch_reflection_crons");
      if (Array.isArray(list)) { setReflJobs(list); }
    } catch { /* backend not running */ }
  };

  const loadTrend = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_trend_screening_crons");
      if (Array.isArray(list)) { setTrendJobs(list); }
    } catch { /* backend not running */ }
  };

  const toggleTrend = async (chainId: TrendChainId, job: CronJobRow | null, enable: boolean) => {
    try {
      if (enable && !job) {
        await invoke("create_trend_screening_cron", {
          cronExpression: trendCrons[chainId],
          enabled: true,
          workflowId: chainId,
        });
        message.success(t("stockAnalysis.scheduledAnalysis.scanStarted"));
      } else if (!enable && job) {
        await invoke("toggle_trend_screening_cron", { id: job.id, enabled: false });
        message.success(t("stockAnalysis.scheduledAnalysis.scanPaused"));
      }
      loadTrend();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.operationFailed"));
    }
  };

  const deleteTrend = async (id: string) => {
    try {
      await invoke("delete_trend_screening_cron", { id });
      loadTrend();
    } catch { /* silent */ }
  };

  const togglePool = async (job: CronJobRow | null, enable: boolean) => {
    try {
      if (enable && !job) {
        await invoke("create_pool_scan_cron", { cronExpression: poolCron, enabled: true });
        message.success(t("stockAnalysis.scheduledAnalysis.scanStarted"));
      } else if (!enable && job) {
        await invoke("toggle_pool_scan_cron", { id: job.id, enabled: false });
        message.success(t("stockAnalysis.scheduledAnalysis.scanPaused"));
      }
      loadPool();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.operationFailed"));
    }
  };

  const deletePool = async (id: string) => {
    try {
      await invoke("delete_pool_scan_cron", { id });
      loadPool();
    } catch { /* silent */ }
  };

  const createRefl = async () => {
    try {
      await invoke("create_batch_reflection_cron", {
        cronExpression: reflCron,
        period: reflPeriod,
        dueOnly: false,
        enabled: true,
      });
      message.success(t("stockAnalysis.scheduledAnalysis.taskCreated"));
      loadRefl();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.createFailed"));
    }
  };

  const toggleRefl = async (job: CronJobRow, enable: boolean) => {
    try {
      await invoke("toggle_batch_reflection_cron", { id: job.id, enabled: enable });
      loadRefl();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.operationFailed"));
    }
  };

  const deleteRefl = async (id: string) => {
    try {
      await invoke("delete_batch_reflection_cron", { id });
      loadRefl();
    } catch { /* silent */ }
  };

  const loadBt = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_decision_backtest_crons");
      if (Array.isArray(list)) { setBtJobs(list); }
    } catch { /* backend not running */ }
  };

  const createBt = async () => {
    try {
      await invoke("create_decision_backtest_cron", {
        cronExpression: btCron,
        // 「全部周期」必须传 null（后端是 Option<String> 的 None）。
        // 传空串不是 None，会被当成一个不存在的周期名 ⇒ 一条 pick 都回测不到，
        // 而任务仍报「成功」（零产物），属于最隐蔽的静默失败。
        periodFilter: btPeriod === "all" ? null : btPeriod,
        tPlusNList: btWindows,
        maxPicks: btMaxPicks,
        enabled: true,
      });
      message.success(t("stockAnalysis.scheduledAnalysis.taskCreated"));
      loadBt();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.createFailed"));
    }
  };

  const toggleBt = async (job: CronJobRow, enable: boolean) => {
    try {
      await invoke("toggle_decision_backtest_cron", { id: job.id, enabled: enable });
      loadBt();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.operationFailed"));
    }
  };

  const deleteBt = async (id: string) => {
    try {
      await invoke("delete_decision_backtest_cron", { id });
      loadBt();
    } catch { /* silent */ }
  };

  const loadWlScan = async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_watchlist_scan_crons");
      if (Array.isArray(list)) { setWlScanJobs(list); }
    } catch { /* backend not running */ }
  };

  const loadTasks = useCallback(async () => {
    try {
      const list = await invoke<CronJobRow[]>("list_stock_crons");
      if (Array.isArray(list)) { setJobs(list); }
    } catch { /* backend not running */ }
  }, []);

  const toggleWlScan = async (job: CronJobRow | null, enable: boolean) => {
    try {
      if (enable && !job) {
        await invoke("create_watchlist_scan_cron", {
          cronExpression: wlCron,
          enabled: true,
        });
        message.success(t("stockAnalysis.scheduledAnalysis.scanStarted"));
      } else if (!enable && job) {
        await invoke("toggle_watchlist_scan_cron", { id: job.id, enabled: false });
        message.success(t("stockAnalysis.scheduledAnalysis.scanPaused"));
      }
      loadWlScan();
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.operationFailed"));
    }
  };

  const deleteWlScan = async (id: string) => {
    try {
      await invoke("delete_watchlist_scan_cron", { id });
      loadWlScan();
    } catch { /* silent */ }
  };

  useEffect(() => {
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      setLoading(true);
      return invoke<CronJobRow[]>("list_stock_crons");
    })
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setJobs(list); }
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    invoke<CronJobRow[]>("list_watchlist_scan_crons")
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setWlScanJobs(list); }
      })
      .catch(() => {});
    invoke<CronJobRow[]>("list_pool_scan_crons")
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setPoolJobs(list); }
      })
      .catch(() => {});
    invoke<CronJobRow[]>("list_batch_reflection_crons")
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setReflJobs(list); }
      })
      .catch(() => {});
    invoke<CronJobRow[]>("list_trend_screening_crons")
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setTrendJobs(list); }
      })
      .catch(() => {});
    invoke<CronJobRow[]>("list_decision_backtest_crons")
      .then((list) => {
        if (cancelled) { return; }
        if (Array.isArray(list)) { setBtJobs(list); }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const create = async (values: Record<string, unknown>) => {
    try {
      await invoke("create_stock_cron", {
        stockCode: values.stockCode,
        stockName: values.stockName,
        cronExpression: values.cronExpression,
      });
      form.resetFields();
      setAdding(false);
      loadTasks();
      message.success(t("stockAnalysis.scheduledAnalysis.taskCreated"));
    } catch {
      message.error(t("stockAnalysis.scheduledAnalysis.createFailed"));
    }
  };

  const toggle = useCallback(async (id: string, active: boolean) => {
    try {
      await invoke("toggle_stock_cron", { id, enabled: active });
      loadTasks();
    } catch { /* silent */ }
  }, [loadTasks]);

  const remove = useCallback(async (id: string) => {
    try {
      await invoke("delete_stock_cron", { id });
      loadTasks();
    } catch { /* silent */ }
  }, [loadTasks]);

  const columns = useMemo(() => [
    {
      title: t("stockAnalysis.scheduledAnalysis.column.id"),
      dataIndex: "name",
      ellipsis: true,
      width: 100,
      render: (v: string) => <span className="text-xs font-mono">{v}</span>,
    },
    { title: t("stockAnalysis.scheduledAnalysis.column.description"), dataIndex: "description", ellipsis: true },
    {
      title: "Cron",
      dataIndex: "schedule",
      width: 120,
      render: (v: string) => <Tag className="text-xs m-0 font-mono">{v}</Tag>,
    },
    {
      title: t("stockAnalysis.scheduledAnalysis.column.executed"),
      dataIndex: "runCount",
      width: 50,
      align: "center" as const,
      render: (v: number) => <span className="text-xs">{v}</span>,
    },
    {
      title: t("stockAnalysis.scheduledAnalysis.column.status"),
      dataIndex: "status",
      width: 60,
      render: (v: string, record: CronJobRow) => (
        <Switch
          size="small"
          checked={v === "active"}
          onChange={(checked) => toggle(record.id, checked)}
        />
      ),
    },
    {
      title: "",
      key: "action",
      width: 30,
      render: (_: unknown, record: CronJobRow) => (
        <Popconfirm
          title={t("stockAnalysis.scheduledAnalysis.confirmDeleteTitle")}
          onConfirm={() => remove(record.id)}
        >
          <Button size="small" type="text" danger icon={<DeleteOutlined />} />
        </Popconfirm>
      ),
    },
  ], [remove, toggle, t]);

  if (loading) { return <Spin size="small" />; }

  return (
    <div className="flex flex-col gap-3">
      <div className="text-sm text-gray-500">
        {t("stockAnalysis.scheduledAnalysis.description")}
      </div>

      {/* watchlist auto scan */}
      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.watchlistScan")}
        styles={{ body: { padding: "8px 12px" } }}
      >
        <div className="flex items-center gap-3">
          <Switch
            checked={wlScanJobs.some((j) => j.status === "active")}
            onChange={(checked) => toggleWlScan(wlScanJobs.length > 0 ? wlScanJobs[0] : null, checked)}
          />
          <span className="text-xs text-gray-400">
            {wlScanJobs.some((j) => j.status === "active")
              ? t("stockAnalysis.scheduledAnalysis.enabled")
              : t("stockAnalysis.scheduledAnalysis.disabled")}
          </span>
          <Select
            size="small"
            style={{ width: 180 }}
            value={wlCron}
            onChange={setWlCron}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.cron.tradeDayClose"), value: "0 21 * * 1-5" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyOpen"), value: "0 9 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyClose"), value: "30 15 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.weeklyMon"), value: "0 9 * * 1" },
            ]}
          />
          {wlScanJobs.length > 0 && (
            <Popconfirm
              title={t("stockAnalysis.scheduledAnalysis.confirmDelete")}
              onConfirm={() => deleteWlScan(wlScanJobs[0].id)}
            >
              <Button size="small" type="text" danger icon={<DeleteOutlined />} />
            </Popconfirm>
          )}
        </div>
      </Card>

      {/* 候选池逐只分析（pool-scan）：reco_picks 候选池 → 逐只完整分析 → pending 反思 */}
      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.poolScan")}
        styles={{ body: { padding: "8px 12px" } }}
      >
        <div className="flex items-center gap-3 flex-wrap">
          <Switch
            checked={poolJobs.some((j) => j.status === "active")}
            onChange={(checked) => togglePool(poolJobs.length > 0 ? poolJobs[0] : null, checked)}
          />
          <span className="text-xs text-gray-400">
            {poolJobs.some((j) => j.status === "active")
              ? t("stockAnalysis.scheduledAnalysis.enabled")
              : t("stockAnalysis.scheduledAnalysis.disabled")}
          </span>
          <Select
            size="small"
            style={{ width: 180 }}
            value={poolCron}
            onChange={setPoolCron}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.cron.tradeDayClose"), value: "0 17 * * 1-5" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyOpen"), value: "0 9 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyClose"), value: "30 15 * * *" },
            ]}
          />
          {poolJobs.length > 0 && (
            <Popconfirm
              title={t("stockAnalysis.scheduledAnalysis.confirmDelete")}
              onConfirm={() => deletePool(poolJobs[0].id)}
            >
              <Button size="small" type="text" danger icon={<DeleteOutlined />} />
            </Popconfirm>
          )}
        </div>
        <div className="text-xs text-gray-500 mt-1">
          {t("stockAnalysis.scheduledAnalysis.poolScanHint")}
        </div>
      </Card>

      {/* 趋势智选定时筛选（trend-screening）：两链各一任务 → reco_picks → 候选池 */}
      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.trendScreening")}
        styles={{ body: { padding: "8px 12px" } }}
      >
        {TREND_CHAINS.map((chain) => {
          const chainJob = trendJobs.find((j) => j.workflowId === chain.id) ?? null;
          const active = chainJob?.status === "active";
          return (
            <div key={chain.id} className="flex items-center gap-3 flex-wrap mb-1">
              <span className="text-xs text-gray-500 w-20 shrink-0">{t(chain.labelKey)}</span>
              <Switch
                checked={active}
                onChange={(checked) => toggleTrend(chain.id, chainJob, checked)}
              />
              <span className="text-xs text-gray-400">
                {active
                  ? t("stockAnalysis.scheduledAnalysis.enabled")
                  : t("stockAnalysis.scheduledAnalysis.disabled")}
              </span>
              <Select
                size="small"
                style={{ width: 180 }}
                value={trendCrons[chain.id]}
                onChange={(v) => setTrendCrons((prev) => ({ ...prev, [chain.id]: v }))}
                options={[
                  // ⚠ 必须含 `trendCrons` 的初值（16:00）：antd Select 找不到匹配 option 时
                  // 会把 cron 表达式原样显示出来。16:00 是刻意的 —— 晚于收盘（15:00）等数据
                  // 落定，早于 pool-scan（17:00）避免同时抢 astock 供应商配额。
                  { label: t("stockAnalysis.scheduledAnalysis.cron.tradeDay1600"), value: "0 16 * * 1-5" },
                  { label: t("stockAnalysis.scheduledAnalysis.cron.dailyClose"), value: "30 15 * * *" },
                  { label: t("stockAnalysis.scheduledAnalysis.cron.weeklyMon"), value: "0 9 * * 1" },
                  { label: t("stockAnalysis.scheduledAnalysis.cron.dailyOpen"), value: "0 9 * * *" },
                ]}
              />
              {chainJob && (
                <Popconfirm
                  title={t("stockAnalysis.scheduledAnalysis.confirmDelete")}
                  onConfirm={() => deleteTrend(chainJob.id)}
                >
                  <Button size="small" type="text" danger icon={<DeleteOutlined />} />
                </Popconfirm>
              )}
            </div>
          );
        })}
        <div className="text-xs text-gray-500 mt-1">
          {t("stockAnalysis.scheduledAnalysis.trendScreeningHint")}
        </div>
      </Card>

      {/* 4 周期反思（batch-reflection）：按超短/短/中/长各自的持有期间隔自动反思 */}
      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.reflectionScan")}
        styles={{ body: { padding: "8px 12px" } }}
        extra={
          <Button size="small" icon={<PlusOutlined />} onClick={createRefl}>
            {t("stockAnalysis.scheduledAnalysis.create")}
          </Button>
        }
      >
        <div className="flex items-center gap-3 flex-wrap">
          <Select
            size="small"
            style={{ width: 130 }}
            value={reflPeriod}
            onChange={setReflPeriod}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.period.ultraShort"), value: "ultra_short" },
              { label: t("stockAnalysis.scheduledAnalysis.period.short"), value: "short" },
              { label: t("stockAnalysis.scheduledAnalysis.period.mid"), value: "mid" },
              { label: t("stockAnalysis.scheduledAnalysis.period.long"), value: "long" },
            ]}
          />
          <Select
            size="small"
            style={{ width: 180 }}
            value={reflCron}
            onChange={setReflCron}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyClose"), value: "30 15 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.daily1800"), value: "0 18 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.weeklyMon"), value: "0 9 * * 1" },
            ]}
          />
        </div>
        <div className="text-xs text-gray-500 mt-1">
          {t("stockAnalysis.scheduledAnalysis.reflectionScanHint")}
        </div>
        {reflJobs.length === 0
          ? <div className="text-xs text-gray-400 mt-2">{t("stockAnalysis.scheduledAnalysis.noTask")}</div>
          : reflJobs.map((j) => (
            <div key={j.id} className="flex items-center gap-2 mt-2">
              <Switch size="small" checked={j.status === "active"} onChange={(c) => toggleRefl(j, c)} />
              <span className="text-xs">{j.description}</span>
              <Tag className="text-xs m-0 font-mono">{j.schedule}</Tag>
              <Popconfirm
                title={t("stockAnalysis.scheduledAnalysis.confirmDelete")}
                onConfirm={() => deleteRefl(j.id)}
              >
                <Button size="small" type="text" danger icon={<DeleteOutlined />} />
              </Popconfirm>
            </div>
          ))}
      </Card>

      {/* 决策回测（decision-backtest）：历史荐股 → T+N 实盘 K 线 → 命中率 + 因子 IC */}
      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.decisionBacktest")}
        styles={{ body: { padding: "8px 12px" } }}
        extra={
          <Button size="small" icon={<PlusOutlined />} onClick={createBt}>
            {t("stockAnalysis.scheduledAnalysis.create")}
          </Button>
        }
      >
        <div className="flex items-center gap-3 flex-wrap">
          <Select
            size="small"
            mode="multiple"
            style={{ width: 190 }}
            value={btWindows}
            onChange={setBtWindows}
            options={BT_WINDOWS.map((n) => ({
              value: n,
              label: t("stockAnalysis.scheduledAnalysis.decisionBacktestWindow", { n }),
            }))}
          />
          <Select
            size="small"
            style={{ width: 130 }}
            value={btPeriod}
            onChange={setBtPeriod}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.decisionBacktestPeriodAll"), value: "all" },
              { label: t("stockAnalysis.scheduledAnalysis.period.short"), value: "short" },
              { label: t("stockAnalysis.scheduledAnalysis.period.mid"), value: "mid" },
              { label: t("stockAnalysis.scheduledAnalysis.period.long"), value: "long" },
            ]}
          />
          <Select
            size="small"
            style={{ width: 180 }}
            value={btCron}
            onChange={setBtCron}
            options={[
              { label: t("stockAnalysis.scheduledAnalysis.cron.daily0700"), value: "0 7 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.dailyClose"), value: "30 15 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.daily1800"), value: "0 18 * * *" },
              { label: t("stockAnalysis.scheduledAnalysis.cron.weeklyMon"), value: "0 9 * * 1" },
            ]}
          />
          <span className="text-xs text-gray-400">
            {t("stockAnalysis.scheduledAnalysis.decisionBacktestMaxPicks")}
          </span>
          <InputNumber
            size="small"
            min={1}
            max={2000}
            step={50}
            style={{ width: 100 }}
            value={btMaxPicks}
            onChange={(v) => v != null && setBtMaxPicks(v)}
          />
        </div>
        <div className="text-xs text-gray-500 mt-1">
          {t("stockAnalysis.scheduledAnalysis.decisionBacktestHint")}
        </div>
        {btJobs.length === 0
          ? <div className="text-xs text-gray-400 mt-2">{t("stockAnalysis.scheduledAnalysis.noTask")}</div>
          : btJobs.map((j) => (
            <div key={j.id} className="flex items-center gap-2 mt-2">
              <Switch size="small" checked={j.status === "active"} onChange={(c) => toggleBt(j, c)} />
              <span className="text-xs">{j.description}</span>
              <Tag className="text-xs m-0 font-mono">{j.schedule}</Tag>
              <Popconfirm
                title={t("stockAnalysis.scheduledAnalysis.confirmDelete")}
                onConfirm={() => deleteBt(j.id)}
              >
                <Button size="small" type="text" danger icon={<DeleteOutlined />} />
              </Popconfirm>
            </div>
          ))}
      </Card>

      <Card
        size="small"
        title={t("stockAnalysis.scheduledAnalysis.newTask")}
        styles={{ body: { padding: "8px 12px" } }}
        extra={
          <Button size="small" icon={<PlusOutlined />} onClick={() => setAdding(!adding)}>
            {adding ? t("stockAnalysis.scheduledAnalysis.cancel") : t("stockAnalysis.scheduledAnalysis.new")}
          </Button>
        }
      >
        {adding && (
          <Form form={form} size="small" layout="inline" onFinish={create}>
            <Form.Item name="stockCode" rules={[{ required: true }]} initialValue={stockCode}>
              <Input placeholder={t("stockAnalysis.scheduledAnalysis.placeholder.stockCode")} style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="stockName" rules={[{ required: true }]} initialValue={stockName}>
              <Input placeholder={t("stockAnalysis.scheduledAnalysis.placeholder.stockName")} style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="cronExpression" rules={[{ required: true }]} initialValue="0 9 * * *">
              <Select
                style={{ width: 160 }}
                options={getCronPresets(t)}
                placeholder={t("stockAnalysis.scheduledAnalysis.placeholder.cron")}
              />
            </Form.Item>
            <Form.Item>
              <Button type="primary" htmlType="submit">{t("stockAnalysis.scheduledAnalysis.create")}</Button>
            </Form.Item>
          </Form>
        )}
      </Card>

      {jobs.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.scheduledAnalysis.empty")} />
        : <Table size="small" dataSource={jobs} columns={columns} rowKey="id" pagination={false} />}
    </div>
  );
}
