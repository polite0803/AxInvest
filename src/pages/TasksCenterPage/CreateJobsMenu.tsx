// SPDX-License-Identifier: AGPL-3.0-only

import { ReplaySweep } from "@/components/time-travel/ReplaySweep";
import { TimeTravelBatchCard } from "@/components/time-travel/TimeTravelBatchCard";
import { Button, Dropdown, type MenuProps, Modal, Space } from "antd";
import { CalendarClock, Play, Plus, Rocket, TimerReset } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { PipelineJobCard } from "./jobs/PipelineJobCard";

type CreateKind = "pipeline" | "batch" | "replay";

/**
 * 统一「新建任务」入口（Run Trigger 范式）：
 * 把旧手动作业页三张发起卡（股票分析管道 / 时间旅行批次 / 回放扫盘）
 * 收口到一个下拉 + Modal；定时任务定义由「计划任务」Tab 管理。
 */
export function CreateJobsMenu({
  onSelectSchedule,
}: {
  onSelectSchedule?: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState<CreateKind | null>(null);

  const items: MenuProps["items"] = [
    {
      key: "scheduled",
      icon: <CalendarClock size={14} />,
      label: t("tasksCenter.create.menuScheduled"),
      onClick: () => onSelectSchedule?.(),
    },
    {
      key: "pipeline",
      icon: <Rocket size={14} />,
      label: t("tasksCenter.create.menuPipeline"),
      onClick: () => setOpen("pipeline"),
    },
    {
      key: "batch",
      icon: <Play size={14} />,
      label: t("tasksCenter.create.menuBatch"),
      onClick: () => setOpen("batch"),
    },
    {
      key: "replay",
      icon: <TimerReset size={14} />,
      label: t("tasksCenter.create.menuReplay"),
      onClick: () => setOpen("replay"),
    },
  ];

  return (
    <>
      <Dropdown menu={{ items }} trigger={["click"]}>
        <Button type="primary" icon={<Plus size={15} />}>
          {t("tasksCenter.create.button")}
        </Button>
      </Dropdown>

      <Modal
        title={t("tasksCenter.create.menuPipeline")}
        open={open === "pipeline"}
        onCancel={() => setOpen(null)}
        footer={null}
        width={780}
        destroyOnHidden
        styles={{ body: { maxHeight: "72vh", overflowY: "auto" } }}
      >
        <PipelineJobCard />
      </Modal>

      <Modal
        title={t("tasksCenter.create.menuBatch")}
        open={open === "batch"}
        onCancel={() => setOpen(null)}
        footer={null}
        width={780}
        destroyOnHidden
        styles={{ body: { maxHeight: "72vh", overflowY: "auto" } }}
      >
        <TimeTravelBatchCard />
      </Modal>

      <Modal
        title={t("tasksCenter.create.menuReplay")}
        open={open === "replay"}
        onCancel={() => setOpen(null)}
        footer={null}
        width={780}
        destroyOnHidden
        styles={{ body: { maxHeight: "72vh", overflowY: "auto" } }}
      >
        <Space direction="vertical" style={{ width: "100%" }}>
          <ReplaySweep />
        </Space>
      </Modal>
    </>
  );
}
