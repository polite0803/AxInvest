// SPDX-License-Identifier: AGPL-3.0-only

const RELEASE_SOURCE_PREFIX = "axagent-release+";

function releaseManifestUrl(source: string): URL | null {
  const value = source.trim();
  if (value.startsWith(RELEASE_SOURCE_PREFIX)) {
    return null;
  }
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !url.pathname.endsWith("/release.json")) {
      return null;
    }
    return url;
  } catch {
    return null;
  }
}

/**
 * 把用户直接粘贴的发布清单 URL 归一化为宿主协议来源。
 *
 * 只识别 `https://` 且路径以 `/release.json` 结尾的 URL；其它地址原样返回，
 * 由后端 source 门禁统一拒绝。这样用户可以直接粘贴发布地址。
 */
export function normalizePluginSource(source: string): string {
  const value = source.trim();
  return releaseManifestUrl(value) ? `${RELEASE_SOURCE_PREFIX}${value}` : value;
}

/** 远程-only 模型：只有 HTTPS `release.json` 发布地址是合法来源。 */
export function isRemotePluginSource(source: string): boolean {
  return isReleasePluginSource(source);
}

/**
 * 判断是否为 HTTPS 发布清单地址。
 *
 * 发布地址不是普通远程 Git/npm 源：成品包 URL 与 SHA-256 都由发布清单提供，
 * 因此安装/更新时不要求用户手填哈希，但后端仍会校验清单声明的哈希。
 */
export function isReleasePluginSource(source: string): boolean {
  const value = source.trim();
  return value.startsWith(`${RELEASE_SOURCE_PREFIX}https://`)
    || releaseManifestUrl(value) !== null;
}
