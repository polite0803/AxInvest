// SPDX-License-Identifier: AGPL-3.0-only

/**
 * CreateFleetModal — 创建办公室弹窗。
 *
 * 在原 `prompt()` 输入名字的基础上扩展：
 * - 场景模板选择（默认办公室 / 投研办公室）
 * - 投资策略选择（与后端 llm_dispatcher.rs 的 strategy 分支对齐）
 *   - investment_research：投研策略
 *   - trading：交易策略
 *   - risk_management：风控策略
 *   - data_analysis：数据分析策略
 *   - （留空）：通用策略
 *
 * 选定场景模板 + 策略后写入 Fleet.metadata.strategy 与 sceneTemplateSlug，
 * 后端 LlmDispatcher 会读取 strategy 注入对应的业务上下文 prompt。
 */
import {
  assignSeedRooms,
  resolveSceneTemplate,
  SCENE_DOMAIN_SLUGS,
  sceneTemplateLabelKey,
  useSceneTemplates,
} from "@/components/office/phaser/sceneTemplates";
import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { message } from "@/lib/toast";
import { useOfficeStore } from "@/stores";
import type { CreateFleetInput, DomainPackProfile, FleetMetadata } from "@/types";
import { Button, Form, Input, Modal, Select, Space, Switch } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

export interface CreateFleetModalProps {
  open: boolean;
  onClose: () => void;
}

interface FormValues {
  name: string;
  sceneTemplateSlug?: string;
  strategy?: string;
  description?: string;
  /** 建房即成队：行业场景模板下自动按域包名册配齐成员（PLAN-office-auto-provision.md 阶段 2） */
  autoSeed?: boolean;
}

/** 策略选项 — 与后端 llm_dispatcher.rs build_system_prompt 的 match 分支对齐 */
const STRATEGY_OPTIONS: Array<{
  value: string;
  key: string;
}> = [
  { value: "investment_research", key: "investment_research" },
  { value: "trading", key: "trading" },
  { value: "risk_management", key: "risk_management" },
  { value: "data_analysis", key: "data_analysis" },
];

export function CreateFleetModal({ open, onClose }: CreateFleetModalProps) {
  const { t } = useTranslation();
  const createFleet = useOfficeStore((s) => s.createFleet);
  const templates = useSceneTemplates();
  const [loading, setLoading] = useState(false);
  const [form] = Form.useForm<FormValues>();
  const sceneSlug = Form.useWatch("sceneTemplateSlug", form);
  const isDomainScene = !!sceneSlug && SCENE_DOMAIN_SLUGS.has(sceneSlug);

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      setLoading(true);
      const metadata: FleetMetadata = {
        description: values.description?.trim() ?? "",
        maxMembers: 0,
        strategy: values.strategy?.trim() || undefined,
        tags: [],
      };
      const input: CreateFleetInput = {
        name: values.name.trim(),
        sceneTemplateSlug: values.sceneTemplateSlug,
        metadata,
      };
      const fleet = await createFleet(input);
      if (fleet) {
        message.success(t("office.createFleet.success", { name: fleet.name }));
        if (values.autoSeed && values.sceneTemplateSlug && SCENE_DOMAIN_SLUGS.has(values.sceneTemplateSlug)) {
          const res = await seedFleetFromDomain(fleet.id, values.sceneTemplateSlug);
          if (res === null) {
            // null = 名册查询失败（已在全局 error 态），不打断建房成功语义
          } else if (res.skipped > 0) {
            message.warning(t("office.createFleet.seedResultSkipped", { seeded: res.seeded, skipped: res.skipped }));
          } else if (res.seeded > 0) {
            message.success(t("office.createFleet.seedResult", { seeded: res.seeded }));
          }
        }
        form.resetFields();
        onClose();
      }
    } catch (e) {
      if (e instanceof Error && e.message) {
        showBackendError(message, e);
      }
    } finally {
      setLoading(false);
    }
  };

  const handleCancel = () => {
    form.resetFields();
    onClose();
  };

  return (
    <Modal
      title={t("office.createFleet.modalTitle")}
      open={open}
      onCancel={handleCancel}
      footer={
        <Space>
          <Button onClick={handleCancel}>{t("office.createFleet.cancel")}</Button>
          <Button type="primary" loading={loading} onClick={handleSubmit}>
            {t("office.createFleet.confirm")}
          </Button>
        </Space>
      }
      width={520}
      destroyOnHidden
    >
      <Form
        form={form}
        layout="vertical"
        initialValues={{ sceneTemplateSlug: "default_office", autoSeed: true }}
      >
        <Form.Item
          name="name"
          label={t("office.createFleet.nameLabel")}
          rules={[{ required: true, message: t("office.createFleet.nameRequired") }]}
        >
          <Input placeholder={t("office.createFleet.namePlaceholder")} />
        </Form.Item>

        <Form.Item
          name="sceneTemplateSlug"
          label={t("office.createFleet.sceneLabel")}
        >
          <Select
            options={templates.map((tpl) => ({
              value: tpl.slug,
              label: t(sceneTemplateLabelKey(tpl)),
            }))}
          />
        </Form.Item>

        {isDomainScene
          ? (
            <Form.Item
              name="autoSeed"
              label={t("office.createFleet.autoSeedLabel")}
              valuePropName="checked"
            >
              <Switch />
            </Form.Item>
          )
          : null}

        <Form.Item
          name="strategy"
          label={t("office.createFleet.strategyLabel")}
          tooltip={t("office.createFleet.strategyTooltip")}
        >
          <Select
            allowClear
            placeholder={t("office.createFleet.strategyPlaceholder")}
            options={STRATEGY_OPTIONS.map((opt) => ({
              value: opt.value,
              label: t(`office.strategy.${opt.key}`),
            }))}
          />
        </Form.Item>

        <Form.Item name="description" label={t("office.createFleet.descriptionLabel")}>
          <Input.TextArea
            rows={2}
            placeholder={t("office.createFleet.descriptionPlaceholder")}
          />
        </Form.Item>
      </Form>
    </Modal>
  );
}

/** 建房即成队：查域包名册并逐条入房（PLAN-office-auto-provision.md 阶段 2）。
 *  单条 addMember 失败（slug 冲突等）计入 skipped 不中断；名册查询失败返回 null。 */
async function seedFleetFromDomain(
  fleetId: string,
  domainPackId: string,
): Promise<{ seeded: number; skipped: number } | null> {
  let profiles: DomainPackProfile[];
  try {
    profiles = await invoke<DomainPackProfile[]>("list_domain_pack_profiles", { domainPackId });
  } catch {
    return null;
  }
  const usable = profiles.filter((p) => p.existsInDb);
  const rooms = assignSeedRooms(usable.length, resolveSceneTemplate(domainPackId));
  const addMember = useOfficeStore.getState().addMember;
  let seeded = 0;
  for (const [i, p] of usable.entries()) {
    const member = await addMember({
      fleetId,
      agentId: p.profileId,
      agentSlug: p.profileId,
      displayName: p.name,
      agentProfileId: p.profileId,
      // manifest 显式指定的房间优先（阶段 3），否则轮转分配
      roomId: p.room ?? rooms[i],
    });
    if (member) {
      seeded += 1;
    }
  }
  return { seeded, skipped: profiles.length - seeded };
}
