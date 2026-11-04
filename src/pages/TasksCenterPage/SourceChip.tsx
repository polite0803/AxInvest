// SPDX-License-Identifier: AGPL-3.0-only

import { Tag } from "antd";
import { useTranslation } from "react-i18next";
import { getAdapter, useTaskSourceRegistry } from "./adapters/registry";
import type { SourceId } from "./model";

/**
 * 任务源彩色 chip；未注册源回退显示原始 id。
 *
 * ⚠ 逐次查注册表（`getAdapter`），**不在模块加载期缓存成 Map**：插件贡献的源是
 * 运行期注册的（任务 #148），模块级快照会永远看不见它们，chip 会退化成裸 id。
 * 名字口径同 `TaskSourceAdapter.name`：宿主源走 i18n key，插件源带纯文本 `name`。
 */
export function SourceChip({ source }: { source: SourceId }) {
  const { t } = useTranslation();
  useTaskSourceRegistry();
  const adapter = getAdapter(source);
  return (
    <Tag color={adapter?.color ?? "default"} style={{ marginInlineEnd: 0 }}>
      {adapter ? adapter.name ?? t(adapter.nameKey) : source}
    </Tag>
  );
}
