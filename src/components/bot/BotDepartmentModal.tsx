// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 部门创建/编辑弹窗（P2-b T5''）。
 *
 * 契约：IPC-P2B §1 部门命令族；Q2 口径（单 Bot 单部门）由后端 UNIQUE 保证，
 * 前端只做展示与输入校验（空名拒绝、Chief 可空 —— ADR-P2B-6：Chief 空缺合法）。
 *
 * 与「AI 办公室」（fleet）的文案区分（ADR-P2B-1）：本弹窗所有键走 `bot.department.*`，
 * hint 明示「部门是常设组织归属、AI 办公室是临时协作场景」。
 */

import { backendErrorText } from "@/lib/errorI18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotDepartment } from "@/types";
import { MinusCircleOutlined } from "@ant-design/icons";
import { App, Form, Input, Modal, Select, Space, Tabs, Tag, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { BotQuotaPanel } from "./BotQuotaPanel";
import { BotUsagePanel } from "./BotUsagePanel";

const { Text } = Typography;

export interface BotDepartmentModalProps {
  open: boolean;
  /** 非空 = 编辑态 */
  editing: BotDepartment | null;
  /** 关闭时回传（含刷新信号） */
  onClose: () => void;
  /** 保存成功后回调（页面层刷新部门列表） */
  onSaved: () => void;
  /** P4-a：打开时默认激活的页签（配额入口从 /bots 直接跳入时使用）；P5-b 追加「用量」。 */
  initialTab?: "basic" | "quota" | "usage";
}

interface DepartmentFormValues {
  name: string;
  description?: string;
  chiefBotId?: string;
}

export function BotDepartmentModal({ open, editing, onClose, onSaved, initialTab = "basic" }: BotDepartmentModalProps) {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [form] = Form.useForm();
  const bots = useBotStore((s) => s.bots);
  const createBotDepartment = useBotStore((s) => s.createBotDepartment);
  const updateBotDepartment = useBotStore((s) => s.updateBotDepartment);
  const addBotToDepartment = useBotStore((s) => s.addBotToDepartment);
  const removeBotFromDepartment = useBotStore((s) => s.removeBotFromDepartment);

  const isEdit = editing !== null;

  // 成员增删：编辑态即时落命令层（add/remove 各有独立 IPC），保存/关闭均刷新页面列表。
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [memberBusy, setMemberBusy] = useState(false);
  const [activeTab, setActiveTab] = useState<"basic" | "quota" | "usage">(initialTab);

  useEffect(() => {
    if (open) {
      setMemberIds(editing?.botIds ?? []);
      setActiveTab(initialTab);
    }
  }, [open, editing, initialTab]);

  const chiefOptions = useMemo(
    () => bots.map((bot) => ({ value: bot.id, label: bot.name })),
    [bots],
  );

  const candidateOptions = useMemo(
    () => bots.filter((bot) => !memberIds.includes(bot.id)).map((bot) => ({ value: bot.id, label: bot.name })),
    [bots, memberIds],
  );

  const updateMember = async (botId: string, action: (id: string) => Promise<void>) => {
    if (!editing) {
      return;
    }
    setMemberBusy(true);
    try {
      await action(botId);
      // 以服务端返回为准（UNIQUE / 级联可能由后端修正结果），页面层 onSaved 同步侧栏分组
      const refreshed = await useBotStore.getState().listBotDepartments();
      const latest = refreshed.find((d) => d.id === editing.id);
      if (latest) {
        setMemberIds(latest.botIds);
      }
      onSaved();
    } catch (e) {
      antdMessage.error(backendErrorText(e));
    } finally {
      setMemberBusy(false);
    }
  };

  useEffect(() => {
    if (open) {
      form.setFieldsValue({
        name: editing?.name ?? "",
        description: editing?.description ?? "",
        chiefBotId: editing?.chiefBotId ?? undefined,
      });
    } else {
      form.resetFields();
    }
  }, [open, editing, form]);

  return (
    <Modal
      open={open}
      footer={activeTab === "quota" || activeTab === "usage" ? null : undefined}
      title={isEdit ? t("bot.department.edit") : t("bot.department.create")}
      okText={isEdit ? t("common.save") : t("bot.department.create")}
      cancelText={t("common.cancel")}
      destroyOnHidden
      onOk={() => {
        void form
          .validateFields()
          .then(async (values: DepartmentFormValues) => {
            try {
              const input = {
                name: values.name.trim(),
                description: values.description ?? "",
                ...(values.chiefBotId
                  ? { chiefBotId: values.chiefBotId }
                  : { chiefBotId: null }),
              };
              if (isEdit && editing) {
                await updateBotDepartment(editing.id, input);
                antdMessage.success(t("bots.saved"));
              } else {
                await createBotDepartment(input);
                antdMessage.success(t("bot.department.create"));
              }
              onSaved();
              onClose();
            } catch (e) {
              antdMessage.error(backendErrorText(e));
            }
          })
          .catch(() => {
            // 表单校验失败：antd 已在字段上标红，无需 toast
          });
      }}
      onCancel={onClose}
      data-testid="bot-department-modal"
    >
      <Tabs
        activeKey={activeTab}
        onChange={(key) => setActiveTab(key as "basic" | "quota" | "usage")}
        destroyInactiveTabPane={false}
        items={[
          {
            key: "basic",
            label: t("bot.department.title"),
            children: (
              <>
                <Form form={form} layout="vertical" requiredMark="optional">
                  <Form.Item
                    name="name"
                    label={t("bot.department.name")}
                    rules={[
                      { required: true, message: t("bot.department.nameRequired") },
                      { whitespace: true, message: t("bot.department.nameRequired") },
                    ]}
                  >
                    <Input maxLength={100} data-testid="bot-department-name" />
                  </Form.Item>
                  <Form.Item name="description" label={t("bot.department.description")}>
                    <Input.TextArea rows={2} maxLength={500} />
                  </Form.Item>
                  <Form.Item name="chiefBotId" label={t("bot.department.chief")}>
                    <Select
                      allowClear
                      placeholder={t("bot.department.chiefPlaceholder")}
                      options={chiefOptions}
                      notFoundContent={t("bot.department.chiefEmpty")}
                      data-testid="bot-department-chief"
                    />
                  </Form.Item>
                  <p style={{ marginBottom: 0, opacity: 0.65, fontSize: 12 }}>{t("bot.department.hint")}</p>
                </Form>

                {isEdit && editing
                  ? (
                    <Space direction="vertical" size={8} style={{ width: "100%", marginTop: 12 }}>
                      <Text strong style={{ fontSize: 13 }}>
                        {t("bot.department.addMember")}
                      </Text>
                      <Space wrap>
                        {memberIds.map((botId) => {
                          const bot = bots.find((b) => b.id === botId);
                          return (
                            <Tag
                              key={botId}
                              closable
                              disabled={memberBusy}
                              closeIcon={<MinusCircleOutlined />}
                              onClose={() => {
                                void updateMember(botId, (id) => removeBotFromDepartment(editing.id, id));
                              }}
                              data-testid={`bot-department-member-${botId}`}
                            >
                              {bot?.name ?? botId}
                            </Tag>
                          );
                        })}
                      </Space>
                      <Select
                        placeholder={t("bot.department.addMember")}
                        options={candidateOptions}
                        disabled={memberBusy}
                        value={null}
                        onChange={(value: string) => {
                          void updateMember(value, (id) => addBotToDepartment(editing.id, id));
                        }}
                        data-testid="bot-department-add-member"
                        style={{ width: "100%" }}
                      />
                    </Space>
                  )
                  : null}
              </>
            ),
          },
          ...(isEdit && editing
            ? [{
              key: "quota",
              label: t("bot.quota.title"),
              children: <BotQuotaPanel departmentId={editing.id} />,
            }]
            : []),
          ...(isEdit && editing
            ? [{
              key: "usage",
              label: t("bot.usage.title"),
              children: <BotUsagePanel scope="department" scopeId={editing.id} />,
            }]
            : []),
        ]}
      />
    </Modal>
  );
}
