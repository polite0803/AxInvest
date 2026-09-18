// i18n-exempt: 变量对象字段名兼容工具，技术字符串（var_type/is_secret），非用户可见 UI 文案。
//
// 工作流模板变量的「前后端字段名」兼容层。
//
// 背景：DB 中变量对象的字段名是 snake_case（`var_type` / `is_secret`）——后端
// `axagent_harness::workflow_types::Variable` 没有 `#[serde(rename_all = "camelCase")]`，
// 而前端 `Variable` 类型（`@/components/workflow/types`）用的是 camelCase
// （`varType` / `isSecret`）。
//
// 后果：任何从 `get_workflow_template` 读回来的模板变量，直接 `v.varType` 恒为
// `undefined` → 控件 switch 全部落到 default 分支（数字没有 Slider、布尔没有 Switch、
// 枚举没有 Select），界面静默退化成纯文本框。这是「后端有数据、前端读不到」的
// 典型静默失效，靠肉眼看不出来。
//
// 用法（唯一正确读法）：
//   varTypeOf(v)   ✓        v.varType   ✗
//   isSecretOf(v)  ✓        v.isSecret  ✗
//
// 写回模板时统一走 `toDbVariable()`，保证 serde 能解析（写 camelCase 会丢字段）。

import type { Variable } from "@/components/workflow/types";

/** DB 中变量对象的原始形态（snake_case） */
interface RawDbVariable {
  var_type?: string;
  is_secret?: boolean;
}

/**
 * 读取变量类型，兼容 DB 来源的 snake_case（`var_type`）与前端 camelCase（`varType`）。
 *
 * @returns `"number"` / `"boolean"` / `"enum"` / `"string"` 等；两者皆无时返回空串。
 */
export function varTypeOf(v: Variable): string {
  const raw = v as unknown as RawDbVariable;
  return v.varType ?? raw.var_type ?? "";
}

/**
 * 读取「是否为敏感变量（密钥类）」，兼容 snake_case 与 camelCase。
 *
 * 注意不能写成 `v.isSecret || raw.is_secret || false`：`false` 是合法值，
 * 用 `??` 逐级回退才能正确表达「显式 false」。
 */
export function isSecretOf(v: Variable): boolean {
  const raw = v as unknown as RawDbVariable;
  return v.isSecret ?? raw.is_secret ?? false;
}

/**
 * 把前端 camelCase 变量对象规范化为后端 snake_case 形式，用于模板写回。
 *
 * 返回类型仍标注为 `Variable`，以便直接喂给 `WorkflowTemplateInput.variables`；
 * 这是刻意的「类型上 camelCase、运行时 snake_case」——读取侧必须走
 * `varTypeOf` / `isSecretOf`，不要直接读字段。
 */
export function toDbVariable(v: Variable): Variable {
  return {
    name: v.name,
    var_type: varTypeOf(v),
    value: v.value,
    description: v.description,
    is_secret: isSecretOf(v),
  } as unknown as Variable;
}
