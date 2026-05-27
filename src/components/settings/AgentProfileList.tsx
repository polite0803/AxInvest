/**
 * 股票分析 AgentProfile 列表编辑器。
 * 编辑模板节点 config.tools（暴露给 LLM 的工具），持久化到 workflow_templates 表。
 *
 * 工具分两种：
 *   固定工具 (⚙️ Fixed) — DAG 中 WorkflowNode::Tool 节点，确定性执行，结果注入 context_sources
 *   暴露工具 (🤖 LLM)  — 模板节点 config.tools，描述发送给 LLM，由 LLM 自主决定调用
 */
import { invoke } from "@/lib/invoke";
import { Button, Divider, message, Select, Space, Spin, Table, Tag, Tooltip } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

interface AgentNodeRow {
  id: string;
  title: string;
  expertId: string;
  profileId: string;
  expertName: string;
  roleName: string;
  tools: string[];
  fixedTools: string[];
}

interface Props {
  onGoToExperts?: () => void;
  onGoToRoles?: () => void;
}

/** 固定 Tool 节点映射 */
const FIXED_TOOL_MAP: Record<string, string[]> = {
  "a-market-analyst": ["get_stock_kline"],
  "a-sentiment": ["get_hot_stocks"],
  "a-news": ["get_announcements"],
  "a-fundamentals": ["get_consensus_eps"],
  "a-policy": ["get_announcements"],
  "a-hot-money": ["get_stock_money_flow"],
  "a-lockup": ["get_announcements"],
  "a-research": ["get_consensus_eps"],
  "a-sector": ["get_industry_ranking"],
};

const FIXED_ALGO_TOOLS: Record<string, string[]> = {
  "research-mgr": [
    "compute_scoring",
    "compute_valuation",
    "compute_portfolio_risk",
  ],
};

/** profile_id → 专家显示名 */
const PROFILE_EXPERT_MAP: Record<string, string> = {
  "stock-market-analyst": "市场技术分析师",
  "stock-sentiment-analyst": "情绪面分析师",
  "stock-news-analyst": "消息面分析师",
  "stock-fundamentals-analyst": "基本面分析师",
  "stock-policy-analyst": "政策面分析师",
  "stock-hot-money-tracker": "资金面追踪",
  "stock-lockup-watcher": "筹码限售观察",
  "stock-research-analyst": "研报分析师",
  "stock-sector-analyst": "板块题材分析师",
  "stock-bull-researcher": "多方研究员",
  "stock-bear-researcher": "空方研究员",
  "stock-aggressive-debator": "激进风险评估",
  "stock-conservative-debator": "保守风险评估",
  "stock-neutral-debator": "中性风险评估",
  "stock-research-manager": "研究经理",
  "stock-trader": "交易员",
  "stock-portfolio-manager": "投资组合经理",
};

const PROFILE_ROLE_MAP: Record<string, string> = {
  "stock-market-analyst": "股票分析师",
  "stock-sentiment-analyst": "股票分析师",
  "stock-news-analyst": "股票分析师",
  "stock-fundamentals-analyst": "股票分析师",
  "stock-policy-analyst": "股票分析师",
  "stock-hot-money-tracker": "股票分析师",
  "stock-lockup-watcher": "股票分析师",
  "stock-research-analyst": "股票分析师",
  "stock-sector-analyst": "股票分析师",
  "stock-bull-researcher": "辩论研究员",
  "stock-bear-researcher": "辩论研究员",
  "stock-aggressive-debator": "风险评估师",
  "stock-conservative-debator": "风险评估师",
  "stock-neutral-debator": "风险评估师",
  "stock-research-manager": "决策者",
  "stock-trader": "交易员",
  "stock-portfolio-manager": "决策者",
};

export function AgentProfileList({ onGoToExperts, onGoToRoles }: Props) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<AgentNodeRow[]>([]);
  const [allTools, setAllTools] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [editingTools, setEditingTools] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);

  const getFixedTools = useCallback((nodeId: string): string[] => {
    return FIXED_TOOL_MAP[nodeId] ?? FIXED_ALGO_TOOLS[nodeId] ?? [];
  }, []);

  const loadAll = useCallback(async () => {
    setLoading(true);
    try {
      const [template, tools] = await Promise.all([
        invoke<{ nodes: unknown[] }>("get_workflow_template", {
          id: "stock-analysis",
        }),
        invoke<string[]>("list_stock_tools"),
      ]);
      setAllTools(Array.isArray(tools) ? tools : []);

      if (!template || !template.nodes) {
        setRows([]);
      } else {
        const nodes = Array.isArray(template.nodes) ? template.nodes : [];
        const parsed = nodes
          .map((n: any) => {
            if (
              !n?.config?.agent_profile_id
              || !n.config.agent_profile_id.startsWith("stock-")
            ) {
              return null;
            }
            const profileId: string = n.config.agent_profile_id;
            const tools: string[] = (n.config.tools ?? []).map(
              (t: any) => t.name ?? t,
            );
            return {
              id: n.base?.id ?? n.id ?? "",
              title: n.base?.title ?? "",
              expertId: profileId,
              profileId,
              expertName: PROFILE_EXPERT_MAP[profileId] ?? profileId,
              roleName: PROFILE_ROLE_MAP[profileId] ?? "-",
              tools,
              fixedTools: getFixedTools(
                n.base?.id ?? n.id ?? "",
              ),
            } as AgentNodeRow;
          })
          .filter(Boolean) as AgentNodeRow[];
        // 按固定顺序排：9 分析师 → 2 辩手 → 3 风险评估 → research-mgr → trader → portfolio-mgr
        setRows(parsed);
      }
    } catch (err) {
      console.error("加载模板节点失败", err);
    } finally {
      setLoading(false);
    }
  }, [getFixedTools]);

  useEffect(() => {
    loadAll();
  }, [loadAll]);

  const handleSaveTools = useCallback(
    async (nodeId: string, tools: string[]) => {
      setSaving(nodeId);
      try {
        await invoke("update_workflow_template_node", {
          templateId: "stock-analysis",
          nodeId,
          input: { tools },
        });
        message.success(t("stockAnalysis.settings.profile.saved"));
        loadAll();
      } catch (err) {
        message.error(
          t("stockAnalysis.settings.profile.saveError", {
            error: String(err),
          }),
        );
      } finally {
        setSaving(null);
      }
    },
    [t, loadAll],
  );

  const classifyTools = useCallback(
    (nodeId: string, exposed: string[]) => {
      const fixed = new Set(getFixedTools(nodeId));
      return {
        fixedOnly: getFixedTools(nodeId),
        both: exposed.filter((t) => fixed.has(t)),
        exposedOnly: exposed.filter((t) => !fixed.has(t)),
      };
    },
    [getFixedTools],
  );

  const toolOptions = useMemo(
    () => allTools.map((t) => ({ label: t, value: t })),
    [allTools],
  );

  const columns: ColumnsType<AgentNodeRow> = [
    {
      title: t("stockAnalysis.settings.profile.colName"),
      dataIndex: "expertName",
      key: "name",
      width: 160,
      render: (name: string, record: AgentNodeRow) => (
        <div>
          <div className="font-medium">📈 {name}</div>
          <div className="text-xs text-gray-400">{record.id}</div>
        </div>
      ),
    },
    {
      title: t("stockAnalysis.settings.tab.experts"),
      key: "expert",
      width: 140,
      render: () => (
        <Button type="link" size="small" onClick={onGoToExperts}>
          → {t("stockAnalysis.settings.tab.experts")}
        </Button>
      ),
    },
    {
      title: t("stockAnalysis.settings.tab.roles"),
      key: "role",
      width: 130,
      render: (_: unknown, record: AgentNodeRow) => (
        <Space size={4}>
          <Tag color="blue">{record.roleName}</Tag>
          <Button type="link" size="small" onClick={onGoToRoles}>
            →
          </Button>
        </Space>
      ),
    },
    {
      title: t("stockAnalysis.settings.profile.colFixedTools"),
      key: "fixedTools",
      width: 170,
      render: (_: unknown, record: AgentNodeRow) => {
        if (record.fixedTools.length === 0) {
          return (
            <span className="text-gray-400 text-xs italic">
              {t("stockAnalysis.settings.profile.noFixedTools")}
            </span>
          );
        }
        return (
          <Space wrap size={[2, 2]}>
            {record.fixedTools.map((tn) => (
              <Tag key={tn} color="default" className="text-xs">
                ⚙️ {tn}
              </Tag>
            ))}
          </Space>
        );
      },
    },
    {
      title: t("stockAnalysis.settings.profile.colExposedTools"),
      key: "exposedTools",
      render: (_: unknown, record: AgentNodeRow) => {
        const isEditing = editingTools === record.id;
        const cls = classifyTools(record.id, record.tools);

        if (isEditing) {
          return (
            <Space direction="vertical" style={{ width: "100%" }}>
              {cls.fixedOnly.length > 0 && (
                <div className="text-xs text-gray-400">
                  {t("stockAnalysis.settings.profile.fixedHint", {
                    tools: cls.fixedOnly.join(", "),
                  })}
                </div>
              )}
              <Select
                mode="multiple"
                style={{ width: "100%", minWidth: 300 }}
                value={record.tools}
                options={toolOptions}
                onChange={(vals) => {
                  setRows((prev) => prev.map((r) => r.id === record.id ? { ...r, tools: vals } : r));
                }}
                placeholder={t("stockAnalysis.settings.profile.selectTools")}
                maxTagCount={6}
              />
              <Space>
                <Button
                  size="small"
                  type="primary"
                  loading={saving === record.id}
                  onClick={() => {
                    handleSaveTools(record.id, record.tools);
                    setEditingTools(null);
                  }}
                >
                  {t("stockAnalysis.settings.profile.save")}
                </Button>
                <Button
                  size="small"
                  onClick={() => {
                    loadAll();
                    setEditingTools(null);
                  }}
                >
                  {t("stockAnalysis.settings.profile.cancel")}
                </Button>
              </Space>
            </Space>
          );
        }

        return (
          <Space direction="vertical" size={2}>
            {cls.fixedOnly.length > 0 && (
              <Space wrap size={[2, 2]}>
                {cls.fixedOnly.map((tn) => (
                  <Tag key={tn} color="default" className="text-xs">
                    ⚙️ {tn}
                  </Tag>
                ))}
              </Space>
            )}
            {cls.both.length > 0 && (
              <Space wrap size={[2, 2]}>
                {cls.both.map((tn) => (
                  <Tooltip
                    key={tn}
                    title={t("stockAnalysis.settings.profile.bothHint")}
                  >
                    <Tag color="cyan" className="text-xs">
                      ⚙️🤖 {tn}
                    </Tag>
                  </Tooltip>
                ))}
              </Space>
            )}
            <Space wrap size={[2, 2]}>
              {cls.exposedOnly.map((tn) => (
                <Tag key={tn} color="green" className="text-xs">
                  🤖 {tn}
                </Tag>
              ))}
            </Space>
            <Divider style={{ margin: "4px 0" }} />
            <Button
              size="small"
              type="dashed"
              onClick={() => setEditingTools(record.id)}
            >
              {t("stockAnalysis.settings.profile.editTools")}
            </Button>
          </Space>
        );
      },
    },
  ];

  if (loading) {
    return (
      <div className="flex justify-center py-12">
        <Spin />
      </div>
    );
  }

  return (
    <div>
      <div className="mb-3 text-sm text-gray-500">
        <Space size={8}>
          <Tag color="default" className="text-xs">
            ⚙️ {t("stockAnalysis.settings.profile.legendFixed")}
          </Tag>
          <Tag color="green" className="text-xs">
            🤖 {t("stockAnalysis.settings.profile.legendExposed")}
          </Tag>
          <Tag color="cyan" className="text-xs">
            ⚙️🤖 {t("stockAnalysis.settings.profile.legendBoth")}
          </Tag>
        </Space>
      </div>
      <Table<AgentNodeRow>
        columns={columns}
        dataSource={rows}
        rowKey="id"
        size="small"
        pagination={false}
        scroll={{ x: 1000 }}
      />
    </div>
  );
}
