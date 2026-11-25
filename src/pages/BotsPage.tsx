// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `/bots` —— Bot 管理页（两栏：左列表 / 右详情与表单）。
 *
 * 组合层职责（见 `PLAN-bot-integration` T3「组件中立性」）：
 *   - 从 provider store 投影模型选项，经 props 注入 `BotEditor`（组件不直读别的 feature store）；
 *   - 会话动作（新建线程 / 打开线程）在本页完成，随后带 `?bot=` 跳到 `/chat`。
 *
 * 「进入某个 Bot 的会话」路由契约：`/chat?bot=<botId>`。
 * `bot` 已登记为 chat 专属查询参数（`CHAT_ONLY_QUERY_PARAMS`），WorkspaceHub 据此切到对话 Tab，
 * ChatPage 把 `botId` 传给 ChatSidebar 做归属过滤，新建会话也带该 `botId`。
 */

import { BotAssignmentsPanel } from "@/components/bot/BotAssignmentsPanel";
import { BotDepartmentModal } from "@/components/bot/BotDepartmentModal";
import { BotDetail } from "@/components/bot/BotDetail";
import { BotEditor, type BotEditorModelOption, type BotEditorValues } from "@/components/bot/BotEditor";
import { BotEmptyState } from "@/components/bot/BotEmptyState";
import { BotList } from "@/components/bot/BotList";
import { backendErrorText } from "@/lib/errorI18n";
import { countUnread, getNotifications } from "@/lib/notification";
import { message } from "@/lib/toast";
import { useBotStore, useConversationStore, useProviderStore } from "@/stores";
import { ensureBotInputRequestListener } from "@/stores/feature/botStore";
import type { Bot, BotDepartment, CreateBotInput, UpdateBotInput } from "@/types";
import { Alert, Button, Spin, theme } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { BotRoutinesPanel } from "./BotsPage/BotRoutinesPanel";

/** `providerId::modelId` → `{ providerId, modelId }`；空串表示不指定模型（两者同为 null）。 */
function parseModelPair(model: string): { providerId: string | null; modelId: string | null } {
  const sep = model.indexOf("::");
  if (sep <= 0) {
    return { providerId: null, modelId: null };
  }
  return { providerId: model.slice(0, sep), modelId: model.slice(sep + 2) };
}

/** Bot → 表单初值（模型用 `providerId::modelId` 表达）。 */
function botToEditorValues(bot: Bot): BotEditorValues {
  return {
    name: bot.name,
    icon: bot.icon ?? "",
    title: bot.title ?? "",
    soul: bot.soul,
    workdir: bot.workdir ?? "",
    approvalMode: bot.approvalMode,
    model: bot.providerId && bot.modelId ? `${bot.providerId}::${bot.modelId}` : "",
  };
}

export function BotsPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const navigate = useNavigate();

  const bots = useBotStore((s) => s.bots);
  const loading = useBotStore((s) => s.loading);
  const saving = useBotStore((s) => s.saving);
  const deleting = useBotStore((s) => s.deleting);
  const error = useBotStore((s) => s.error);
  const activeBotId = useBotStore((s) => s.activeBotId);
  const loadBots = useBotStore((s) => s.loadBots);
  const setActiveBot = useBotStore((s) => s.setActiveBot);
  const getBot = useBotStore((s) => s.getBot);
  const createBot = useBotStore((s) => s.createBot);
  const updateBot = useBotStore((s) => s.updateBot);
  const deleteBot = useBotStore((s) => s.deleteBot);

  const providers = useProviderStore((s) => s.providers);
  const fetchProviders = useProviderStore((s) => s.fetchProviders);

  // P2-a：Bot 未读角标数据源 —— localStorage 通知按 botId 计数；
  // CustomEvent（axagent:notification）触发重读保持角标新鲜。
  const [notificationVersion, setNotificationVersion] = useState(0);
  useEffect(() => {
    const bump = () => setNotificationVersion((v) => v + 1);
    window.addEventListener("axagent:notification", bump);
    return () => {
      window.removeEventListener("axagent:notification", bump);
    };
  }, []);

  const unreadByBot = useMemo(() => {
    // notificationVersion 仅作依赖驱动重算（eslint 兜新鲜度）
    void notificationVersion;
    const notifications = getNotifications();
    const map = new Map<string, number>();
    for (const bot of bots) {
      const n = countUnread(notifications, bot.id);
      if (n > 0) {
        map.set(bot.id, n);
      }
    }
    return map;
  }, [bots, notificationVersion]);

  const [editorOpen, setEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [editingBotId, setEditingBotId] = useState<string | null>(null);
  const [dismissedError, setDismissedError] = useState<string | null>(null);

  // P2-b：部门数据（分组侧栏 + 部门管理弹窗的数据源）
  const listBotDepartments = useBotStore((s) => s.listBotDepartments);
  const [departments, setDepartments] = useState<BotDepartment[]>([]);
  const [departmentModalOpen, setDepartmentModalOpen] = useState(false);
  const [editingDepartment, setEditingDepartment] = useState<BotDepartment | null>(null);
  const [departmentModalTab, setDepartmentModalTab] = useState<"basic" | "quota">("basic");

  // P4-b：待答请求（事件订阅 + 挂载时拉 pending；归零即撤徽章）。
  const inputRequests = useBotStore((s) => s.inputRequests) ?? [];
  const loadInputRequests = useBotStore((s) => s.loadInputRequests);
  useEffect(() => {
    void ensureBotInputRequestListener?.();
    if (typeof loadInputRequests === "function") {
      void loadInputRequests({ status: "pending" }).catch(() => {});
    }
  }, [loadInputRequests]);
  // B4-R：待答数按 botId 聚合（`Record<botId, number>` 逐行注入）；
  // 头部徽章仍用全量 pending 总数，与逐行计数同源、一次遍历。
  const { pendingInputCount, pendingByBot } = useMemo(() => {
    const byBot: Record<string, number> = {};
    let total = 0;
    for (const r of inputRequests) {
      if (r.status !== "pending") {
        continue;
      }
      total += 1;
      if (r.botId) {
        byBot[r.botId] = (byBot[r.botId] ?? 0) + 1;
      }
    }
    return { pendingInputCount: total, pendingByBot: byBot };
  }, [inputRequests]);

  const reloadDepartments = useCallback(() => {
    void listBotDepartments()
      .then((rows) => setDepartments(Array.isArray(rows) ? rows : []))
      .catch(() => {
        // 部门加载失败不阻断 Bot 列表（侧栏退化为无分组）
        setDepartments([]);
      });
  }, [listBotDepartments]);

  useEffect(() => {
    reloadDepartments();
  }, [reloadDepartments]);

  useEffect(() => {
    void loadBots();
  }, [loadBots]);

  useEffect(() => {
    if (providers.length === 0) {
      void fetchProviders();
    }
  }, [providers.length, fetchProviders]);

  // 首次加载或删除后：自动选中第一个 Bot（没有选中项时才写，避免抢走用户的选择）
  useEffect(() => {
    if (!activeBotId && bots.length > 0) {
      setActiveBot(bots[0].id);
    }
  }, [activeBotId, bots, setActiveBot]);

  const activeBot = useMemo(
    () => bots.find((b) => b.id === activeBotId) ?? bots[0] ?? null,
    [bots, activeBotId],
  );

  const editingBot = useMemo(
    () => (editorMode === "edit" && editingBotId ? bots.find((b) => b.id === editingBotId) ?? null : null),
    [editorMode, editingBotId, bots],
  );

  const modelOptions = useMemo<BotEditorModelOption[]>(
    () =>
      providers.flatMap((provider) =>
        provider.enabled
          ? provider.models
            .filter((model) => model.enabled)
            .map((model) => ({ value: `${provider.id}::${model.modelId}`, label: `${provider.name} / ${model.name}` }))
          : []
      ),
    [providers],
  );

  const editorInitialValues = useMemo(
    () => (editingBot ? botToEditorValues(editingBot) : undefined),
    [editingBot],
  );

  const openCreate = useCallback(() => {
    setEditorMode("create");
    setEditingBotId(null);
    setEditorOpen(true);
  }, []);

  // 编辑前拉一次最新快照（列表是挂载时的快照，可能已被 SOUL 写入或外部改动刷新过）
  const openEdit = useCallback(async () => {
    if (!activeBot) {
      return;
    }
    try {
      await getBot(activeBot.id);
    } catch {
      // 拉最新失败就用列表快照，编辑入口不因刷新失败而不可用
    }
    setEditorMode("edit");
    setEditingBotId(activeBot.id);
    setEditorOpen(true);
  }, [activeBot, getBot]);

  const handleSubmit = useCallback(
    async (values: BotEditorValues) => {
      try {
        if (editorMode === "edit" && editingBot) {
          const input: UpdateBotInput = {};
          if (values.name !== editingBot.name) {
            input.name = values.name;
          }
          if (values.icon !== (editingBot.icon ?? "")) {
            input.icon = values.icon;
          }
          if (values.title !== (editingBot.title ?? "")) {
            input.title = values.title || null;
          }
          if (values.soul !== editingBot.soul) {
            input.soul = values.soul;
          }
          if (values.workdir !== (editingBot.workdir ?? "")) {
            input.workdir = values.workdir || null;
          }
          if (values.approvalMode !== editingBot.approvalMode) {
            input.approvalMode = values.approvalMode;
          }
          const currentModel = editingBot.providerId && editingBot.modelId
            ? `${editingBot.providerId}::${editingBot.modelId}`
            : "";
          if (values.model !== currentModel) {
            const pair = parseModelPair(values.model);
            input.providerId = pair.providerId;
            input.modelId = pair.modelId;
          }
          await updateBot(editingBot.id, input);
          message.success(t("bots.saved"));
        } else {
          const pair = parseModelPair(values.model);
          const input: CreateBotInput = {
            name: values.name,
            icon: values.icon || undefined,
            title: values.title || null,
            workdir: values.workdir || null,
            approvalMode: values.approvalMode,
            providerId: pair.providerId,
            modelId: pair.modelId,
          };
          if (values.soul.trim()) {
            input.soul = values.soul;
          }
          await createBot(input);
          message.success(t("bots.createSuccess"));
        }
        setEditorOpen(false);
      } catch (e) {
        message.error(
          `${editorMode === "edit" ? t("bots.saveFailed") : t("bots.createFailed")} — ${backendErrorText(e)}`,
        );
      }
    },
    [createBot, editingBot, editorMode, t, updateBot],
  );

  const handleDelete = useCallback(async () => {
    if (!activeBot) {
      return;
    }
    try {
      await deleteBot(activeBot.id);
      message.success(t("bots.deleteSuccess"));
    } catch (e) {
      message.error(`${t("bots.deleteFailed")} — ${backendErrorText(e)}`);
    }
  }, [activeBot, deleteBot, t]);

  const handleCreateThread = useCallback(async () => {
    if (!activeBot) {
      return;
    }
    try {
      await useConversationStore
        .getState()
        .createConversation(t("chat.newConversation"), undefined, undefined, { botId: activeBot.id });
      navigate(`/chat?bot=${encodeURIComponent(activeBot.id)}`);
    } catch (e) {
      message.error(backendErrorText(e));
    }
  }, [activeBot, navigate, t]);

  const handleOpenThread = useCallback(
    (conversationId: string) => {
      void useConversationStore.getState().setActiveConversation(conversationId);
      if (activeBot) {
        navigate(`/chat?bot=${encodeURIComponent(activeBot.id)}`);
      }
    },
    [activeBot, navigate],
  );

  const showError = error !== null && error !== dismissedError;

  return (
    <div style={{ display: "flex", height: "100%", minHeight: 0 }}>
      <aside
        style={{
          width: 268,
          flexShrink: 0,
          borderRight: `1px solid ${token.colorBorderSecondary}`,
          padding: 12,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
        }}
      >
        <BotList
          bots={bots}
          activeBotId={activeBot?.id ?? null}
          unreadByBot={unreadByBot}
          departments={departments}
          onSelect={setActiveBot}
          onCreate={openCreate}
          pendingBadgeCount={pendingInputCount}
          pendingByBot={pendingByBot}
          onOpenQuota={departments.length > 0
            ? () => {
              setEditingDepartment(departments[0]);
              setDepartmentModalTab("quota");
              setDepartmentModalOpen(true);
            }
            : undefined}
          onManageDepartments={() => {
            setEditingDepartment(null);
            setDepartmentModalTab("basic");
            setDepartmentModalOpen(true);
          }}
          onEditDepartment={(department) => {
            setEditingDepartment(department);
            setDepartmentModalTab("basic");
            setDepartmentModalOpen(true);
          }}
        />
      </aside>

      <section style={{ flex: 1, minWidth: 0, padding: 16, display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <Button type="link" href="/bots/workflows" data-testid="bot-workflow-entry">
            {t("bot.workflow.title")}
          </Button>
        </div>
        {showError
          ? <Alert type="error" showIcon closable message={error} onClose={() => setDismissedError(error)} />
          : null}

        {loading && bots.length === 0
          ? (
            <div style={{ display: "flex", justifyContent: "center", padding: 48 }}>
              <Spin />
            </div>
          )
          : activeBot
          ? (
            <BotDetail
              bot={activeBot}
              deleting={deleting}
              routinesPanel={<BotRoutinesPanel botId={activeBot.id} />}
              assignmentsPanel={<BotAssignmentsPanel bot={activeBot} bots={bots} />}
              onEdit={() => {
                void openEdit();
              }}
              onDelete={() => {
                void handleDelete();
              }}
              onCreateThread={() => {
                void handleCreateThread();
              }}
              onOpenThread={handleOpenThread}
            />
          )
          : <BotEmptyState onCreate={openCreate} />}
      </section>

      <BotDepartmentModal
        open={departmentModalOpen}
        editing={editingDepartment}
        initialTab={departmentModalTab}
        onClose={() => setDepartmentModalOpen(false)}
        onSaved={reloadDepartments}
      />

      <BotEditor
        open={editorOpen}
        mode={editorMode}
        initialValues={editorInitialValues}
        modelOptions={modelOptions}
        saving={saving}
        onCancel={() => setEditorOpen(false)}
        onSubmit={(values) => {
          void handleSubmit(values);
        }}
      />
    </div>
  );
}
