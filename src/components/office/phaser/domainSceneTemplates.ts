// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 域包办公室场景模板注入（PLAN-office-auto-provision.md 阶段 3）。
 *
 * 后端 `list_office_scene_templates` 扫描 `config/opc/domain_packs/<id>/office_scene.yaml`，
 * 经 `registerSceneTemplate` 注入上游数组——新增域包零代码获得办公室场景。
 * 与 TS 内置模板同 slug 时 registerSceneTemplate 天然去重（内置优先，阶段 4 迁移前不变）。
 * 浏览器 mock 模式（非 Tauri）无域包文件系统 ⇒ 静默跳过。
 */
import { invoke, isTauri } from "@/lib/invoke";
import type { OfficeSceneTemplate } from "./sceneTemplates";
import { registerSceneTemplate } from "./sceneTemplates";

export async function registerDomainSceneTemplates(): Promise<number> {
  if (!isTauri()) {
    return 0;
  }
  const templates = await invoke<OfficeSceneTemplate[]>("list_office_scene_templates");
  for (const tpl of templates) {
    registerSceneTemplate(tpl);
  }
  return templates.length;
}
