// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 薄 re-export —— 卡片实现已于 2026-11-07（任务 #193）下沉到中性目录
 * `src/components/common/JobCard.tsx`。
 *
 * 保留本文件只为 `src/pages/**` 的存量消费方（如
 * `src/pages/TasksCenterPage/jobs/PipelineJobCard.tsx`）——该目录不在本批允许改动范围内。
 * **新增代码请直接 import `@/components/common/JobCard`**，不要再从这里转一道：
 * 组件中立性门禁判的是 import 字面量指向的目录，留在这里的引用会让
 * 「time-travel 依赖 tasks」这条违规复活。
 */
export { JobCard } from "@/components/common/JobCard";
