// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { Alert, App, Button, Space, Switch, theme } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";

/**
 * 决策否决门开关面板（2026-10-26 用户裁决）。
 *
 * 四门：
 *   - 风控门（高/极高风险禁止加仓/持仓）
 *   - 情绪门（全市场封板率不足）
 *   - 估值门（严重高估）
 *   - 顶层采纳档内买入（解除 `R-CALIB-CAP` 的动作联动）
 *
 * ⚠ 本面板**刻意不走模板变量**（`StockAnalysisConfigPanel` 那条路）：
 * 改模板变量会触发模板 `version + 1`（换代快照）⇒ 与「冻结换代、样本可比」冲突。
 * 三门走独立的 `settings` 表 ⇒ **改开关不换代、不重播种、下一轮分析即生效**。
 */
export function DecisionGatesPanel() {
  const { t } = useTranslation();
  const { message } = App.useApp();
  const { token } = theme.useToken();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [state, setState] = useState({
    riskVetoEnabled: false,
    sentimentEnabled: false,
    valuationEnabled: false,
    tierBuyEnabled: false,
  });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await invoke<typeof state>("get_decision_gates");
      setState(next);
    } catch (e) {
      message.error(t("stockAnalysis.decisionGates.loadFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [message, t]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = useCallback(
    async (next: typeof state) => {
      setSaving(true);
      try {
        const saved = await invoke<typeof state>("set_decision_gates", {
          riskVetoEnabled: next.riskVetoEnabled,
          sentimentEnabled: next.sentimentEnabled,
          valuationEnabled: next.valuationEnabled,
          tierBuyEnabled: next.tierBuyEnabled,
        });
        setState(saved);
        message.success(t("stockAnalysis.decisionGates.saved"));
      } catch (e) {
        message.error(t("stockAnalysis.decisionGates.saveFailed", { error: String(e) }));
      } finally {
        setSaving(false);
      }
    },
    [message, t],
  );

  const items = [
    {
      key: "riskVetoEnabled" as const,
      label: t("stockAnalysis.decisionGates.riskVeto.label"),
      desc: t("stockAnalysis.decisionGates.riskVeto.desc"),
    },
    {
      key: "sentimentEnabled" as const,
      label: t("stockAnalysis.decisionGates.sentiment.label"),
      desc: t("stockAnalysis.decisionGates.sentiment.desc"),
    },
    {
      key: "valuationEnabled" as const,
      label: t("stockAnalysis.decisionGates.valuation.label"),
      desc: t("stockAnalysis.decisionGates.valuation.desc"),
    },
    {
      key: "tierBuyEnabled" as const,
      label: t("stockAnalysis.decisionGates.tierBuy.label"),
      desc: t("stockAnalysis.decisionGates.tierBuy.desc"),
    },
  ];

  return (
    <SettingsGroup title={t("stockAnalysis.decisionGates.title")}>
      <Alert
        type="warning"
        showIcon
        style={{ marginBottom: 12 }}
        message={t("stockAnalysis.decisionGates.warning")}
        description={t("stockAnalysis.decisionGates.warningDetail")}
      />
      <Space direction="vertical" size={12} style={{ width: "100%" }}>
        {items.map((it) => (
          <div
            key={it.key}
            style={{
              display: "flex",
              alignItems: "flex-start",
              justifyContent: "space-between",
              gap: 16,
              padding: "8px 4px",
              borderBottom: `1px solid ${token.colorBorderSecondary}`,
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>{it.label}</div>
              <div
                style={{
                  fontSize: 12,
                  color: token.colorTextSecondary,
                  marginTop: 2,
                  lineHeight: 1.5,
                }}
              >
                {it.desc}
              </div>
            </div>
            <Switch
              checked={state[it.key]}
              disabled={loading || saving}
              onChange={(checked) => {
                // 乐观更新 + 失败回滚：写命令失败时 load() 会把真值拉回来。
                const next = { ...state, [it.key]: checked };
                setState(next);
                void save(next);
              }}
            />
          </div>
        ))}
        <div style={{ display: "flex", justifyContent: "flex-end", paddingTop: 4 }}>
          <Button size="small" onClick={() => void load()} loading={loading}>
            {t("common.refresh")}
          </Button>
        </div>
      </Space>
    </SettingsGroup>
  );
}
