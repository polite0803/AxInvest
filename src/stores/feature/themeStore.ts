// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import { create } from "zustand";
import { persist } from "zustand/middleware";

export interface ThemeMetadata {
  name: string;
  version: string;
  author?: string;
  description?: string;
}

export interface ThemeColors {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent?: string;
  selectionBackground?: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

export interface UiTheme {
  primary?: string;
  secondary?: string;
  accent?: string;
  error?: string;
  warning?: string;
  success?: string;
  textPrimary?: string;
  textSecondary?: string;
  border?: string;
  background?: string;
  surface?: string;
}

export interface Theme {
  metadata: ThemeMetadata;
  colors: ThemeColors;
  ui?: UiTheme;
}

interface ThemeState {
  currentTheme: string;
  themes: ThemeMetadata[];
  customThemes: Theme[];
  isLoading: boolean;
  error: string | null;
  setCurrentTheme: (themeName: string) => void;
  loadThemes: () => Promise<void>;
  loadTheme: (themeName: string) => Promise<Theme | null>;
  saveCustomTheme: (theme: Theme) => Promise<void>;
  deleteCustomTheme: (themeName: string) => Promise<void>;
  getThemeColors: (themeName: string) => ThemeColors | null;
}

/** Catppuccin Mocha —— 终端默认配色，也是 xterm 初始主题的唯一来源。 */
export const TERMINAL_THEME_DEFAULT: ThemeColors = {
  background: "#1e1e2e",
  foreground: "#cdd6f4",
  cursor: "#f5e0dc",
  cursorAccent: "#1e1e2e",
  selectionBackground: "#585b7066",
  black: "#45475a",
  red: "#f38ba8",
  green: "#a6e3a1",
  yellow: "#f9e2af",
  blue: "#89b4fa",
  magenta: "#f5c2e7",
  cyan: "#94e2d5",
  white: "#bac2de",
  brightBlack: "#585b70",
  brightRed: "#f38ba8",
  brightGreen: "#a6e3a1",
  brightYellow: "#f9e2af",
  brightBlue: "#89b4fa",
  brightMagenta: "#f5c2e7",
  brightCyan: "#94e2d5",
  brightWhite: "#a6adc8",
};

export const BUILT_IN_THEMES: Record<string, ThemeColors> = {
  default: TERMINAL_THEME_DEFAULT,
  // 同一套配色的旧名字；此前 ThemeManager 里另维护了一份，现已合并到这里
  "catppuccin-mocha": TERMINAL_THEME_DEFAULT,
  monokai: {
    background: "#272822",
    foreground: "#f8f8f2",
    cursor: "#f8f8f0",
    cursorAccent: "#272822",
    selectionBackground: "#49483E",
    black: "#272822",
    red: "#f92672",
    green: "#a6e22e",
    yellow: "#f4bf75",
    blue: "#66d9ef",
    magenta: "#ae81ff",
    cyan: "#a1efe4",
    white: "#f8f8f2",
    brightBlack: "#75715E",
    brightRed: "#f92672",
    brightGreen: "#a6e22e",
    brightYellow: "#f4bf75",
    brightBlue: "#66d9ef",
    brightMagenta: "#ae81ff",
    brightCyan: "#a1efe4",
    brightWhite: "#f9f8f5",
  },
  gruvbox: {
    background: "#282828",
    foreground: "#ebdbb2",
    cursor: "#ebdbb2",
    cursorAccent: "#282828",
    selectionBackground: "#3c3836",
    black: "#282828",
    red: "#cc241d",
    green: "#98971a",
    yellow: "#d79921",
    blue: "#458588",
    magenta: "#b16286",
    cyan: "#689d6a",
    white: "#a89984",
    brightBlack: "#928374",
    brightRed: "#fb4934",
    brightGreen: "#b8bb26",
    brightYellow: "#fabd2f",
    brightBlue: "#83a598",
    brightMagenta: "#d3869b",
    brightCyan: "#8ec07c",
    brightWhite: "#ebdbb2",
  },
};

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      currentTheme: "default",
      themes: [],
      customThemes: [],
      isLoading: false,
      error: null,

      setCurrentTheme: (themeName: string) => {
        set({ currentTheme: themeName });
        applyThemeToDocument(themeName);
        // Map theme name to its corresponding data-theme-preset attribute
        const presetMap: Record<string, string> = {
          default: "deep-dusk",
          "catppuccin-mocha": "deep-dusk",
          monokai: "deep-dusk",
          gruvbox: "deep-dusk",
        };
        const preset = presetMap[themeName] || themeName;
        document.documentElement.setAttribute("data-theme-preset", preset);
      },

      loadThemes: async () => {
        set({ isLoading: true, error: null });
        try {
          const themeList = await invoke<ThemeMetadata[]>("list_themes");
          const builtInThemes: ThemeMetadata[] = [
            {
              name: "default",
              version: "1.0.0",
              author: "AxAgent Team",
              description: "Default Catppuccin Mocha theme",
            },
            {
              name: "monokai",
              version: "1.0.0",
              author: "Wimer Hazenberg",
              description: "Monokai color scheme",
            },
            {
              name: "gruvbox",
              version: "1.0.0",
              author: "github.com/morhetz/gruvbox",
              description: "Gruvbox dark theme",
            },
          ];
          set({ themes: [...builtInThemes, ...themeList], isLoading: false });
        } catch (e) {
          set({ error: String(e), isLoading: false });
        }
      },

      loadTheme: async (themeName: string) => {
        try {
          const theme = await invoke<Theme>("get_theme", { name: themeName });
          return theme;
        } catch {
          return null;
        }
      },

      saveCustomTheme: async (theme: Theme) => {
        try {
          await invoke("save_theme", { theme });
          set((state) => ({
            customThemes: [...state.customThemes, theme],
          }));
        } catch (e) {
          set({ error: String(e) });
        }
      },

      deleteCustomTheme: async (themeName: string) => {
        try {
          await invoke("delete_theme", { name: themeName });
          set((state) => ({
            customThemes: state.customThemes.filter(
              (t) => t.metadata.name !== themeName,
            ),
          }));
        } catch (e) {
          set({ error: String(e) });
        }
      },

      getThemeColors: (themeName: string) => {
        if (BUILT_IN_THEMES[themeName]) {
          return BUILT_IN_THEMES[themeName];
        }
        const customTheme = get().customThemes.find(
          (t) => t.metadata.name === themeName,
        );
        return customTheme?.colors || null;
      },
    }),
    {
      name: "axagent-theme-storage",
      partialize: (state) => ({
        currentTheme: state.currentTheme,
      }),
    },
  ),
);

export function applyThemeToDocument(themeName: string) {
  const store = useThemeStore.getState();
  const colors = store.getThemeColors(themeName);

  if (colors) {
    // 作用域只到根节点上的 --theme-* 变量：它们服务终端面板（xterm 色板）。
    // 不要再改 body 的背景/前景——那会把整个应用压成终端配色，盖掉 oklch 主题 token。
    document.documentElement.style.setProperty(
      "--theme-background",
      colors.background,
    );
    document.documentElement.style.setProperty(
      "--theme-foreground",
      colors.foreground,
    );
    document.documentElement.style.setProperty("--theme-cursor", colors.cursor);
    document.documentElement.style.setProperty("--theme-black", colors.black);
    document.documentElement.style.setProperty("--theme-red", colors.red);
    document.documentElement.style.setProperty("--theme-green", colors.green);
    document.documentElement.style.setProperty("--theme-yellow", colors.yellow);
    document.documentElement.style.setProperty("--theme-blue", colors.blue);
    document.documentElement.style.setProperty(
      "--theme-magenta",
      colors.magenta,
    );
    document.documentElement.style.setProperty("--theme-cyan", colors.cyan);
    document.documentElement.style.setProperty("--theme-white", colors.white);
    document.documentElement.style.setProperty(
      "--theme-bright-black",
      colors.brightBlack,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-red",
      colors.brightRed,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-green",
      colors.brightGreen,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-yellow",
      colors.brightYellow,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-blue",
      colors.brightBlue,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-magenta",
      colors.brightMagenta,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-cyan",
      colors.brightCyan,
    );
    document.documentElement.style.setProperty(
      "--theme-bright-white",
      colors.brightWhite,
    );
  }
}

export function getXtermTheme(themeName: string) {
  const store = useThemeStore.getState();
  const colors = store.getThemeColors(themeName);

  if (!colors) {
    return BUILT_IN_THEMES["default"];
  }

  return {
    background: colors.background,
    foreground: colors.foreground,
    cursor: colors.cursor,
    cursorAccent: colors.cursorAccent || colors.background,
    selectionBackground: colors.selectionBackground || "#585b7066",
    black: colors.black,
    red: colors.red,
    green: colors.green,
    yellow: colors.yellow,
    blue: colors.blue,
    magenta: colors.magenta,
    cyan: colors.cyan,
    white: colors.white,
    brightBlack: colors.brightBlack,
    brightRed: colors.brightRed,
    brightGreen: colors.brightGreen,
    brightYellow: colors.brightYellow,
    brightBlue: colors.brightBlue,
    brightMagenta: colors.brightMagenta,
    brightCyan: colors.brightCyan,
    brightWhite: colors.brightWhite,
  };
}

/**
 * 启动时把当前终端配色落到根变量上，必须在首帧渲染前调用。
 *
 * 这里不加载主题列表：`themes` 的唯一消费方是 ThemeManager，它在挂载时自己调
 * `loadThemes()`；在启动同步路径上发 IPC 只会制造一个随时可能失败的早期请求。
 */
export function initializeTheme() {
  applyThemeToDocument(useThemeStore.getState().currentTheme);
}
