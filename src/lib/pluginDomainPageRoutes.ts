// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件 OPC 域页的路由投影解析。
 *
 * 纯函数，不读 IO、不持有第二张注册表：
 * - `pages` 来自后端 `plugin_list_pages`（manifest.pages[] 的实时投影）；
 * - `packs` 来自后端 `plugin_list_domain_packs`（启用域的 logical manifest 投影）。
 *
 * 只有**同时**满足以下条件的页面才成为主路由：
 * 1. 插件已启用（后端 `plugin_list_pages` 本身也只投影启用的插件）；
 * 2. 页面加载形态是 bundle（schema 页仍走动态 UI 通道）；
 * 3. page id 是可用作 URL 段的稳定标识。
 *
 * 域包**不是**投影的前置条件，只用于计算 `legacyPath`（旧 `/opc/domain/*` 链）：
 * T3 起股票等非 OPC 插件也要有一等 URL 与侧栏入口；若仍要求 domainPacks 非空，
 * 股票页即使打包完成也只能从工作台「插件页」入口打开，启停对导航无可见变化。
 * 门禁 `scripts/check-route-reachability.mjs` 的 `parsePluginPageRoutes()` 同样按
 * 「全部 bundle 页」展开，两侧口径必须一致。
 *
 * 路径使用独立命名空间，避免与主壳静态路由争抢所有权：
 *   `/plugins/<pluginName>/<pageId>`
 *
 * 插件禁用时后端投影会过滤该插件，本函数输出自然去掉对应路由，调用方据此刷新。
 */
import type { PluginDomainPackDto, PluginDomainPacksInfoDto, PluginPageDto, PluginPagesInfoDto } from "@/types";

export interface PluginDomainPageRoute {
  readonly id: string;
  readonly pluginId: string;
  readonly pluginName: string;
  readonly page: PluginPageDto;
  readonly pageId: string;
  readonly path: string;
  readonly legacyPath?: string;
  readonly pluginSlug: string;
  readonly navKey: string;
  readonly navLabel: string;
}

const URL_SEGMENT = /^[a-z0-9][a-z0-9-]{0,63}$/i;
const PLUGIN_DOMAIN_PAGE_PATH = /^\/opc\/domain\/[a-z0-9][a-z0-9-]{0,63}$/i;

/** 取域包 logical manifest 的 `page.path`；形状不对时直接判为不可映射。 */
function domainPackPagePath(pack: PluginDomainPackDto): string | undefined {
  if (typeof pack.page !== "object" || pack.page === null) { return undefined; }
  const path = (pack.page as { path?: unknown }).path;
  return typeof path === "string" && PLUGIN_DOMAIN_PAGE_PATH.test(path) ? path : undefined;
}

/** 域包 id 是 manifest 里的逻辑 id；运行期页面 id 统一带 `opc-` 前缀并使用连字符。 */
function domainPackPageId(packId: string): string {
  return `opc-${packId.replace(/_/g, "-")}`;
}

/** 插件完整 id 的注册表键在 URL 中不适合直接暴露（可能含 `@`、scope 等）。 */
export function pluginRouteSlug(pluginId: string): string {
  return pluginId.toLowerCase().replace(/@/g, "-").replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
}

/**
 * 从两个后端投影解析插件域页路由；输入顺序不计入结果顺序。
 *
 * 这里不吞错：调用方负责把 `plugin_list_pages` / `plugin_list_domain_packs` 的失败
 * 显示成错误态。纯函数只拒绝「输入结构本身不可用」的项。
 */
export function resolvePluginDomainPageRoutes(
  pages: readonly PluginPagesInfoDto[],
  packs: readonly PluginDomainPacksInfoDto[],
): PluginDomainPageRoute[] {
  // 禁用插件的域包不贡献旧链映射：与 pages 的 enabled 过滤同口径，
  // 否则插件停用后 /opc/domain/* 旧链仍会把用户送进已禁用插件页。
  const packsById = new Map(packs.filter((item) => item.enabled).map((item) => [item.id, item] as const));
  const out: PluginDomainPageRoute[] = [];
  for (const info of pages) {
    if (!info.enabled) {
      continue;
    }
    const slug = pluginRouteSlug(info.id);
    if (!slug) {
      continue;
    }
    for (const page of info.pages) {
      if (page.loading !== "bundle" || !URL_SEGMENT.test(page.id)) {
        continue;
      }
      // 只有与当前 page id 同名的域包路径才挂旧链；一个页面被多个域包引用时，
      // 取第一个可映射的旧路径，避免两个旧路径各自猜一个目标。
      let legacyPath: string | undefined;
      for (const pack of packsById.get(info.id)?.domainPacks ?? []) {
        const path = domainPackPagePath(pack);
        if (!path) { continue; }
        if (page.id === domainPackPageId(pack.id)) {
          legacyPath = path;
          break;
        }
      }
      out.push({
        id: `plugin-page:${info.id}:${page.id}`,
        pluginId: info.id,
        pluginName: info.name,
        page,
        pageId: page.id,
        path: `/plugins/${slug}/${page.id}`,
        legacyPath,
        pluginSlug: slug,
        navKey: `plugin-page:${info.id}:${page.id}`,
        navLabel: page.title || page.id,
      });
    }
  }
  return out.sort((a, b) => a.path.localeCompare(b.path));
}
