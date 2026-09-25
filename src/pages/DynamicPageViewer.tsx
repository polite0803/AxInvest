// SPDX-License-Identifier: AGPL-3.0-only

import { DynamicUIRenderer } from "@/components/dynamicUI/DynamicUIRenderer";
import { SchemaIdContext } from "@/components/dynamicUI/SchemaIdContext";
import { RouteGuard } from "@/components/shared/RouteGuard";
import { useDynamicUIStore, usePluginStore } from "@/stores";
import type { DynamicAction, UISchema } from "@/types";
import { Result, Spin } from "antd";
import { useEffect, useMemo, useReducer } from "react";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";

function parseSchema(json: string): UISchema | null {
  try {
    return JSON.parse(json) as UISchema;
  } catch {
    return null;
  }
}

export function DynamicPageViewer() {
  const { schemaId } = useParams<{ schemaId: string }>();
  const { t } = useTranslation();
  const getSchema = useDynamicUIStore((s) => s.getSchema);

  type ViewState =
    | { kind: "loading" }
    | { kind: "error"; message: string }
    | { kind: "ready"; schema: UISchema; pluginId: string | null };

  type ViewAction =
    | { type: "start_load" }
    | { type: "load_error"; message: string }
    | { type: "load_ok"; schema: UISchema; pluginId: string | null }
    | { type: "reset" };

  function viewReducer(_state: ViewState, action: ViewAction): ViewState {
    switch (action.type) {
      case "start_load":
        return { kind: "loading" };
      case "load_error":
        return { kind: "error", message: action.message };
      case "load_ok":
        return { kind: "ready", schema: action.schema, pluginId: action.pluginId };
      case "reset":
        return { kind: "loading" };
    }
  }

  const [viewState, dispatch] = useReducer(viewReducer, { kind: "loading" });

  useEffect(() => {
    if (!schemaId) {
      return;
    }

    let cancelled = false;
    dispatch({ type: "start_load" });

    getSchema(schemaId)
      .then((record) => {
        if (cancelled) { return; }
        const parsed = parseSchema(record.schemaJson);
        if (!parsed) {
          dispatch({ type: "load_error", message: t("dynamicUIManager.invalidSchema") });
        } else {
          // 插件来源的 Schema 才做动作回流（`origin: "plugin"` 时 `ownerId` 即 pluginId）。
          dispatch({
            type: "load_ok",
            schema: parsed,
            pluginId: record.origin === "plugin" && record.ownerId ? record.ownerId : null,
          });
        }
      })
      .catch(() => {
        if (cancelled) { return; }
        dispatch({ type: "load_error", message: t("dynamicUI.schemaNotFound") });
      });

    return () => {
      cancelled = true;
    };
  }, [schemaId, getSchema, t]);

  const runUiAction = usePluginStore((s) => s.runUiAction);
  const readyPluginId = viewState.kind === "ready" ? viewState.pluginId : null;

  // 插件来源 Schema 的动作回流（PLAN §10.5-2）：`DynamicUIRenderer` 的 `onAction`
  // 是宿主感知动作的唯一通路（Button 等组件只经它），把它接到插件自己的 worker 进程。
  // 引用必须稳定 —— `DynamicUIRenderer` 的 onMount/onUnmount 副作用以 onAction 为依赖。
  const handleAction = useMemo(
    () => (readyPluginId ? (action: DynamicAction) => void runUiAction(readyPluginId, action) : undefined),
    [readyPluginId, runUiAction],
  );

  if (!schemaId) {
    return (
      <div style={{ padding: 48, textAlign: "center" }}>
        <Result status="404" title="404" subTitle={t("dynamicUI.schemaNotFound")} />
      </div>
    );
  }

  if (viewState.kind === "loading") {
    return (
      <div className="flex items-center justify-center h-full w-full" style={{ minHeight: 200 }}>
        <Spin size="large" />
      </div>
    );
  }

  const isError = viewState.kind === "error";
  const errorSubTitle = isError ? viewState.message : undefined;
  const readySchema = viewState.kind === "ready" ? viewState.schema : null;

  return (
    <RouteGuard allowed={readySchema !== null} subTitle={errorSubTitle}>
      {readySchema && (
        <div className="p-6" style={{ flex: 1, overflow: "auto" }}>
          <SchemaIdContext.Provider value={{ schemaId }}>
            <DynamicUIRenderer schema={readySchema} onAction={handleAction} />
          </SchemaIdContext.Provider>
        </div>
      )}
    </RouteGuard>
  );
}
