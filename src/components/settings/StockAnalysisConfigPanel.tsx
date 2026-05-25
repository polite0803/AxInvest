import type { Variable, WorkflowTemplateInput, WorkflowTemplateResponse } from "@/components/workflow/types";
import { invoke } from "@/lib/invoke";
import { Button, Input, InputNumber, message, Select, Slider, Switch, Tag, theme } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "./SettingsGroup";

const TEMPLATE_ID = "stock-analysis";

function extractPrefix(name: string): string {
  const idx = name.indexOf("_");
  return idx > 0 ? name.slice(0, idx) : "";
}

interface VariableGroup {
  prefix: string;
  i18nKey: string;
  vars: Variable[];
}

function groupVariables(variables: Variable[], prefixes: Record<string, string>): {
  grouped: VariableGroup[];
  ungrouped: Variable[];
} {
  const map = new Map<string, Variable[]>();
  const ungrouped: Variable[] = [];
  for (const v of variables) {
    const pre = extractPrefix(v.name);
    if (pre && prefixes[pre]) {
      map.get(pre)?.push(v) ?? map.set(pre, [v]);
    } else {
      ungrouped.push(v);
    }
  }
  const grouped: VariableGroup[] = [];
  for (const [prefix, i18nKey] of Object.entries(prefixes)) {
    if (map.has(prefix)) {
      grouped.push({ prefix, i18nKey, vars: map.get(prefix)! });
    }
  }
  return { grouped, ungrouped };
}

function parseEnumOptions(desc?: string): string[] {
  if (!desc) { return []; }
  const match = desc.match(/: (.+)/);
  if (match) { return match[1].split(/\s*\/\s*/).map((s) => s.trim()); }
  return [];
}

function inferStep(v: Variable): number {
  if (v.description?.includes("温度")) { return 0.1; }
  return 1;
}

interface Props {
  showVendorHealth?: boolean;
  vendorHealth?: Record<string, "ok" | "fail" | "pending">;
  checkingVendors?: boolean;
  onCheckVendor?: (name: string) => void;
  onCheckAllVendors?: () => void;
}

/** number 控件 — 窄屏竖排，宽屏横排 */
function NumberControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  const hasPct = v.description?.includes("%") ?? false;
  const val = Number(value ?? 0);
  return (
    <span className="sacp-number">
      <Slider
        min={0}
        max={v.description?.includes("温度") ? 2 : 100}
        step={inferStep(v)}
        className="sacp-number-slider"
        value={val}
        onChange={(v2) => onChange(v.name, v2)}
      />
      <InputNumber
        size="small"
        className="sacp-number-input"
        value={val}
        suffix={hasPct ? "%" : undefined}
        onChange={(v2) => v2 != null && onChange(v.name, v2)}
      />
    </span>
  );
}

function VariableControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  switch (v.var_type) {
    case "boolean":
      return <Switch checked={!!value} onChange={(c) => onChange(v.name, c)} />;
    case "enum": {
      const options = parseEnumOptions(v.description);
      return (
        <Select
          size="small"
          style={{ width: 140 }}
          value={String(value ?? "")}
          onChange={(val) => onChange(v.name, val)}
          options={options.map((o) => ({ value: o, label: o }))}
        />
      );
    }
    case "number":
      return <NumberControl v={v} value={value} onChange={onChange} />;
    default:
      return (
        <Input
          size="small"
          style={{ maxWidth: 180 }}
          value={String(value ?? "")}
          onChange={(e) => onChange(v.name, e.target.value)}
        />
      );
  }
}

export function StockAnalysisConfigPanel(props: Props) {
  const { showVendorHealth, vendorHealth, checkingVendors, onCheckVendor, onCheckAllVendors } = props;
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [template, setTemplate] = useState<WorkflowTemplateResponse | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const prefixes = useMemo(() => ({
    vendor: t("stockAnalysis.settings.group.vendor"),
    agent: t("stockAnalysis.settings.group.agent"),
    tool: t("stockAnalysis.settings.group.tool"),
    scoring: t("stockAnalysis.settings.group.scoring"),
    rule: t("stockAnalysis.settings.group.rule"),
    pos: t("stockAnalysis.settings.group.pos"),
    value: t("stockAnalysis.settings.group.value"),
    monitor: t("stockAnalysis.settings.group.monitor"),
  } as Record<string, string>), [t]);

  useEffect(() => {
    invoke<WorkflowTemplateResponse | null>("get_workflow_template", { id: TEMPLATE_ID })
      .then((rsp) => {
        if (rsp) {
          setTemplate(rsp);
          const map: Record<string, unknown> = {};
          for (const v of rsp.variables) { map[v.name] = v.value; }
          setValues(map);
        }
      })
      .catch(() => message.error(t("stockAnalysis.settings.loadFailed")))
      .finally(() => setLoading(false));
  }, [t]);

  const { grouped, ungrouped } = useMemo(() => {
    if (!template) { return { grouped: [], ungrouped: [] as Variable[] }; }
    return groupVariables(template.variables, prefixes);
  }, [template, prefixes]);

  const handleChange = (name: string, val: unknown) => {
    setValues((prev) => ({ ...prev, [name]: val }));
  };

  const handleSave = async () => {
    if (!template) { return; }
    setSaving(true);
    const updatedVars = template.variables.map((v) => ({ ...v, value: values[v.name] ?? v.value }));
    const input: WorkflowTemplateInput = {
      name: template.name,
      description: template.description,
      icon: template.icon,
      tags: template.tags,
      trigger_config: template.trigger_config,
      nodes: template.nodes,
      edges: template.edges,
      input_schema: template.input_schema,
      output_schema: template.output_schema,
      variables: updatedVars,
      error_config: template.error_config,
    };
    try {
      await invoke<boolean>("update_workflow_template", { id: TEMPLATE_ID, input });
      message.success(t("stockAnalysis.settings.saveSuccess"));
    } catch {
      message.error(t("stockAnalysis.settings.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div style={{ textAlign: "center", padding: 24, color: token.colorTextQuaternary }}>{t("common.loading")}</div>
    );
  }

  const rowStyle = { padding: "4px 0" };

  const renderGroup = (title: string, vars: Variable[], isVendor: boolean) => (
    <SettingsGroup
      key={title}
      title={title}
      extra={isVendor && onCheckAllVendors
        ? (
          <Button size="small" loading={checkingVendors} onClick={onCheckAllVendors}>
            {t("stockAnalysis.settings.checkHealth")}
          </Button>
        )
        : undefined}
    >
      <div className="sacp-vars">
        {vars.map((v) => (
          <div key={v.name} style={rowStyle} className="flex items-center justify-between sacp-row">
            <span className="sacp-var-label" style={{ fontSize: 13, color: token.colorText }}>
              {v.description ?? v.name}
            </span>
            <span style={{ display: "inline-flex", alignItems: "center", gap: 8, flexShrink: 0, marginLeft: 16 }}>
              <VariableControl v={v} value={values[v.name]} onChange={handleChange} />
              {isVendor && onCheckVendor && (
                <Tag
                  color={vendorHealth?.[v.name] === "ok"
                    ? "success"
                    : vendorHealth?.[v.name] === "fail"
                    ? "error"
                    : "default"}
                  style={{ cursor: "pointer" }}
                  onClick={() => onCheckVendor(v.name)}
                >
                  {vendorHealth?.[v.name] === "ok"
                    ? t("stockAnalysis.settings.connected")
                    : vendorHealth?.[v.name] === "fail"
                    ? t("stockAnalysis.settings.disconnected")
                    : t("stockAnalysis.settings.check")}
                </Tag>
              )}
            </span>
          </div>
        ))}
      </div>
    </SettingsGroup>
  );

  return (
    <div>
      {ungrouped.length > 0 && renderGroup(t("stockAnalysis.settings.general"), ungrouped, false)}
      {grouped.map((g) => renderGroup(g.i18nKey, g.vars, g.prefix === "vendor" && !!showVendorHealth))}
      <div style={{ display: "flex", justifyContent: "flex-end", paddingTop: 16 }}>
        <Button type="primary" loading={saving} onClick={handleSave}>
          {t("stockAnalysis.settings.saveConfig")}
        </Button>
      </div>
    </div>
  );
}
