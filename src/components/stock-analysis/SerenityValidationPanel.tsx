import { ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Space, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { invoke } from "@/lib/invoke";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";

/**
 * `serenity_validation_summary` 的返回形态（后端 DTO 为 camelCase）。
 *
 * 刻意在本文件里就地声明而不进 `@/types`：这条链暂无第二个消费者，
 * 放进 barrel 会让它看起来像全站共享类型（禁区 12 的复用判据是「已有相同定义」，不是「可能将来有」）。
 */
interface SerenityValidationSummary {
  rows: number;
  candidates: number;
  judged: number;
  outperform: number;
  /** `judged === 0` 时为 null —— 后端不出数，前端也不许把它补成 0% */
  hitRate: number | null;
  labelCounts: { label: string; count: number }[];
  latestValidatedAt: string | null;
  generation: string | null;
  inWeightLoop: boolean;
}

/**
 * 趋势智选后验面板（B1，`PLAN-chain-closure-remediation.md` §3）。
 *
 * 存在理由：智选的 T+N 超额验证早就在跑（07:00 内置任务的独立一趟，现网已落 354 行），
 * 但产物只进 tracing 日志 —— 前端从未 invoke 过决策回测那条命令，界面上看不到任何
 * 智选后验读数，于是「智选有没有效果」在 UI 上是个无法回答的问题。
 *
 * 三条呈现纪律（缺一即造歧义）：
 * 1. 缺席分列：库里零行 / 有行但零判定 / 取数失败，各占一句，不共用一枚「0%」；
 * 2. 分母为 0 不出数：后端 `hitRate` 已是 nullable，这里不补 0；
 * 3. `inWeightLoop === false` 必须说出来 —— 否则这格会被读成「智选效果差所以没降权」，
 *    而真因是它的后验标签域按设计不进荐股 IC 闭环。
 */
export function SerenityValidationPanel() {
  const { t } = useTranslation();
  // 代际**跟着时间锚点走**（同 RecommendationPanel）：回放态按那批的截止日读，
  // 实盘态读实盘代。刻意不做成 props —— 一个没人传的形参就是「参数接了线没接」的老毛病。
  const anchorMode = useTimeAnchorStore((s) => s.mode);
  const anchorDate = useTimeAnchorStore((s) => s.asOfDate);
  const generation = anchorMode === "replay" ? anchorDate : null;
  const [summary, setSummary] = useState<SerenityValidationSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<SerenityValidationSummary>("serenity_validation_summary", {
        generation,
      });
      setSummary(data);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSummary(null);
    } finally {
      setLoading(false);
    }
  }, [generation]);

  useEffect(() => {
    void load();
  }, [load]);

  const metric = (labelKey: string, value: string, testid: string) => (
    <div className="flex flex-col gap-0.5">
      <span style={{ fontSize: 11, color: "var(--muted)" }}>{t(labelKey)}</span>
      <strong data-testid={testid}>{value}</strong>
    </div>
  );

  let body: ReactNode;
  if (loading) {
    body = <Spin size="small" style={{ display: "block", margin: "16px auto" }} />;
  } else if (error) {
    body = (
      <Alert
        type="error"
        showIcon
        className="text-xs!"
        data-testid="serenity-validation-failed"
        title={t("backtest.serenityValidation.stateFetchFailed", { error })}
      />
    );
  } else if (!summary || summary.rows === 0) {
    body = (
      <Alert
        type="warning"
        showIcon
        className="text-xs!"
        data-testid="serenity-validation-no-rows"
        title={t("backtest.serenityValidation.stateNoRows")}
      />
    );
  } else if (summary.judged === 0) {
    body = (
      <>
        <Alert
          type="info"
          showIcon
          className="text-xs! mb-2!"
          data-testid="serenity-validation-unjudged"
          title={t("backtest.serenityValidation.stateUnjudged")}
        />
        <div className="flex gap-4">
          {metric("backtest.serenityValidation.rows", String(summary.rows), "serenity-validation-rows")}
          {metric(
            "backtest.serenityValidation.candidates",
            String(summary.candidates),
            "serenity-validation-candidates",
          )}
        </div>
      </>
    );
  } else {
    body = (
      <>
        <div className="flex flex-wrap gap-4">
          {metric("backtest.serenityValidation.rows", String(summary.rows), "serenity-validation-rows")}
          {metric(
            "backtest.serenityValidation.candidates",
            String(summary.candidates),
            "serenity-validation-candidates",
          )}
          {metric(
            "backtest.serenityValidation.judged",
            String(summary.judged),
            "serenity-validation-judged",
          )}
          {metric(
            "backtest.serenityValidation.outperform",
            String(summary.outperform),
            "serenity-validation-outperform",
          )}
          {metric(
            "backtest.serenityValidation.hitRate",
            summary.hitRate == null ? "—" : `${(summary.hitRate * 100).toFixed(1)}%`,
            "serenity-validation-hit-rate",
          )}
          {metric(
            "backtest.serenityValidation.latest",
            summary.latestValidatedAt ?? "—",
            "serenity-validation-latest",
          )}
        </div>
        <div className="mt-2 flex flex-wrap gap-1">
          {summary.labelCounts.map((l) => (
            <Tag key={l.label} data-testid={`serenity-validation-label-${l.label}`}>
              {l.label} · {l.count}
            </Tag>
          ))}
        </div>
      </>
    );
  }

  return (
    <Card
      size="small"
      title={t("backtest.serenityValidation.title")}
      data-testid="serenity-validation-panel"
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Space size={6}>
          {generation && (
            <Tag color="blue" data-testid="serenity-validation-generation">
              {generation}
            </Tag>
          )}
          <Button
            size="small"
            icon={<ReloadOutlined />}
            loading={loading}
            onClick={() => void load()}
            data-testid="serenity-validation-reload"
          >
            {t("common.refresh")}
          </Button>
        </Space>
      }
    >
      {body}
      {/* 两条恒定声明：只在拿到读数后说，否则「未参与闭环」会被读成「取数失败」的同义句 */}
      {!loading && !error && summary && summary.rows > 0 && (
        <>
          <Alert
            type="info"
            showIcon
            className="text-xs! mt-2!"
            data-testid="serenity-validation-loop-note"
            title={<span className="text-xs">{t("backtest.serenityValidation.loopNote")}</span>}
          />
          <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 6 }}>
            {t("backtest.serenityValidation.excessNote")}
          </div>
        </>
      )}
    </Card>
  );
}
