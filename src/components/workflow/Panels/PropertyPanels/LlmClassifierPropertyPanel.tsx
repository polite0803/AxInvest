// SPDX-License-Identifier: AGPL-3.0-only

import { ModelSelect } from "@/components/shared/ModelSelect";
import { Button, Divider, Input, InputNumber, theme } from "antd";
import { Plus, Trash2 } from "lucide-react";
import React from "react";
import { useTranslation } from "react-i18next";
import type { LlmClassifierNode, WorkflowNode } from "../../types";
import { BasePropertyPanel } from "./BasePropertyPanel";
interface Props {
  node: WorkflowNode;
  onUpdate: (u: Partial<WorkflowNode>) => void;
  onDelete: () => void;
}
export const LlmClassifierPropertyPanel: React.FC<Props> = ({ node, onUpdate, onDelete }) => {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const n = node as unknown as LlmClassifierNode; // SAFE: WorkflowNode union narrowed to specific node type via config field access
  const c = n.config || { categories: [], prompt: "", model: "", inputVar: "", outputVar: "" };
  const sc = (k: string, v: unknown) => onUpdate({ config: { ...c, [k]: v } });
  const labelStyle: React.CSSProperties = { color: token.colorTextTertiary, fontSize: 12 };
  const hintStyle: React.CSSProperties = {
    color: token.colorTextTertiary,
    fontSize: 11,
    marginTop: 2,
  };
  // 节点级供应商通路（2026-09-20 裁决）：`ModelSelect` 写回的是 `providerId::modelId`
  // 复合值，后端 `resolve_model_for_node` 会把它拆成「供应商 + 模型」两部分
  // （见 dao/src/repo/provider.rs::split_node_model）——即**允许节点跨供应商**。
  // 留空则沿用 __workflow_provider_id__ → profile.suggestedProviderId → 项目默认。
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div>
        <label style={labelStyle}>{t("workflow.nodeConfig.llmInputVar")}</label>
        <Input value={c.inputVar} onChange={(e) => sc("inputVar", e.target.value)} size="small" />
      </div>
      <div>
        <label style={labelStyle}>{t("workflow.nodeConfig.llmPrompt")}</label>
        <Input.TextArea
          value={c.prompt}
          onChange={(e) => sc("prompt", e.target.value)}
          rows={3}
          size="small"
        />
      </div>
      <div>
        <label style={labelStyle}>
          {t("workflow.nodeConfig.llmCategories", { count: c.categories.length })}
        </label>
        <div style={{ display: "flex", gap: 4, marginBottom: 4, flexWrap: "wrap" }}>
          {c.categories.map((cat, i) => (
            // FIXME: categories 是字符串数组且可编辑/删除/排序，无稳定唯一标识
            <span
              key={`cat-${i}`}
              style={{
                display: "flex",
                gap: 2,
                alignItems: "center",
                background: token.colorFillSecondary,
                borderRadius: 4,
                padding: "0 4px",
                fontSize: 11,
              }}
            >
              <Input
                size="small"
                value={cat}
                onChange={(e) => {
                  const cats = [...c.categories];
                  cats[i] = e.target.value;
                  sc("categories", cats);
                }}
                style={{ width: 60, fontSize: 11 }}
              />
              <Button
                type="text"
                size="small"
                danger
                icon={<Trash2 size={10} />}
                onClick={() => sc("categories", c.categories.filter((_: unknown, j: number) => j !== i))}
              />
            </span>
          ))}
        </div>
        <Button
          size="small"
          icon={<Plus size={12} />}
          onClick={() => sc("categories", [...c.categories, `category_${c.categories.length + 1}`])}
        >
          {t("workflow.nodeConfig.llmAddCategory")}
        </Button>
      </div>
      <Divider style={{ margin: "8px 0" }} />
      <div>
        <label style={labelStyle}>{t("workflow.nodeConfig.llmModel")}</label>
        <ModelSelect
          value={c.model || undefined}
          onChange={(value) => sc("model", value || "")}
          placeholder={t("workflow.props.selectModel")}
          allowClear
          style={{ width: "100%" }}
        />
        <div style={hintStyle}>{t("workflow.nodeConfig.llmModelHint")}</div>
      </div>
      <div>
        <label style={labelStyle}>{t("workflow.nodeConfig.llmConfidenceThreshold")}</label>
        <InputNumber
          value={c.confidenceThreshold}
          onChange={(value) => sc("confidenceThreshold", value ?? undefined)}
          min={0}
          max={1}
          step={0.05}
          size="small"
          style={{ width: "100%" }}
          placeholder="0.5"
        />
        <div style={hintStyle}>{t("workflow.nodeConfig.llmConfidenceThresholdHint")}</div>
      </div>
      <div>
        <label style={labelStyle}>{t("workflow.nodeConfig.llmFallbackLabel")}</label>
        <Input
          value={c.fallbackLabel ?? ""}
          onChange={(e) => sc("fallbackLabel", e.target.value || undefined)}
          size="small"
        />
        <div style={hintStyle}>{t("workflow.nodeConfig.llmFallbackHint")}</div>
      </div>
      <Divider style={{ margin: "8px 0" }} />
      <BasePropertyPanel node={node} onUpdate={onUpdate} onDelete={onDelete} />
    </div>
  );
};
