// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 变量控件（数值 / 通用）—— 原先在多个 ConfigPanel 各有一份**本地副本**，2026-09-14 收敛到本文件。
 *
 * ## 收敛范围
 * - `NumberControl`：`DemandDiscoveryConfigPanel` / `LiteraryCreationConfigPanel` /
 *   `StockAnalysisConfigPanel` 三份 → 本文件**一份**。
 *   其中 LiteraryCreation 那份**只有 `InputNumber` 且硬编码 `min={0}`**（无滑杆），
 *   另两份走 `inferNumberBounds` 动态量程 + 滑杆。统一为**动态量程**：
 *   滑杆粗调、`InputNumber` 精调，且 `InputNumber` 自身**不设 `min`/`max`**，
 *   被量程挡住的精确值仍可直接键入（见 `numberBounds.ts` 的两条不变量）。
 * - `VariableControl`：上述三个面板的三份。逻辑同构（`varTypeOf` 分派 boolean / enum /
 *   number / 文本），唯一差异是默认文本输入框的 `maxWidth`（220 / 200 / 180）→ 统一取 **220**。
 *
 * ## 为什么 `CodeRefactorConfigPanel` 的同名控件**不合并**
 * 它是**同名不同义**：变量类型是 `RefactorVariable`（自带 `type` / `min` / `max` / `options`，
 * 不走 `varTypeOf` 的 snake_case 兼容层），且需要 `disabled` 与「按变量名翻译 enum label」
 * 两个本文件没有的能力。强行合并会把 OPC 面板的行为一起改掉 ⇒ 保留其本地实现。
 */
import type { Variable } from "@/components/workflow/types";
import { varTypeOf } from "@/lib/workflowVariables";
import { Input, InputNumber, Select, Slider, Switch } from "antd";
import { useTranslation } from "react-i18next";
import { inferNumberBounds } from "./numberBounds";
import { parseEnumOptions } from "./parseEnumOptions";

/**
 * 数值控件 —— 滑杆量程按**默认值量级**推断，不用当前值（避免拖动时量程抖动）。
 */
export function NumberControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  const { t } = useTranslation();
  const desc = t(v.description ?? "");
  const hasPct = desc.includes("%");
  const val = Number(value ?? 0);
  const bounds = inferNumberBounds(v);
  return (
    <span className="sacp-number">
      <Slider
        min={bounds.min}
        max={bounds.max}
        step={bounds.step}
        className="sacp-number-slider"
        value={Math.min(Math.max(val, bounds.min), bounds.max)}
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

/**
 * 变量控件 —— 按变量类型分派到对应输入控件。
 */
export function VariableControl({ v, value, onChange }: {
  v: Variable;
  value: unknown;
  onChange: (name: string, val: unknown) => void;
}) {
  const { t } = useTranslation();
  const desc = t(v.description ?? "");
  // 必须走 varTypeOf：DB 来源变量的键是 snake_case 的 var_type，直接读 v.varType
  // 恒为 undefined → 所有控件静默退化成纯文本框。
  switch (varTypeOf(v)) {
    case "boolean":
      return <Switch checked={!!value} onChange={(c) => onChange(v.name, c)} />;
    case "enum": {
      const options = parseEnumOptions(desc);
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
          style={{ maxWidth: 220 }}
          value={String(value ?? "")}
          onChange={(e) => onChange(v.name, e.target.value)}
        />
      );
  }
}
