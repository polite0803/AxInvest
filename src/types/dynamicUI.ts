// SPDX-License-Identifier: AGPL-3.0-only

import type React from "react";

// ── 组件类型枚举 ──
export type DynamicComponentType =
  | "Container"
  | "Row"
  | "Column"
  | "Grid"
  | "Card"
  | "Tabs"
  | "Accordion"
  | "Form"
  | "Input"
  | "Number"
  | "Select"
  | "DatePicker"
  | "Switch"
  | "Checkbox"
  | "Radio"
  | "Textarea"
  | "Table"
  | "Chart"
  | "List"
  | "Dashboard"
  | "CodeEditor"
  | "FilePreview"
  | "Markdown"
  | "Image"
  | "Button"
  | "Text"
  | "Divider"
  | "Progress"
  | "Tag"
  | "Tree"
  | "Timeline";

// ── 数据源配置 ──
export interface DataSourceConfig {
  /** 数据源类型 */
  type: "store" | "api" | "static" | "agent-generated";
  /** 各类型对应的配置 */
  config: Record<string, unknown>;
  /** 轮询间隔 ms，0 表示不轮询 */
  polling?: number;
}

// ── 事件处理器 ──
export interface EventHandler {
  /** 触发时机 */
  trigger: "onClick" | "onChange" | "onSubmit" | "onMount" | "onUnmount";
  /** 触发后执行的动作列表 */
  actions: DynamicAction[];
}

// ── 动态动作（兼容 ActionRouter 体系） ──
export interface DynamicAction {
  type:
    | "invoke"
    | "navigate"
    | "emit"
    | "store"
    | "function"
    | "chain"
    | "update-schema";
  config: Record<string, unknown>;
}

// ── 条件渲染规则 ──
export interface ConditionalRule {
  field: string;
  operator:
    | "eq"
    | "neq"
    | "gt"
    | "gte"
    | "lt"
    | "lte"
    | "in"
    | "contains"
    | "exists"
    | "empty";
  value?: unknown;
}

export type ConditionalDisplay =
  | ConditionalRule[]
  | {
    logic: "and" | "or";
    rules: ConditionalDisplay[];
    not?: boolean;
  };

// ── 语义化增强：重要性等级 ──
export type DynamicImportance = "low" | "medium" | "high" | "critical";

// ── 语义化增强：组件运行状态 ──
export type DynamicStatus = "pending" | "ready" | "error" | "loading";

// ── UI Schema 顶层结构 ──
export interface UISchema {
  /** Schema 版本号 */
  version: string;
  /** 组件唯一标识 */
  id: string;
  /** 组件类型 */
  type: DynamicComponentType;
  /** 组件属性 */
  props: Record<string, unknown>;
  /** 子组件 */
  children?: UISchema[];
  /** 数据源配置 */
  dataSource?: DataSourceConfig;
  /** 事件处理器 */
  events?: EventHandler[];
  /** 条件显示规则（支持AND/OR/NOT逻辑组合） */
  conditionalDisplay?: ConditionalDisplay;
  /** 样式覆盖 */
  style?: Record<string, string | number>;
  /** 语义化：重要性等级（用于排序和渲染优先级） */
  importance?: DynamicImportance;
  /** 语义化：运行状态（用于状态徽章显示） */
  status?: DynamicStatus;
  /** 语义化：fallback Schema（当前组件渲染出错时的替代方案） */
  fallback?: UISchema;
}

// ── 组件注册表项 ──
export interface ComponentRegistryEntry {
  /** 组件类型 */
  type: DynamicComponentType;
  /** React 组件 */
  component: React.ComponentType<DynamicUIProps>;
  /** 组件分类 */
  category: "container" | "data-display" | "form" | "media" | "misc";
  /** 显示标签 */
  label: string;
  /** 默认属性 */
  defaultProps?: Record<string, unknown>;
}

// ── DynamicUIRenderer 接收的 props ──
export interface DynamicUIProps {
  /** UI Schema */
  schema: UISchema;
  /** 外部注入的数据上下文 */
  dataContext?: Record<string, unknown>;
  /** 动作回调 */
  onAction?: (action: DynamicAction) => void;
  /** 子节点 */
  children?: React.ReactNode;
}

// ── Schema 校验结果 ──
export interface SchemaValidationResult {
  /** 是否通过校验 */
  valid: boolean;
  /** 校验错误列表 */
  errors: SchemaValidationError[];
}

export interface SchemaValidationError {
  /** 错误路径（如 root.children[0].props） */
  path: string;
  /** 错误描述 */
  message: string;
}

// ── 有效的 DynamicComponentType 集合 ──
export const VALID_DYNAMIC_COMPONENT_TYPES: ReadonlySet<string> = new Set<DynamicComponentType>([
  "Container",
  "Row",
  "Column",
  "Grid",
  "Card",
  "Tabs",
  "Accordion",
  "Form",
  "Input",
  "Number",
  "Select",
  "DatePicker",
  "Switch",
  "Checkbox",
  "Radio",
  "Textarea",
  "Table",
  "Chart",
  "List",
  "Dashboard",
  "CodeEditor",
  "FilePreview",
  "Markdown",
  "Image",
  "Button",
  "Text",
  "Divider",
  "Progress",
  "Tag",
  "Tree",
  "Timeline",
]);

// ── 组件类型 → 必填 props 映射（用于 Schema 校验） ──
export const COMPONENT_REQUIRED_PROPS: Readonly<
  Record<DynamicComponentType, string[]>
> = {
  Container: [],
  Row: [],
  Column: [],
  Grid: ["columns"],
  Card: [],
  Tabs: [],
  Accordion: [],
  Form: [],
  Input: [],
  Number: [],
  Select: [],
  DatePicker: [],
  Switch: [],
  Checkbox: [],
  Radio: [],
  Textarea: [],
  Table: ["columns"],
  Chart: ["chartType"],
  List: [],
  Dashboard: ["items"],
  CodeEditor: [],
  FilePreview: [],
  Markdown: [],
  Image: [],
  Button: [],
  Text: [],
  Divider: [],
  Progress: [],
  Tag: [],
  Tree: ["treeData"],
  Timeline: ["items"],
};

// ── 持久化 Schema 相关类型 ──

/**
 * Schema 来源维度（后端 `dynamic_ui_schemas.origin`）。
 *
 * ⚠ 本字段在提交 `43803e5ac` 后**已变为安全锚点**：它回答「这行是谁注册的、可不可信」，
 * 由下面的 `TRUSTED_SCHEMA_ORIGINS` 与后端同名常量做 fail-closed 归属判定。
 * 与 `isBuiltin`（二值、删除 / 修改守卫）仍职责分离，但它**不再是「纯展示元数据」**，
 * 也不该再被当作展示字段用 —— 「内容是不是 AI 生成的」这类展示语义请另建展示字段，
 * 复用 `origin` 承载等于让一个字段同时管授权判定与 UI 呈现，是下一次权限绕过的成因。
 */
export type DynamicUISchemaOrigin = "builtin" | "user" | "ai" | "plugin";

/**
 * 「可信来源」白名单（fail-closed 判据的白名单侧）。
 *
 * 权威锚点在后端 `src-tauri/src/commands/dynamic_ui.rs` 的同名常量
 * `TRUSTED_SCHEMA_ORIGINS`（提交 `43803e5ac` 落地），两侧集合必须逐字相同 ——
 * 漂移就会出现「前端放行、后端拒」或反之的分叉。刻意**不列 `plugin`**：
 * `plugin` / 空串 / 未知字面量一律落到拒绝侧，因为「未知值」最坏情况下就是「插件写的」，
 * 放行未知值与 default allow 等价。
 *
 * 两个消费方（共用这一份词表，避免第三处重复定义）：
 * - `src/lib/dynamicUI/schemaActionPermissions.ts` ⇒ 决定是否注入受限动作权限面；
 * - `src/lib/browserMock.ts` ⇒ dev mock 的 update / delete 归属闸门，必须与真机同判据，
 *   否则 `npm run dev` 下能改删真机拒掉的行，这类缺陷在开发期完全看不见。
 *
 * `"ai"` 留在白名单里**不是给新代码的出口**，只为存量行兜底：`43803e5ac` 起创建面把
 * `origin` 硬写成 `"user"`（前端唯一传 `"ai"` 的入参 `origin: generatedByAI ? "ai"` 已在
 * `975652a17` 摘除），此后**没有任何写面**能产出 `"ai"` 档 —— 全仓写入点只剩
 * `create_dynamic_ui_schema`（`"user"`）与插件安装通道（`"plugin"`）。而 `43803e5ac` 之前
 * 后端采信客户端自报的 `origin`，真实库里确有 `"ai"` 的历史行。把 `"ai"` 从白名单摘掉，
 * 那些行会被八个归属闸门入口一律拒绝（`load_schema_for_command` 7 处 + `unpin` 内联判定），
 * 又因 `origin != "plugin"` 不归插件撤销通道管，
 * 变成用户既改不动也删不掉的孤儿 ⇒ 保留它挡不住任何新攻击面（新代码造不出该字面量），
 * 放过的只有历史数据。**要恢复「AI 生成」的展示能力请新增展示字段，不要回写 `origin`。**
 */
export const TRUSTED_SCHEMA_ORIGINS: ReadonlySet<string> = new Set(["builtin", "user", "ai"]);

export interface DynamicUISchemaRecord {
  id: string;
  title: string;
  description: string;
  schemaJson: string;
  category: string;
  tags: string[];
  version: string;
  isBuiltin: boolean;
  origin: DynamicUISchemaOrigin;
  /** 来源归属：`origin === "plugin"` 时为 pluginId，其余为空串 */
  ownerId: string;
  createdAt: string;
  updatedAt: string;
}

export interface DynamicUIFormDataRecord {
  id: string;
  schemaId: string;
  formDataJson: string;
  instanceKey: string;
  updatedAt: string;
}

export interface CreateDynamicUISchemaParams {
  title: string;
  description: string;
  schemaJson: string;
  category: string;
  tags: string[];
  /**
   * 已废弃：后端与 dev mock **均忽略此入参**，权威 `origin` 由宿主写入 ——
   * `create_dynamic_ui_schema` 在函数体里把 `origin` 硬写成 `"user"`
   * （`src-tauri/src/commands/dynamic_ui.rs` 的 origin 锚，提交 `43803e5ac`），
   * dev mock 同判据。传 `"ai"` / `"plugin"` 不会改变落库来源，也不得再用于分支。
   * 字段本身保留，仅为与 Rust 侧 `CreateSchemaRequest` 的 `#[serde(default)]` 废弃入参形状对齐。
   */
  origin?: DynamicUISchemaOrigin;
  /** 已废弃：同 `origin`，值被忽略，落库 `ownerId` 恒为空串（非插件行的归属标识本就无区分度）。 */
  ownerId?: string;
}

export interface UpdateDynamicUISchemaParams {
  title?: string;
  description?: string;
  schemaJson?: string;
  category?: string;
  tags?: string[];
  /** 语义化版本号（可选），不传则自动递增 patch */
  version?: string;
  /** 变更说明 */
  changeLog?: string;
}

export interface SaveDynamicUIFormDataParams {
  schemaId: string;
  formDataJson: string;
  instanceKey?: string;
}

// ── 导航钉入配置类型 ──

export interface DynamicUIPinRecord {
  schemaId: string;
  title: string;
  groupName: string;
  position: number;
  createdAt: string;
  updatedAt: string;
}

export interface PinDynamicUISchemaParams {
  schemaId: string;
  title: string;
  groupName: string;
  position?: number;
}

export interface UpdateDynamicUIPinParams {
  title?: string;
  groupName?: string;
  position?: number;
}

// ── 版本管理类型 ──

export interface DynamicUISchemaVersion {
  id: number;
  schemaId: string;
  version: string;
  title: string;
  description: string;
  schemaJson: string;
  category: string;
  tags: string[];
  changeLog: string;
  createdAt: number;
}

export interface ListVersionsResponse {
  versions: DynamicUISchemaVersion[];
  currentVersion: string;
}
