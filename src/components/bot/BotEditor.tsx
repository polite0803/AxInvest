// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 创建 / 编辑表单（Modal）。
 *
 * 字段与 `IPC.md` §2 的 `CreateBotInput` 对齐：名称（必填）、头像、角色（title）、
 * SOUL、模型（provider+model 成对）、工作目录、审批级别。
 *
 * 审批级别在 P1 **仅保存配置**（执行语义 P2 生效），故表单内给出 `bots.approvalModeP2Hint`。
 * 模型选项由调用方（`pages/BotsPage`）从 provider store 投影后传入 —— 组件不直读其它 feature store。
 */

import { APPROVAL_MODES, type ApprovalMode } from "@/types";
import { Input, Modal, Select, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/** 模型下拉项：`value` 为 `providerId::modelId`（成对提交，避免 BOT_MODEL_PAIR_INVALID）。 */
export interface BotEditorModelOption {
  value: string;
  label: string;
}

export interface BotEditorValues {
  name: string;
  icon: string;
  title: string;
  soul: string;
  workdir: string;
  approvalMode: ApprovalMode;
  /** `providerId::modelId`；空串 = 不指定（用默认模型） */
  model: string;
}

export interface BotEditorProps {
  open: boolean;
  mode: "create" | "edit";
  initialValues?: Partial<BotEditorValues>;
  modelOptions: BotEditorModelOption[];
  saving?: boolean;
  onCancel: () => void;
  onSubmit: (values: BotEditorValues) => void;
}

const EMPTY_VALUES: BotEditorValues = {
  name: "",
  icon: "",
  title: "",
  soul: "",
  workdir: "",
  approvalMode: "ask",
  model: "",
};

export function BotEditor({
  open,
  mode,
  initialValues,
  modelOptions,
  saving = false,
  onCancel,
  onSubmit,
}: BotEditorProps) {
  const { t } = useTranslation();
  const [values, setValues] = useState<BotEditorValues>({ ...EMPTY_VALUES, ...initialValues });
  const [nameError, setNameError] = useState(false);

  // 每次打开时重置（Modal 常驻挂载，避免上一次编辑残留到下一次）
  useEffect(() => {
    if (open) {
      setValues({ ...EMPTY_VALUES, ...initialValues });
      setNameError(false);
    }
  }, [open, initialValues]);

  const patch = (next: Partial<BotEditorValues>) => {
    setValues((prev) => ({ ...prev, ...next }));
  };

  const approvalOptions = useMemo(
    () => APPROVAL_MODES.map((mode) => ({ value: mode, label: t(`bots.approval.${mode}`) })),
    [t],
  );

  const handleOk = () => {
    const name = values.name.trim();
    if (!name) {
      setNameError(true);
      return;
    }
    onSubmit({
      ...values,
      name,
      icon: values.icon.trim(),
      title: values.title.trim(),
      workdir: values.workdir.trim(),
    });
  };

  return (
    <Modal
      open={open}
      title={mode === "create" ? t("bots.createTitle") : t("bots.editTitle")}
      onCancel={onCancel}
      onOk={handleOk}
      confirmLoading={saving}
      okText={t("bots.save")}
      cancelText={t("common.cancel")}
      width={560}
      destroyOnHidden
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.name")}</Text>
          <Input
            id="bot-editor-name"
            value={values.name}
            status={nameError ? "error" : undefined}
            placeholder={t("bots.namePlaceholder")}
            onChange={(e) => {
              patch({ name: e.target.value });
              if (nameError && e.target.value.trim()) {
                setNameError(false);
              }
            }}
            maxLength={64}
          />
          {nameError ? <Text type="danger">{t("bots.nameRequired")}</Text> : null}
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.avatar")}</Text>
          <Input
            id="bot-editor-icon"
            value={values.icon}
            placeholder="🤖"
            onChange={(e) => patch({ icon: e.target.value })}
            maxLength={8}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.role")}</Text>
          <Input
            id="bot-editor-title"
            value={values.title}
            placeholder={t("bots.rolePlaceholder")}
            onChange={(e) => patch({ title: e.target.value })}
            maxLength={128}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.soul")}</Text>
          <Input.TextArea
            id="bot-editor-soul"
            value={values.soul}
            placeholder={t("bots.soulPlaceholder")}
            onChange={(e) => patch({ soul: e.target.value })}
            autoSize={{ minRows: 6, maxRows: 12 }}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.model")}</Text>
          <Select
            id="bot-editor-model"
            allowClear
            showSearch
            optionFilterProp="label"
            value={values.model || undefined}
            placeholder={t("bots.modelPlaceholder")}
            options={modelOptions}
            onChange={(v: string | undefined) => patch({ model: v ?? "" })}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.workdir")}</Text>
          <Input
            id="bot-editor-workdir"
            value={values.workdir}
            placeholder={t("bots.workdirPlaceholder")}
            onChange={(e) => patch({ workdir: e.target.value })}
          />
        </label>

        <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <Text>{t("bots.approvalMode")}</Text>
          <Select
            id="bot-editor-approval-mode"
            value={values.approvalMode}
            options={approvalOptions}
            onChange={(v: ApprovalMode) => patch({ approvalMode: v })}
          />
          <Text type="secondary" style={{ fontSize: 12 }}>
            {t("bots.approvalModeP2Hint")}
          </Text>
        </label>
      </div>
    </Modal>
  );
}
