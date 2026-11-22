// SPDX-License-Identifier: AGPL-3.0-only

const SHA256_RE = /^[0-9a-f]{64}$/i;
const WINDOWS_ABSOLUTE_RE = /^[a-z]:[\\/]/i;
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
 * 只识别 `https://` 且路径以 `/release.json` 结尾的 URL；普通 HTTPS Git 地址
 * 原样返回。这样“发布地址”可以直接粘贴，同时不会把所有 HTTPS 地址误升为发布源。
 */
export function normalizePluginSource(source: string): string {
  const value = source.trim();
  return releaseManifestUrl(value) ? `${RELEASE_SOURCE_PREFIX}${value}` : value;
}

/**
 * 判断插件来源是否需要安装期完整性锚点。
 *
 * 与 Rust `parse_install_source` 的用户可见分类保持一致：以下形态都从外部网络或
 * 包注册表取得内容，安装/更新时必须提供期望 SHA-256。
 */
export function isRemotePluginSource(source: string): boolean {
  const value = source.trim();
  if (!value) {
    return false;
  }

  if (value === "~") {
    return false;
  }

  if (isReleasePluginSource(value)) {
    return false;
  }

  // 本地目录优先判定，避免 `/tmp/pkg@1` 这类合法本地路径被 `@` 误判成 npm 包。
  if (
    value.startsWith("/")
    || value.startsWith("./")
    || value.startsWith("../")
    || value.startsWith(".\\")
    || value.startsWith("..\\")
    || value.startsWith("~/")
    || value.startsWith("~\\")
    || value.startsWith("\\\\")
    || WINDOWS_ABSOLUTE_RE.test(value)
  ) {
    return false;
  }

  if (
    value.startsWith("http://")
    || value.startsWith("https://")
    || value.startsWith("git@")
    || value.startsWith("openclaw:")
    || value.startsWith("@")
    || value.toLowerCase().endsWith(".git")
    || value.includes(".git#")
    || value.includes("@")
  ) {
    return true;
  }

  // 裸 npm 包名：不含路径分隔符、空白或 URI 协议。
  return !value.includes("/")
    && !value.includes("\\")
    && !value.includes(":")
    && !/\s/.test(value);
}

/** SHA-256 必须恰好是 64 个十六进制字符；前后空白允许由调用方先 trim。 */
export function isPluginSha256(value: string): boolean {
  return SHA256_RE.test(value.trim());
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
