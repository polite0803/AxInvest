import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import path from "path";
import type { PluginOption } from "vite";
import monacoEditorPluginModule from "vite-plugin-monaco-editor";
import { defineConfig, type Plugin } from "vitest/config";

interface MonacoEditorPluginModule {
  default?: Plugin;
  [key: string]: unknown;
}

const monacoEditorPlugin = (monacoEditorPluginModule as MonacoEditorPluginModule).default
  || monacoEditorPluginModule as Plugin;

const host = process.env.TAURI_DEV_HOST;

// Only bundle commonly-used Shiki language grammars (saves ~8 MB in build).
// Languages not listed here will gracefully degrade (no syntax highlighting).
const SHIKI_ALLOWED_LANGS = new Set([
  "angular-html",
  "angular-ts",
  "astro",
  "bash",
  "c",
  "cpp",
  "csharp",
  "css",
  "dart",
  "dockerfile",
  "go",
  "graphql",
  "html",
  "html-derivative",
  "java",
  "javascript",
  "json",
  "json5",
  "jsonc",
  "jsx",
  "kotlin",
  "less",
  "lua",
  "markdown",
  "mdc",
  "mdx",
  "objective-c",
  "objective-cpp",
  "php",
  "python",
  "ruby",
  "rust",
  "sass",
  "scss",
  "shell",
  "shellscript",
  "sql",
  "svelte",
  "swift",
  "toml",
  "tsx",
  "typescript",
  "vue",
  "vue-html",
  "xml",
  "yaml",
]);

function shikiLanguageFilter(): Plugin {
  return {
    name: "shiki-language-filter",
    enforce: "pre",
    resolveId(id) {
      const m = id.match(/^@shikijs\/langs\/(.+)$/);
      if (m && !SHIKI_ALLOWED_LANGS.has(m[1])) {
        return "\0shiki-lang-noop";
      }
      return null;
    },
    load(id) {
      if (id === "\0shiki-lang-noop") {
        return "export default []";
      }
      return null;
    },
  };
}

// Remove crossorigin attributes from script/link tags in the built HTML.
// Tauri's custom protocol (tauri://localhost) does not handle CORS preflight
// requests, so crossorigin attributes cause all scripts and stylesheets to
// fail loading in the packaged app, resulting in a blank window.
function removeCrossorigin(): Plugin {
  return {
    name: "remove-crossorigin",
    transformIndexHtml(html) {
      return html.replace(/ crossorigin/g, "");
    },
  };
}

function optionalPeerDepPlugin(): PluginOption {
  return {
    name: "optional-peer-dep",
    resolveId(id) {
      if (id.startsWith("__vite-optional-peer-dep:")) {
        // Map zustand's optional peer dep (immer) to the actual package
        const match = id.match(/^__vite-optional-peer-dep:([^:]+)/);
        if (match) {
          return { id: match[1], external: false };
        }
      }
      return null;
    },
  } as Plugin;
}

export default defineConfig(async () => ({
  base: "./",
  plugins: [
    react(),
    tailwindcss(),
    monacoEditorPlugin({}),
    shikiLanguageFilter(),
    removeCrossorigin(),
    optionalPeerDepPlugin(),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  css: {
    transformer: "lightningcss",
    lightningcss: {
      errorRecovery: true, // Tailwind v4 CSS var() 在 @media 中会触发 strict 模式错误
    },
  },
  build: {
    sourcemap: false, // 生产构建不暴露源码
    modulePreload: { polyfill: false },
    chunkSizeWarningLimit: 1000, // 1MB 合理上限，及时捕获包体积回归
    rolldownOptions: {
      output: {
        codeSplitting: {
          minSize: 20000,
          groups: [
            // ── High-priority named groups (split BEFORE the generic vendor group) ──
            {
              name: "monaco-editor",
              test: /monaco-editor/,
              priority: 30,
            },
            {
              name: "markstream",
              test: /markstream/,
              priority: 25,
            },
            {
              name: "antd-vendor",
              test: /node_modules\/(?:antd|@ant-design|antd-style|@lobehub|@rc-component|rc-[^/]+)/,
              priority: 20,
            },
            {
              name: "react-vendor",
              test: /node_modules\/(?:react[^/]*|scheduler|react-dom|react-router|@remix-run)/,
              priority: 20,
            },
            {
              name: "tauri-vendor",
              test: /node_modules\/@tauri-apps/,
              priority: 20,
            },
            {
              name: "markdown-vendor",
              test: /node_modules\/(?:stream-markdown|stream-monaco|katex)/,
              priority: 20,
            },
            {
              name: "i18n-vendor",
              test: /node_modules\/(?:i18next|react-i18next)/,
              priority: 20,
            },
            {
              name: "ui-vendor",
              test:
                /node_modules\/(?:lucide-react|overlayscrollbars|clsx|emoji-picker-element|html2canvas|@tanstack|reactflow|@atlaskit)/,
              priority: 20,
            },
            {
              name: "d2-vendor",
              test: /node_modules\/@terrastruct\/d2/,
              priority: 20,
            },
            // ── Fallback: everything else in node_modules ──
            {
              name: "vendor",
              test: /node_modules/,
              priority: 10,
            },
          ],
        },
      },
    },
  },
  clearScreen: false,
  optimizeDeps: {
    include: ["immer"],
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    // React 19 scheduler setImmediate 回调在 jsdom 销毁后访问 window → ReferenceError
    // pool: "forks" 使每个测试文件独立进程，进程退出时所有 pending 回调一并终止
    pool: "forks",
    // TODO: 这些测试需要修复 CI 兼容性问题后重新启用
    // 已知问题：mock 依赖缺失、异步竞态导致 CI 环境偶发失败
    exclude: [],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      exclude: ["src/test/**", "src/**/*.test.*", "src/**/*.spec.*"],
    },
    // immer 是 zustand 的可选 peer dep，vitest 分叉进程中的
    // __vite-optional-peer-dep 解析机制需要显式 inline 才能找到包
    deps: {
      inline: ["immer"],
    },
  },
}));
