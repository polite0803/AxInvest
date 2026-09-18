// SPDX-License-Identifier: AGPL-3.0-only

import { WorkflowSettings } from "@/components/settings";
import { WorkflowEditor, WorkflowExecutor } from "@/components/workflow";
import type { WorkflowTemplateResponse } from "@/components/workflow/types";
import { withoutParams } from "@/lib/workspaceTabs";
import { ReactFlowProvider } from "@xyflow/react";
import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";

/**
 * 本页消费后须清理的查询参数。
 * ⚠ 只清这两个 —— 用 `setSearchParams({})` 会连工作台 Tab 参数 `ws` 一起抹掉，
 * 使 `/chat?ws=workflow&template=X` 跳转后地址栏退回 `/chat`（深链失效）。
 */
const WORKFLOW_PAGE_QUERY_PARAMS = ["template", "domain"] as const;

/**
 * 工作流页面：包含「我的工作流」与「市场」两个 Tab（由 WorkflowSettings 内部提供）。
 * 编辑器全屏模式：创建新或编辑现有时隐藏列表，直接展示编辑器。
 * 运行模式：打开执行面板（动态 UI 表单 + 实时执行结果）。
 */
export function WorkflowPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const [editingTemplateId, setEditingTemplateId] = useState<
    string | undefined
  >(undefined);
  const [isEditingSystem, setIsEditingSystem] = useState(false);
  const [isCreatingNew, setIsCreatingNew] = useState(false);
  const [runningTemplate, setRunningTemplate] = useState<
    WorkflowTemplateResponse | null
  >(null);
  const urlInitDoneRef = useRef(false);

  // URL query 参数初始化（仅页面挂载时执行一次）
  useEffect(() => {
    if (urlInitDoneRef.current) {
      return;
    }
    const template = searchParams.get("template");
    const domain = searchParams.get("domain");

    if (!template && !domain) {
      urlInitDoneRef.current = true;
      return;
    }

    urlInitDoneRef.current = true;

    if (template) {
      setEditingTemplateId(template);
    } else if (domain) {
      // 仅有 domain 参数时，进入创建模式
      setIsCreatingNew(true);
    }

    // 清理 URL 参数（保留 `ws`，见文件头常量的说明）
    setSearchParams((prev) => withoutParams(prev, WORKFLOW_PAGE_QUERY_PARAMS), { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 编辑器全屏模式：创建新或编辑现有时隐藏 Tabs
  if (isCreatingNew || editingTemplateId) {
    return (
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <ReactFlowProvider>
          <WorkflowEditor
            templateId={isCreatingNew ? undefined : editingTemplateId}
            isSystemTemplate={isEditingSystem}
            onClose={() => {
              setEditingTemplateId(undefined);
              setIsEditingSystem(false);
              setIsCreatingNew(false);
            }}
          />
        </ReactFlowProvider>
      </div>
    );
  }

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <WorkflowSettings
        onOpenEditor={(templateId?: string) => setEditingTemplateId(templateId)}
        onOpenSystemEditor={(templateId: string) => {
          setEditingTemplateId(templateId);
          setIsEditingSystem(true);
        }}
        onCreateNew={() => setIsCreatingNew(true)}
        onRunWorkflow={(template) => setRunningTemplate(template)}
      />
      {runningTemplate && (
        <WorkflowExecutor
          workflow={runningTemplate}
          open
          onClose={() => setRunningTemplate(null)}
        />
      )}
    </div>
  );
}
