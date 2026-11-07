// SPDX-License-Identifier: AGPL-3.0-only

import { Tooltip } from "@/components/layout/Tooltip";
import { MemoryGraphView } from "@/components/memory/MemoryGraphView";
import { SourceManager } from "@/components/settings/SourceManager";
import { useLlmWikiStore } from "@/stores/feature/llmWikiStore";
import { useMemoryStore } from "@/stores/feature/memoryStore";
import { Button, theme } from "antd";
import { GitGraph } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

/**
 * LLM 知识库中心页（/knowledge 与 /llm-wiki 共用）。
 * 去掉冗余 header，让 SourceManager 自身承担标题栏，最大化利用垂直空间。
 *
 * 右上角浮动「学习图」入口：将原 /learning-graph 隐藏路由暴露给用户，
 * 作为知识源的衍生可视化能力（技能/记忆/洞察/实体关系图）。
 *
 * 页级注入（任务 #219，组件中立性门禁）：SourceManager 属 settings 域，
 * 不得直接引用 memory / llmWiki 域的 store 与组件；本页是其唯一消费方，
 * 由页面订阅两个 store 的所需切片并经 props 注入，记忆图视图同样经渲染函数注入。
 */
export function KnowledgeHubPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const navigate = useNavigate();

  // 选择器订阅：只取 SourceManager 实际消费的切片，动作引用在 zustand 中恒定
  const namespaces = useMemoryStore((s) => s.namespaces);
  const memoryLoading = useMemoryStore((s) => s.loading);
  const loadNamespaces = useMemoryStore((s) => s.loadNamespaces);
  const wikis = useLlmWikiStore((s) => s.wikis);
  const loadWikis = useLlmWikiStore((s) => s.loadWikis);
  const deleteWiki = useLlmWikiStore((s) => s.deleteWiki);

  return (
    <div className="kb-layout" data-testid="knowledge-hub" style={{ position: "relative" }}>
      <SourceManager
        memory={{ namespaces, loading: memoryLoading, loadNamespaces }}
        wiki={{ wikis, loadWikis, deleteWiki }}
        renderMemoryGraph={(onClose) => <MemoryGraphView onClose={onClose} />}
      />
      <Tooltip title={t("nav.learningGraph")} placement="left">
        <Button
          type="default"
          shape="circle"
          icon={<GitGraph size={16} />}
          onClick={() => navigate("/knowledge/learning")}
          aria-label={t("nav.learningGraph")}
          style={{
            position: "absolute",
            top: 8,
            right: 12,
            zIndex: 10,
            backgroundColor: token.colorBgElevated,
            boxShadow: token.boxShadowSecondary,
            borderColor: token.colorBorderSecondary,
          }}
        />
      </Tooltip>
    </div>
  );
}
