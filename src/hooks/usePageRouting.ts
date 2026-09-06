// SPDX-License-Identifier: AGPL-3.0-only

import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import type { PageKey } from "@/types";
import { useLocation, useNavigate } from "react-router-dom";

/** 单一路径来源：直接复用 pageRegistry 的权威映射，禁止本地散写。 */
const pageKeyToPath = BUILTIN_PAGE_PATH as Record<PageKey, string>;

/** path→key 反查表：用于识别多段路径（如 /opc/invoices、/devtools/trace-explorer）。 */
const pathToPageKeyMap: Record<string, PageKey> = Object.fromEntries(
  Object.entries(BUILTIN_PAGE_PATH).map(([key, path]) => [path, key as PageKey]),
) as Record<string, PageKey>;

const pathToPageKey = (path: string): PageKey => {
  if (path === "/" || path === "") {
    return "dashboard";
  }
  // 优先按完整路径反查（覆盖 /opc/invoices 这类多段路径）
  if (path in pathToPageKeyMap) {
    return pathToPageKeyMap[path];
  }
  // 再按 L1 段反查（/opc/:tab 的 tab 段不改变页面归属）
  const l1 = `/${path.slice(1).split("/")[0]}`;
  if (l1 in pathToPageKeyMap) {
    return pathToPageKeyMap[l1];
  }
  const key = path.slice(1);
  if (key in pageKeyToPath) {
    return key as PageKey;
  }
  return "chat";
};

export function useActivePage(): PageKey {
  const location = useLocation();
  return pathToPageKey(location.pathname);
}

export function usePageNavigation() {
  const navigate = useNavigate();

  const navigateTo = (page: PageKey) => {
    navigate(pageKeyToPath[page]);
  };

  const isActive = (page: PageKey): boolean => {
    return pageKeyToPath[page] === window.location.pathname;
  };

  return { navigateTo, isActive };
}

export { pageKeyToPath, pathToPageKey };
