/**
 * 跨环境安全地打开外部 URL。
 *
 * Tauri WebView 中 `target="_blank"` 的原生跳转默认被 WebView 拦截
 * （点击无反应），必须走 tauri-plugin-opener；浏览器模式（dev mock）
 * 无该插件，回退 window.open。
 */
export async function openExternal(url: string): Promise<void> {
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
