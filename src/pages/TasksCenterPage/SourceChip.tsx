// SPDX-License-Identifier: AGPL-3.0-only

import { Tag } from "antd";
import { useTranslation } from "react-i18next";
import { getAllAdapters } from "./adapters/registry";
import type { SourceId } from "./model";

const ADAPTER_META = new Map(
  getAllAdapters().map((a) => [a.id, { nameKey: a.nameKey, color: a.color }]),
);

/** 任务源彩色 chip；未注册源回退显示原始 id。 */
export function SourceChip({ source }: { source: SourceId }) {
  const { t } = useTranslation();
  const meta = ADAPTER_META.get(source);
  return (
    <Tag color={meta?.color ?? "default"} style={{ marginInlineEnd: 0 }}>
      {meta ? t(meta.nameKey) : source}
    </Tag>
  );
}
