import { PageHeader } from "@/components/stock-analysis/_shared/PageHeader";
import { AsOfDatePicker } from "@/components/time-travel/AsOfDatePicker";
import { ReplayBadge, ReplayWatermark } from "@/components/time-travel/ReplayBadge";
import { ReplaySweep } from "@/components/time-travel/ReplaySweep";
import { TimeTravelBatchCard } from "@/components/time-travel/TimeTravelBatchCard";
import { investPath } from "@/lib/investRoutes";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { Alert, Button, Card, Empty, Space, Typography } from "antd";
import { RotateCcw, Zap } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

const { Paragraph } = Typography;

/**
 * ReplayWorkbench — 时间旅行回放工作台
 *
 * 设计目标（HCI §10.1）：
 *   - 强制重选 as-of（进入本页时不沿用旧 asOfDate）
 *   - 给用户一个明确的"我现在用过去眼光看市场"的操作空间
 *   - 在工作台内可跳转分析/荐股面板（带 as-of 注入）
 *
 * 行为：
 *   - 页面加载 → 渲染 AsOfDatePicker，等待用户选日期
 *   - 用户选定 → enterReplayWorkbench(date) 覆盖当前 as-of
 *   - 选定后展示 3 个动作：分析单股 / 看荐股 / 看回测
 *   - "切回 Live" 按钮 → 调用 enterLive(requireConfirm=false)
 *     （PageHeader 的 ModeSwitch 仍会弹二次确认 Modal）
 *
 * 入口：工作区 → 回测视图右上角时钟按钮（`BacktestView`）跳 `/invest/replay`
 * （`BUILTIN_PAGE_PATH.investReplay`）；投资中心标题栏另有「批次复盘」快捷入口（Drawer），
 * 两者复用同一 `TimeTravelBatchCard`。
 *
 * ⚠ 路由历史：本页原由顶级 `/replay-workbench` 承载，该路由已于 2026-10-28 删除，
 * 现为投资域内的 `/invest/replay`（ContentArea 注册的 `LazyReplayWorkbenchPage`）。
 * 任何「进入回放工作台」的新调用点请走 `BUILTIN_PAGE_PATH.investReplay`，
 * 不要再写字面量，也不要再取已删除的 `replay-workbench` 键（那会得到 `undefined`）。
 */
export function ReplayWorkbench() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const mode = useTimeAnchorStore((s) => s.mode);
  const enterReplayWorkbench = useTimeAnchorStore((s) => s.enterReplayWorkbench);
  const enterLive = useTimeAnchorStore((s) => s.enterLive);
  const [chosen, setChosen] = useState<string | null>(asOfDate);

  const isLocked = mode === "replay" && asOfDate !== null;

  const handlePick = (date: string) => {
    enterReplayWorkbench(date);
    setChosen(date);
  };

  const handleReset = () => {
    setChosen(null);
  };

  const handleSwitchToLive = () => {
    // 注意：ModeSwitch 的二次确认 Modal 仍会在切到 live 时弹出
    enterLive(false);
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        titleKey="replayWorkbench.title"
        backTo="/invest"
        backLabelKey="nav.chat"
        showTimeAnchor
      />
      <div style={{ padding: "24px 16px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 16 }}>
          {isLocked && <ReplayBadge />}
        </div>

        <Paragraph type="secondary" style={{ marginBottom: 16 }}>
          {t("replayWorkbench.description")}
        </Paragraph>

        <Card
          title={
            <span>
              <span style={{ marginRight: 8 }}>1️⃣</span>
              {t("replayWorkbench.step1.title")}
            </span>
          }
          size="small"
          style={{ marginBottom: 16 }}
        >
          {chosen === null
            ? (
              <AsOfDatePicker
                onPick={handlePick}
                onCancel={() => navigate(investPath({ view: "backtest" }))}
              />
            )
            : (
              <Space size="middle" wrap>
                <span style={{ fontSize: 13 }}>
                  {t("replayWorkbench.step1.currentlyPicked", { date: chosen })}
                </span>
                <Button
                  size="small"
                  icon={<RotateCcw size={12} />}
                  onClick={handleReset}
                  data-testid="replay-reset-btn"
                >
                  {t("replayWorkbench.step1.reselect")}
                </Button>
              </Space>
            )}
        </Card>

        {chosen && (
          <Card
            title={
              <span>
                <span style={{ marginRight: 8 }}>2️⃣</span>
                {t("replayWorkbench.step2.title")}
              </span>
            }
            size="small"
            style={{ marginBottom: 16, position: "relative" }}
          >
            <div style={{ position: "relative" }}>
              <Paragraph type="secondary" style={{ marginBottom: 12 }}>
                {t("replayWorkbench.step2.description", { date: chosen })}
              </Paragraph>
              <Space wrap>
                <Button
                  type="primary"
                  onClick={() => navigate(investPath({ view: "analysis" }))}
                  data-testid="goto-stock-analysis"
                >
                  {t("replayWorkbench.step2.analyze")}
                </Button>
                <Button
                  onClick={() => navigate(investPath({ view: "backtest" }))}
                  data-testid="goto-backtest"
                >
                  {t("replayWorkbench.step2.backtest")}
                </Button>
                <Button
                  onClick={() => navigate(investPath({ view: "monitor" }))}
                  data-testid="goto-watchlist"
                >
                  {t("replayWorkbench.step2.watchlist")}
                </Button>
              </Space>
              <ReplayWatermark />
            </div>
          </Card>
        )}

        <Alert
          type="info"
          showIcon
          title={t("replayWorkbench.notice")}
          style={{ marginBottom: 16 }}
        />

        {
          /*
          历史验证区（与当日生产严格分开，故不放在选股页）：
          1. 时间旅行批次复盘 —— 选定单个历史交易日，as-of 封闭世界一轮统一
             回放「智能荐股 + 趋势智选」两条平级选股链，合并去重后逐只 replay
             分析，并按 2/5/28/90 交易日反思；
          2. ReplaySweep —— 跨多日的策略回测批量扫描。
          两者都是「用过去的数据做事后验证」，与当日 00:00/01:00 的荐股生产
          不同时、不同逻辑。投资中心标题栏另有批次复盘快捷入口（Drawer），
          与本页渲染同一个卡片组件，无第二套实现。
        */
        }
        <TimeTravelBatchCard />
        <ReplaySweep />

        {isLocked && (
          <Card size="small" style={{ borderColor: "var(--red)" }}>
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("replayWorkbench.exitHint")}
            />
            <div style={{ textAlign: "center", marginTop: 12 }}>
              <Button
                danger
                icon={<Zap size={12} />}
                onClick={handleSwitchToLive}
                data-testid="replay-exit-btn"
              >
                {t("replayWorkbench.exitBtn")}
              </Button>
            </div>
          </Card>
        )}
      </div>
    </div>
  );
}
