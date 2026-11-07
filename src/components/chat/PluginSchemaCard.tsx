// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 聊天消息侧的 **schema 卡片**（B8-1 实现半边；终局形态见 `PLAN-shell-plus-plugins.md` §93 / §102）。
 *
 * 一句话职责：调用方只交 `(schemaId, dataContext)`，宿主从动态 UI 库取出 schema，
 * 用与页面侧同一条渲染链画出来 —— 主 origin 里**不出现任何插件 React 代码**，
 * 插件只交「声明 + 数据」（§93 的安全边界：第三方代码能摸 `window` / Tauri IPC）。
 *
 * ── 为什么渲染的是 `DynamicUIRenderer` 而不是 `DynamicUIStandalone` ────────────────
 *
 * 页面侧（`PluginPageHost.tsx:108` 的 schema 分支）用 `DynamicUIStandalone`，因为页面只认 schemaId。
 * 卡片必须把**消息带来的数据投影**注入渲染上下文，而 `DynamicUIStandalone` 今天没有 `dataContext`
 * 入参（它自己用表单持久化数据 + `_schemaId/_instanceKey` 拼上下文，见 `DynamicUIStandalone.tsx:151`）。
 * `DynamicUIRenderer` 正是它内部那棵树（`DynamicUIStandalone.tsx:169` 把 mergedContext 传下去）
 * ⇒ 这里只是少走一层包装，**不是第二套渲染实现**：组件注册表、数据绑定、动作路由、错误边界
 * 全部还是 `src/lib/dynamicUI/**` + `src/components/dynamicUI/**` 那一份。
 * `PLAN-shell-plus-plugins.md` §102.1 记的形态是 `<DynamicUIStandalone schemaId dataContext>`；
 * 等动态 UI 侧补上该入参，本文件可原地切回，外层契约不变。
 *
 * ── 守住的三条 ────────────────────────────────────────────────────────────────
 *
 * 1. **插件不交渲染代码**（§93）：本文件没有、也不许有指向插件产物的 import；
 *    数据只经 `dataContext` 进 schema，形状由插件与 schema 自己对齐，宿主不校验（§102.4-2：
 *    校验等于第二套 DTO）。
 * 2. **取不到 schema 就是明确的错误态**：落 `dynamicUI.schemaNotFound` 的既有 11 语言译文，
 *    后端原串作 description —— 不白屏、不把异常抛到消息渲染树（渲染期错误另由
 *    `DynamicUIRenderer` 内部的 SchemaErrorBoundary 兜住）。
 * 3. **动作回流只走 `onAction`**：卡片不认知插件身份、不新开 IPC 面。消息侧的动作桥
 *    （交给 `usePluginStore.runUiAction`）不在本文件，传了 `onAction` 就透传，没传就维持
 *    动态 UI 既有行为。
 *
 * 本文件不参与插件发现（清单里有哪些卡片由插件声明面负责），也不读写 `Message` 的字段 ——
 * 消息侧怎么把卡片规格递进来，见同目录 `PluginSchemaCardSlot.tsx`。
 */

import { DynamicUIRenderer } from "@/components/dynamicUI/DynamicUIRenderer";
import { SchemaIdContext } from "@/components/dynamicUI/SchemaIdContext";
import { usePluginSchemaCard } from "@/hooks/usePluginSchemaCard";
import { permissionsForSchemaRecord } from "@/lib/dynamicUI/schemaActionPermissions";
import type { DynamicAction, DynamicUISchemaRecord, UISchema } from "@/types";
import { Alert, Spin } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 卡片契约：宿主只认这两个字段（外加一个纯展示的标题）。
 *
 * - `schemaId`：动态 UI 库里已收敛的 schema id（插件安装/启用时由 `schema.json` 注册，`owner_id = pluginId`）。
 * - `dataContext`：交给 schema 的数据投影。**键名与 schema 内数据节点的 id 对齐** ——
 *   `Table` 节点 `id = "rows"` 时读 `dataContext.rows`（`resolveDynamicArray` 的口径，
 *   `src/lib/dynamicUI/utils.ts:330`），其余同理。空对象与缺省等价。
 */
export interface PluginSchemaCardSpec {
  schemaId: string;
  dataContext?: Record<string, unknown>;
  /** 卡片标题：插件给的用户可读文案，不经宿主 11 语言 key（自带数据一起进来）。 */
  title?: string;
}

export interface PluginSchemaCardProps {
  spec: PluginSchemaCardSpec;
  onAction?: (action: DynamicAction) => void;
  style?: React.CSSProperties;
}

/**
 * 一张 schema 卡片。加载态 / 错误态与 `DynamicUIStandalone` 同形（`Spin` + `Alert`），
 * 只是尺寸取小一号 —— 它嵌在消息气泡里，不是整页。
 */
export function PluginSchemaCard({ spec, onAction, style }: PluginSchemaCardProps) {
  const { t } = useTranslation();
  const getSchema = usePluginSchemaCard();
  const [schema, setSchema] = useState<UISchema | null>(null);
  // 记录本身留一份：权限面要从**这张卡自己的记录**判来源，不能靠全局 `currentSchema` 回查
  // （同屏多张卡片时后加载者会覆盖它 ⇒ 先渲染的那张落到 undefined ⇒ 不受限）。
  const [record, setRecord] = useState<DynamicUISchemaRecord | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    setSchema(null);
    setRecord(null);
    setError(null);
    void (async () => {
      try {
        const loaded = await getSchema(spec.schemaId);
        if (!mounted) {
          return;
        }
        // 与 DynamicUIStandalone.tsx:59 同一口径：库里存的是 schemaJson 串，解析失败按加载失败处理
        setSchema(JSON.parse(loaded.schemaJson) as UISchema);
        setRecord(loaded);
      } catch (e) {
        if (mounted) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    })();
    return () => {
      mounted = false;
    };
  }, [spec.schemaId, getSchema]);

  // 来源 → 动作权限面：插件来源注入受限面，其余（user/ai/builtin）undefined 即不收紧。
  const actionPermissions = useMemo(() => permissionsForSchemaRecord(record), [record]);

  // SchemaIdContext 与页面侧同款：表单类节点靠它定位 schema（插件页也是这么包的）
  const schemaIdValue = useMemo(() => ({ schemaId: spec.schemaId }), [spec.schemaId]);

  return (
    <div data-plugin-schema-card={spec.schemaId} style={{ marginTop: 8, ...style }}>
      {spec.title
        ? (
          <div style={{ fontWeight: 500, marginBottom: 6 }}>
            {spec.title}
          </div>
        )
        : null}
      {error
        ? (
          <Alert
            type="error"
            showIcon
            title={t("dynamicUI.schemaNotFound")}
            description={error}
          />
        )
        : schema
        ? (
          <SchemaIdContext.Provider value={schemaIdValue}>
            <DynamicUIRenderer
              schema={schema}
              dataContext={spec.dataContext}
              onAction={onAction}
              actionPermissions={actionPermissions}
            />
          </SchemaIdContext.Provider>
        )
        : (
          <div style={{ padding: 8 }}>
            <Spin size="small" />
          </div>
        )}
    </div>
  );
}
