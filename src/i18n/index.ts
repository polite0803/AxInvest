// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhCN from "./locales/zh-CN.json";

// 只同步 bundle 默认语言 zh-CN，其余 10 种语言（含回退语言 en-US）一律动态 import。
// dev 模式下 Vite 把 JSON 展开成 JS 字面量：单份 locale 实测 3.2-3.3 MB（源文件 712-733 KB，
// 膨胀约 4.6 倍）。静态挂在 DOMContentLoaded 之前会整份计入「首屏弹出」的等待时间。
const LAZY_LOCALES: Record<string, () => Promise<{ default: Record<string, unknown> }>> = {
  "en-US": () => import("./locales/en-US.json"),
  "zh-TW": () => import("./locales/zh-TW.json"),
  ja: () => import("./locales/ja.json"),
  ko: () => import("./locales/ko.json"),
  fr: () => import("./locales/fr.json"),
  de: () => import("./locales/de.json"),
  es: () => import("./locales/es.json"),
  ru: () => import("./locales/ru.json"),
  hi: () => import("./locales/hi.json"),
  ar: () => import("./locales/ar.json"),
};

const inflight = new Map<string, Promise<void>>();

/** 幂等加载某语言的资源包；已 bundle / 正在加载 / 加载失败后重试都只走一条路径。 */
function loadLocale(lng: string): Promise<void> {
  if (i18n.hasResourceBundle(lng, "translation")) {
    return Promise.resolve();
  }
  const loader = LAZY_LOCALES[lng];
  if (!loader) {
    return Promise.resolve();
  }
  const cached = inflight.get(lng);
  if (cached) {
    return cached;
  }
  const promise = loader()
    .then((mod) => {
      i18n.addResourceBundle(lng, "translation", mod.default, true, true);
    })
    .catch((e) => {
      // 失败不留缓存，让下一次 changeLanguage 能重试
      inflight.delete(lng);
      console.warn(`[i18n] Failed to load locale "${lng}":`, e);
    });
  inflight.set(lng, promise);
  return promise;
}

i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
  },
  lng: "zh-CN",
  fallbackLng: "en-US",
  interpolation: { escapeValue: false },
  // 允许 resources 中只包含部分语言，其余语言运行时动态加载
  partialBundledLanguages: true,
});

// 不再无条件预加载 en-US：它是 fallbackLng，而 fallback 对 zh-CN 用户是**结构性死路径**
// —— zh-CN 与 en-US 键集已逐键对证完全一致（各 15370 键，zh 缺 0），且
// `scripts/check-i18n-key-parity.mjs` 门禁强制 11 种语言键集对齐，缺键进不来。
// 真正的 en-US 用户走下面的 changeLanguage 拦截器，由它 await 加载后再切换，
// 因此不会出现「切过去显示裸 key」的窗口。省掉首帧后 3.2 MB 的解析。

// 拦截 changeLanguage：切换前先加载对应 locale 资源，避免切换瞬间显示未翻译 key。
// 所有调用方（AppInitializer / App.tsx / TitleBar / GeneralSettings）无需改动。
const originalChangeLanguage = i18n.changeLanguage.bind(i18n);
i18n.changeLanguage = (async (lng?: string) => {
  if (lng) {
    await loadLocale(lng);
  }
  return originalChangeLanguage(lng);
}) as typeof i18n.changeLanguage;

export default i18n;
