// SPDX-License-Identifier: AGPL-3.0-only

import type { ABTestResult, EvolutionPort, SkillVersion } from "@/components/common/evolutionPort";
import { Button, Card, Collapse, Empty, Tag, theme, Typography } from "antd";
import { Activity, ChevronDown, Clock, Play, TrendingDown, TrendingUp } from "lucide-react";
import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkflowNode } from "../types/workflow.types";

const { Text } = Typography;

interface EvolutionTabProps {
  currentWorkflowId: string | null;
  nodes: WorkflowNode[];
  /** 进化能力端口（任务 #220）：由 AIPanel 透传（源头为 WorkflowPage 注入）。缺省时只读展示空态、触发按钮 no-op。 */
  evolutionPort?: EvolutionPort;
}

export const EvolutionTab: React.FC<EvolutionTabProps> = React.memo(({ currentWorkflowId, nodes, evolutionPort }) => {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  const [nodeHistories, setNodeHistories] = useState<Record<string, SkillVersion[]>>({});
  const [abResults, setAbResults] = useState<ABTestResult[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    // 端口缺省（如测试未注入）：清空数据并结束加载，不再直引 evolution store。
    if (!evolutionPort) {
      setNodeHistories({});
      setAbResults([]);
      setLoading(false);
      return;
    }

    // 端口已异步化(任务 #218 批一):store getter 返回 Promise,包 async IIFE 逐节点拉取。
    // 传参说明:节点→技能绑定关系盘上不存在(消费方拿不到真实 skillId,已回报纠正,等 leader 改裁),
    // 按裁定「无绑定 ⇒ 空态」传空串;store 对空串短路返回 [],绑定落地后此处改传真 skillId 即生效。
    (async () => {
      // 逐节点取进化版本历史(端口返回 store 权威 SkillVersion[])。
      const histories: Record<string, SkillVersion[]> = {};
      for (const node of nodes) {
        try {
          const history = await evolutionPort.getSkillEvolutionHistory("");
          if (history && history.length > 0) {
            histories[node.id] = history;
          }
        } catch {
          // 该节点无历史 —— 跳过
        }
      }
      if (!cancelled) { setNodeHistories(histories); }

      // 取 A/B 测试结果(端口返回 store 权威 ABTestResult[],键空间为 skill_id)。
      try {
        const results = await evolutionPort.getABTestResults("");
        if (!cancelled && results) { setAbResults(results); }
      } catch {
        // 无 A/B 结果
      }

      if (!cancelled) { setLoading(false); }
    })();

    return () => {
      cancelled = true;
    };
  }, [currentWorkflowId, nodes, evolutionPort]);

  const handleTriggerAll = useCallback(() => {
    if (!evolutionPort) { return; }
    for (const node of nodes) {
      try {
        evolutionPort.triggerSkillEvolution(node.id);
      } catch {
        // 忽略单个失败
      }
    }
  }, [nodes, evolutionPort]);

  const handleTriggerSingle = useCallback((nodeId: string) => {
    if (!evolutionPort) { return; }
    try {
      evolutionPort.triggerSkillEvolution(nodeId);
    } catch {
      // 忽略
    }
  }, [evolutionPort]);

  // SkillVersion.timestamp 为毫秒时间戳（与 store / AIPanel 口径一致）。
  const formatTime = useCallback((ts: number): string => {
    try {
      return new Date(ts).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return String(ts);
    }
  }, []);

  const hasAnyHistory = Object.keys(nodeHistories).length > 0;

  return (
    <div style={{ height: "100%", overflowY: "auto", padding: "12px" }}>
      {/* 顶部标题 + 全部触发按钮 */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 12,
        }}
      >
        <Text strong style={{ fontSize: 13, color: token.colorText }}>
          {t("workflow.evolution.title")}
        </Text>
        {nodes.length > 0 && (
          <Button
            type="primary"
            size="small"
            icon={<Play size={12} />}
            onClick={handleTriggerAll}
            style={{ fontSize: 12 }}
          >
            {t("workflow.evolution.triggerAll")}
          </Button>
        )}
      </div>

      {/* 可优化节点列表 */}
      {nodes.length === 0
        ? <Empty description={t("workflow.evolution.noNodes")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : !hasAnyHistory && !loading
        ? <Empty description={t("workflow.evolution.noHistory")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {nodes.map((node) => {
              const history = nodeHistories[node.id];
              const latest = history?.[0];

              // SkillVersion.metrics 为 Record<string, { before; after }>，取成功率/耗时的前后差。
              const sr = latest?.metrics?.successRate;
              const srDiff = sr ? sr.after - sr.before : null;
              const lat = latest?.metrics?.avgTime;
              const latDiff = lat ? lat.after - lat.before : null;

              return (
                <Card
                  key={node.id}
                  size="small"
                  style={{
                    background: token.colorBgContainer,
                    border: `1px solid ${token.colorBorderSecondary}`,
                  }}
                  styles={{ body: { padding: "8px 10px" } }}
                >
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
                        <Text
                          style={{
                            fontSize: 13,
                            fontWeight: 500,
                            color: token.colorText,
                            maxWidth: 140,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {node.title || node.id}
                        </Text>
                        <Tag
                          color="purple"
                          style={{ fontSize: 10, margin: 0, padding: "0 4px", lineHeight: "16px" }}
                        >
                          {node.type}
                        </Tag>
                      </div>

                      {latest
                        ? (
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: 10,
                              fontSize: 11,
                              color: token.colorTextSecondary,
                            }}
                          >
                            <span style={{ display: "flex", alignItems: "center", gap: 2 }}>
                              <Activity size={10} />
                              v{latest.version}
                            </span>
                            <span style={{ display: "flex", alignItems: "center", gap: 2 }}>
                              <Clock size={10} />
                              {formatTime(latest.timestamp)}
                            </span>
                            {srDiff != null && (
                              <span
                                style={{
                                  display: "flex",
                                  alignItems: "center",
                                  gap: 2,
                                  color: srDiff >= 0 ? token.colorSuccess : token.colorError,
                                }}
                              >
                                {srDiff >= 0 ? <TrendingUp size={10} /> : <TrendingDown size={10} />}
                                {t("workflow.evolution.successRate", {
                                  diff: `${srDiff >= 0 ? "+" : ""}${srDiff.toFixed(1)}%`,
                                })}
                              </span>
                            )}
                            {latDiff != null && (
                              <span
                                style={{
                                  display: "flex",
                                  alignItems: "center",
                                  gap: 2,
                                  color: latDiff <= 0 ? token.colorSuccess : token.colorError,
                                }}
                              >
                                {latDiff <= 0 ? <TrendingDown size={10} /> : <TrendingUp size={10} />}
                                {t("workflow.evolution.latency", {
                                  diff: `${latDiff >= 0 ? "+" : ""}${latDiff.toFixed(1)}`,
                                })}
                              </span>
                            )}
                          </div>
                        )
                        : (
                          <Text style={{ fontSize: 11, color: token.colorTextTertiary }}>
                            {t("workflow.evolution.noEvolutionRecord")}
                          </Text>
                        )}
                      {latest?.summary && (
                        <Text
                          style={{
                            display: "block",
                            fontSize: 11,
                            color: token.colorTextTertiary,
                            marginTop: 2,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {latest.summary}
                        </Text>
                      )}
                    </div>

                    <Button
                      type="link"
                      size="small"
                      icon={<Play size={12} />}
                      onClick={() => handleTriggerSingle(node.id)}
                      style={{ fontSize: 11, flexShrink: 0 }}
                    >
                      {t("workflow.evolution.triggerEvolution")}
                    </Button>
                  </div>
                </Card>
              );
            })}
          </div>
        )}

      {/* A/B 测试区（消费 store 权威 ABTestResult：metric / valueA / valueB / change / winner） */}
      <div style={{ marginTop: 16 }}>
        <Collapse
          ghost
          size="small"
          expandIcon={({ isActive }) => (
            <ChevronDown size={12} style={{ transform: isActive ? "rotate(180deg)" : undefined, transition: "0.2s" }} />
          )}
          items={[
            {
              key: "ab-test",
              label: (
                <Text strong style={{ fontSize: 12, color: token.colorText }}>
                  {t("workflow.evolution.abTestTitle", { count: abResults.length })}
                </Text>
              ),
              children: abResults.length === 0
                ? (
                  <Empty
                    description={t("workflow.evolution.noActiveABTest")}
                    image={Empty.PRESENTED_IMAGE_SIMPLE}
                    style={{ padding: "8px 0" }}
                  />
                )
                : (
                  <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                    {abResults.map((res, idx) => (
                      <Card
                        key={`${res.metric}-${idx}`}
                        size="small"
                        style={{ background: token.colorFillTertiary, border: "none" }}
                        styles={{ body: { padding: "8px 10px" } }}
                      >
                        <div
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            marginBottom: 4,
                          }}
                        >
                          <Text style={{ fontSize: 12, color: token.colorText }}>
                            {res.metric}
                          </Text>
                          {res.winner && res.winner !== "tie" && (
                            <Tag
                              color="green"
                              style={{ fontSize: 10, margin: 0, padding: "0 4px", lineHeight: "16px" }}
                            >
                              {t("workflow.evolution.winner", { winner: res.winner })}
                            </Tag>
                          )}
                        </div>
                        <div style={{ display: "flex", gap: 16, fontSize: 11, color: token.colorTextSecondary }}>
                          <span>A={res.valueA.toFixed(2)}</span>
                          <span>B={res.valueB.toFixed(2)}</span>
                          <span>
                            {t("workflow.evolution.diff")}:{" "}
                            {res.change === null ? "—" : `${res.change >= 0 ? "+" : ""}${res.change.toFixed(1)}%`}
                          </span>
                        </div>
                      </Card>
                    ))}
                  </div>
                ),
            },
          ]}
        />
      </div>
    </div>
  );
});
