// SPDX-License-Identifier: AGPL-3.0-only

import { IconEditor } from "@/components/shared/IconEditor";
import { ModelSelect, parseModelValue } from "@/components/shared/ModelSelect";
import { safeJoinIds } from "@/lib/validators";
import { Avatar, Divider, Input, Modal, theme, Typography } from "antd";
import { FolderOpen } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export interface CategoryEditFormData {
  name: string;
  iconType: string | null;
  iconValue: string | null;
  systemPrompt: string | null;
  defaultProviderId: string | null;
  defaultModelId: string | null;
}

interface CategoryEditModalProps {
  open: boolean;
  onClose: () => void;
  onOk: (data: CategoryEditFormData) => void;
  initialName?: string;
  initialIconType?: string | null;
  initialIconValue?: string | null;
  initialSystemPrompt?: string | null;
  initialDefaultProviderId?: string | null;
  initialDefaultModelId?: string | null;
  title?: string;
  confirmLoading?: boolean;
}

export function CategoryEditModal({
  open,
  onClose,
  onOk,
  initialName = "",
  initialIconType = null,
  initialIconValue = null,
  initialSystemPrompt = null,
  initialDefaultProviderId = null,
  initialDefaultModelId = null,
  title,
  confirmLoading = false,
}: CategoryEditModalProps) {
  const { t } = useTranslation();
  const { token } = theme.useToken();

  const [name, setName] = useState("");
  const [iconType, setIconType] = useState<string | null>(null);
  const [iconValue, setIconValue] = useState<string | null>(null);
  const [systemPrompt, setSystemPrompt] = useState("");
  const [defaultProviderId, setDefaultProviderId] = useState<string | null>(
    null,
  );
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setName(initialName);
    setIconType(initialIconType);
    setIconValue(initialIconValue);
    setSystemPrompt(initialSystemPrompt ?? "");
    setDefaultProviderId(initialDefaultProviderId);
    setDefaultModelId(initialDefaultModelId);
  }, [
    open,
    initialName,
    initialIconType,
    initialIconValue,
    initialSystemPrompt,
    initialDefaultProviderId,
    initialDefaultModelId,
  ]);

  const selectedModelValue = defaultProviderId && defaultModelId
    ? safeJoinIds([defaultProviderId, defaultModelId], "::")
    : undefined;

  const handleDefaultModelChange = useCallback((value: string | undefined) => {
    const parsed = parseModelValue(value);
    setDefaultProviderId(parsed?.providerId ?? null);
    setDefaultModelId(parsed?.modelId ?? null);
  }, []);

  const handleOk = useCallback(() => {
    onOk({
      name: name.trim(),
      iconType: iconType,
      iconValue: iconValue,
      systemPrompt: systemPrompt.trim() || null,
      defaultProviderId: defaultProviderId,
      defaultModelId: defaultModelId,
    });
  }, [
    name,
    iconType,
    iconValue,
    systemPrompt,
    defaultProviderId,
    defaultModelId,
    onOk,
  ]);

  const canSubmit = name.trim().length > 0;

  return (
    <Modal
      title={title ?? t("chat.createCategory")}
      open={open}
      onCancel={onClose}
      onOk={handleOk}
      okButtonProps={{ disabled: !canSubmit || confirmLoading }}
      confirmLoading={confirmLoading}
      width={560}
      mask={{ enabled: true, blur: true }}
    >
      <div className="flex flex-col items-center gap-3 py-3">
        <IconEditor
          iconType={iconType}
          iconValue={iconValue}
          onChange={(type, value) => {
            setIconType(type);
            setIconValue(value);
          }}
          size={40}
          defaultIcon={
            <Avatar
              size={40}
              icon={<FolderOpen size={18} />}
              style={{
                cursor: "pointer",
                backgroundColor: token.colorFillSecondary,
                color: token.colorTextSecondary,
              }}
            />
          }
        />

        <Input
          placeholder={t("chat.categoryNamePlaceholder")}
          value={name}
          onChange={(e) => setName(e.target.value)}
          onPressEnter={handleOk}
          style={{ maxWidth: 340 }}
        />

        <Input.TextArea
          placeholder={t("chat.categorySystemPromptPlaceholder")}
          value={systemPrompt}
          onChange={(e) => setSystemPrompt(e.target.value)}
          autoSize={{ minRows: 5, maxRows: 10 }}
          style={{ maxWidth: 340 }}
        />

        <Divider style={{ margin: "4px 0 0" }} />

        <div style={{ width: "100%", maxWidth: 420 }}>
          <Typography.Text strong style={{ display: "block", marginBottom: 8 }}>
            {t("settings.defaultConversationModel")}
          </Typography.Text>
          <ModelSelect
            value={selectedModelValue}
            onChange={handleDefaultModelChange}
            placeholder={t("settings.useActiveModel")}
            modelTypes={["Chat"]}
            style={{ width: "100%" }}
          />
        </div>
      </div>
    </Modal>
  );
}
