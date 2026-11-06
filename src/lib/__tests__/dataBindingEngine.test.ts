// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { SkillPermissions } from "@/types";
import { resolveDataSource, subscribeDataSource } from "../dynamicUI/DataBindingEngine";

const registryGet = vi.fn();

vi.mock("@/lib/storeRegistry", () => ({
  getStoreRegistry: () => ({
    get: registryGet,
  }),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
}));

vi.mock("@/stores/feature/executionStore", () => ({
  useExecutionStore: {
    getState: vi.fn(),
  },
}));

describe("DataBindingEngine", () => {
  describe("resolveDataSource", () => {
    describe("static 类型", () => {
      it("应返回静态值", async () => {
        const result = await resolveDataSource({
          type: "static",
          config: { value: "hello" },
        });
        expect(result).toBe("hello");
      });

      it("应返回数字静态值", async () => {
        const result = await resolveDataSource({
          type: "static",
          config: { value: 42 },
        });
        expect(result).toBe(42);
      });

      it("应返回 null 静态值", async () => {
        const result = await resolveDataSource({
          type: "static",
          config: { value: null },
        });
        expect(result).toBeNull();
      });
    });

    describe("store 类型", () => {
      it("应返回整个 store 状态", async () => {
        const state = { user: { name: "Alice" }, count: 5 };
        registryGet.mockReturnValue({
          get: () => state,
          subscribe: () => vi.fn(),
        });

        const result = await resolveDataSource({
          type: "store",
          config: { storeName: "userStore" },
        });
        expect(result).toEqual(state);
      });

      it("应使用 selector 获取嵌套值", async () => {
        const state = { profile: { settings: { theme: "dark" } } };
        registryGet.mockReturnValue({
          get: () => state,
          subscribe: () => vi.fn(),
        });

        const result = await resolveDataSource({
          type: "store",
          config: { storeName: "userStore", selector: "profile.settings.theme" },
        });
        expect(result).toBe("dark");
      });

      it("store 未注册时应抛出错误", async () => {
        registryGet.mockReturnValue(null);

        await expect(
          resolveDataSource({
            type: "store",
            config: { storeName: "missing" },
          }),
        ).rejects.toThrow();
      });
    });

    describe("api 类型", () => {
      it("invoke 方法应调用 invoke（真实命令名，#204 回归）", async () => {
        const { invoke } = await import("@/lib/invoke");
        vi.mocked(invoke).mockClear();
        vi.mocked(invoke).mockResolvedValue({ ok: true });

        // 必须用仓内真实存在的命令名：上一版这里写的是 `dynamic_ui_get_schema` ——
        // 一条不存在的命令，只为凑过 `startsWith("dynamic_ui_")` 的旧前缀判据而存在。
        // 本仓命令名动词前置，真实形态是 `get_dynamic_ui_schema`。
        const result = await resolveDataSource({
          type: "api",
          config: { endpoint: "get_dynamic_ui_schema", method: "invoke", params: { id: 1 } },
        });
        expect(invoke).toHaveBeenCalledWith("get_dynamic_ui_schema", { id: 1 });
        expect(result).toEqual({ ok: true });
      });

      it("invoke 方法应拒绝旧前缀形态的假命令名（负控，锁死 #204 的成因）", async () => {
        const { invoke } = await import("@/lib/invoke");
        vi.mocked(invoke).mockClear();
        vi.mocked(invoke).mockResolvedValue({ ok: true });

        // `dynamic_ui_` 前缀开头但不是任何真实命令 ⇒ 显式枚举白名单必须拒
        await expect(
          resolveDataSource({
            type: "api",
            config: { endpoint: "dynamic_ui_get_schema", method: "invoke" },
          }),
        ).rejects.toThrow();
        expect(invoke).not.toHaveBeenCalled();
      });

      it("invoke 方法应阻止白名单外的端点", async () => {
        const { invoke } = await import("@/lib/invoke");
        vi.mocked(invoke).mockClear();
        vi.mocked(invoke).mockResolvedValue({ ok: true });

        await expect(
          resolveDataSource({
            type: "api",
            config: { endpoint: "get_data", method: "invoke", params: { id: 1 } },
          }),
        ).rejects.toThrow();
        expect(invoke).not.toHaveBeenCalled();
      });

      it("fetch 方法成功时应返回 JSON", async () => {
        globalThis.fetch = vi.fn().mockResolvedValue({
          ok: true,
          json: () => Promise.resolve({ data: "fetched" }),
        });

        const result = await resolveDataSource({
          type: "api",
          config: { endpoint: "https://api.example.com", method: "fetch" },
        });
        expect(result).toEqual({ data: "fetched" });
      });

      it("fetch 失败时应抛出错误", async () => {
        globalThis.fetch = vi.fn().mockResolvedValue({
          ok: false,
          statusText: "Not Found",
        });

        await expect(
          resolveDataSource({
            type: "api",
            config: { endpoint: "https://api.example.com", method: "fetch" },
          }),
        ).rejects.toThrow();
      });

      // ── #206：RequestInit 白名单化（Schema 原文不再整体透传）──────────────
      // 每组负控前都有同型正控先证明 fetch 真的带着收敛后的 init 执行了。
      describe("fetch RequestInit 白名单（#206）", () => {
        const mockFetchOk = () => {
          const fetchMock = vi.fn().mockResolvedValue({
            ok: true,
            json: () => Promise.resolve({ ok: 1 }),
          });
          globalThis.fetch = fetchMock;
          return fetchMock;
        };

        it("正控：白名单内字段（GET + content-type 头）收敛后原样传给 fetch", async () => {
          const fetchMock = mockFetchOk();

          await resolveDataSource({
            type: "api",
            config: {
              endpoint: "https://api.example.com/x",
              method: "fetch",
              params: { method: "get", headers: { "Content-Type": "application/json" } },
            },
          });
          expect(fetchMock).toHaveBeenCalledWith("https://api.example.com/x", {
            method: "GET",
            headers: { "Content-Type": "application/json" },
          });
        });

        it("正控：POST + 字符串 body 放行", async () => {
          const fetchMock = mockFetchOk();

          await resolveDataSource({
            type: "api",
            config: {
              endpoint: "https://api.example.com/x",
              method: "fetch",
              params: { method: "POST", body: '{"a":1}' },
            },
          });
          expect(fetchMock).toHaveBeenCalledWith("https://api.example.com/x", {
            method: "POST",
            body: '{"a":1}',
          });
        });

        it("无 params 时以空 init 请求（正控）", async () => {
          const fetchMock = mockFetchOk();

          await resolveDataSource({
            type: "api",
            config: { endpoint: "https://api.example.com/x", method: "fetch" },
          });
          expect(fetchMock).toHaveBeenCalledWith("https://api.example.com/x", {});
        });

        it("负控：白名单外字段（credentials）被拒且不发请求", async () => {
          const fetchMock = mockFetchOk();

          await expect(
            resolveDataSource({
              type: "api",
              config: {
                endpoint: "https://api.example.com/x",
                method: "fetch",
                params: { credentials: "include" },
              },
            }),
          ).rejects.toThrow();
          expect(fetchMock).not.toHaveBeenCalled();
        });

        it("负控：Authorization 头被拒（跨域凭据面不容 Schema 伪造）", async () => {
          const fetchMock = mockFetchOk();

          await expect(
            resolveDataSource({
              type: "api",
              config: {
                endpoint: "https://api.example.com/x",
                method: "fetch",
                params: { headers: { Authorization: "Bearer sk-xxx" } },
              },
            }),
          ).rejects.toThrow();
          expect(fetchMock).not.toHaveBeenCalled();
        });

        it("负控：非 GET/POST 方法（PUT/DELETE）被拒", async () => {
          const fetchMock = mockFetchOk();

          await expect(
            resolveDataSource({
              type: "api",
              config: { endpoint: "https://api.example.com/x", method: "fetch", params: { method: "PUT" } },
            }),
          ).rejects.toThrow();
          await expect(
            resolveDataSource({
              type: "api",
              config: { endpoint: "https://api.example.com/x", method: "fetch", params: { method: "DELETE" } },
            }),
          ).rejects.toThrow();
          expect(fetchMock).not.toHaveBeenCalled();
        });

        it("负控：GET 携带 body 被拒；非字符串 body 被拒", async () => {
          const fetchMock = mockFetchOk();

          await expect(
            resolveDataSource({
              type: "api",
              config: { endpoint: "https://api.example.com/x", method: "fetch", params: { body: "x" } },
            }),
          ).rejects.toThrow();
          await expect(
            resolveDataSource({
              type: "api",
              config: {
                endpoint: "https://api.example.com/x",
                method: "fetch",
                params: { method: "POST", body: { obj: 1 } },
              },
            }),
          ).rejects.toThrow();
          expect(fetchMock).not.toHaveBeenCalled();
        });
      });
    });

    describe("agent-generated 类型", () => {
      it("应返回 agent 生成数据", async () => {
        const { useExecutionStore } = await import("@/stores/feature/executionStore");
        const mockState = {
          gen_123: { data: "generated", score: 0.95 },
        };
        vi.mocked(useExecutionStore.getState).mockReturnValue(
          mockState as unknown as ReturnType<typeof useExecutionStore.getState>,
        );

        const result = await resolveDataSource({
          type: "agent-generated",
          config: { generationId: "gen_123" },
        });
        expect(result).toEqual({ data: "generated", score: 0.95 });
      });

      it("应使用 selector 获取嵌套值", async () => {
        const { useExecutionStore } = await import("@/stores/feature/executionStore");
        const mockState = {
          gen_456: { output: { result: "nested" } },
        };
        vi.mocked(useExecutionStore.getState).mockReturnValue(
          mockState as unknown as ReturnType<typeof useExecutionStore.getState>,
        );

        const result = await resolveDataSource({
          type: "agent-generated",
          config: { generationId: "gen_456", selector: "output.result" },
        });
        expect(result).toBe("nested");
      });

      it("generationId 不存在时应抛出错误", async () => {
        const { useExecutionStore } = await import("@/stores/feature/executionStore");
        vi.mocked(useExecutionStore.getState).mockReturnValue(
          {} as unknown as ReturnType<typeof useExecutionStore.getState>,
        );

        await expect(
          resolveDataSource({
            type: "agent-generated",
            config: { generationId: "nonexistent" },
          }),
        ).rejects.toThrow();
      });
    });
  });

  describe("subscribeDataSource", () => {
    it("应返回 unsubscribe 对象", async () => {
      const subscriber = await subscribeDataSource(
        { type: "static", config: { value: "test" } },
        vi.fn(),
      );
      expect(subscriber).toHaveProperty("unsubscribe");
      expect(typeof subscriber.unsubscribe).toBe("function");
    });

    it("初始加载应调用 onData", async () => {
      const onData = vi.fn();
      await subscribeDataSource(
        { type: "static", config: { value: "initial" } },
        onData,
      );
      expect(onData).toHaveBeenCalledWith("initial");
    });

    it("取消订阅后不应再更新", async () => {
      const onData = vi.fn();
      const subscriber = await subscribeDataSource(
        { type: "static", config: { value: "test" } },
        onData,
      );
      subscriber.unsubscribe();
      onData.mockClear();
      await subscribeDataSource(
        { type: "static", config: { value: "after-unsubscribe" } },
        onData,
      );
      expect(onData).toHaveBeenCalledWith("after-unsubscribe");
    });

    it("unsubscribe 可多次调用不被报错", async () => {
      const subscriber = await subscribeDataSource(
        { type: "static", config: { value: "test" } },
        vi.fn(),
      );
      expect(() => {
        subscriber.unsubscribe();
        subscriber.unsubscribe();
      }).not.toThrow();
    });
  });

  /**
   * 来源受限权限面在 dataSource 这条路上的执法。
   *
   * 每条负控都配一条同型正控：先证明「带上权限面并没有把路整体打死」，
   * 才谈得上「拒绝是因为判据命中，不是因为链路没起来」。
   */
  describe("来源受限权限面", () => {
    // 与 `PLUGIN_SCHEMA_ALLOWED_COMMANDS`（`src/lib/dynamicUI/schemaActionPermissions.ts`）同形：
    // 2026-11-06 收口后插件档只剩 `get_dynamic_ui_schema` 一条只读投影，表单三条全部摘除
    // （判据：`args` 不可校验 ⇒ 表单命令进了面就等于让 dataSource 端点借 `params.schemaId`
    // 指向他人的宿主行）。本夹具断言的是「在面上/不在面上」这套机制，命令名跟着权威清单走。
    const pluginFace = {
      commands: ["get_dynamic_ui_schema"],
      events: [],
      storeRead: [],
      storeWrite: [],
      navigate: [],
    } as unknown as SkillPermissions;

    const readProviderStore = {
      commands: [],
      storeRead: ["providerStore:proxyUrl"],
    } as unknown as SkillPermissions;

    it("invoke 端点：面上有则放行（正控）", async () => {
      const { invoke } = await import("@/lib/invoke");
      vi.mocked(invoke).mockClear();
      vi.mocked(invoke).mockResolvedValue({ rows: [] });

      const result = await resolveDataSource(
        {
          type: "api",
          config: { endpoint: "get_dynamic_ui_schema", method: "invoke", params: { id: "s-1" } },
        },
        pluginFace,
      );
      expect(invoke).toHaveBeenCalledWith("get_dynamic_ui_schema", { id: "s-1" });
      expect(result).toEqual({ rows: [] });
    });

    it("invoke 端点：dynamic_ui_ 前缀内但不在面上的写命令被拒，且不打 IPC（负控）", async () => {
      const { invoke } = await import("@/lib/invoke");
      vi.mocked(invoke).mockClear();
      vi.mocked(invoke).mockResolvedValue({ ok: true });

      // create_dynamic_ui_schema 会把 origin 硬写成 "user"（可信）⇒ 放行即提权
      await expect(
        resolveDataSource(
          { type: "api", config: { endpoint: "create_dynamic_ui_schema", method: "invoke" } },
          pluginFace,
        ),
      ).rejects.toThrow();
      await expect(
        resolveDataSource(
          { type: "api", config: { endpoint: "delete_dynamic_ui_schema", method: "invoke" } },
          pluginFace,
        ),
      ).rejects.toThrow();
      // 表单三条（2026-11-06 起不在插件档）同样拒：端点在 dataSource 这条路上也要过面
      await expect(
        resolveDataSource(
          {
            type: "api",
            config: { endpoint: "save_dynamic_ui_form_data", method: "invoke", params: { schemaId: "s-1" } },
          },
          pluginFace,
        ),
      ).rejects.toThrow();
      expect(invoke).not.toHaveBeenCalled();
    });

    // ── #206：fetch 端点同样必须过权限面（此前 fetch 分支一条权限都不查）──────
    it("fetch 端点：面上没有该端点 ⇒ 拒且不发请求（负控，插件档现状 = 全部拒）", async () => {
      const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({}) });
      globalThis.fetch = fetchMock;

      // pluginFace.commands 只有命令名 `get_dynamic_ui_schema`，不含任何 URL ⇒
      // 插件来源声明 fetch 数据源 today 一律 fail-closed
      await expect(
        resolveDataSource(
          { type: "api", config: { endpoint: "https://api.example.com/x", method: "fetch" } },
          pluginFace,
        ),
      ).rejects.toThrow();
      expect(fetchMock).not.toHaveBeenCalled();
    });

    it("fetch 端点：面上以通配声明该端点族 ⇒ 放行（同文件正控，证明负控不是链路没起来）", async () => {
      const fetchMock = vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ data: 1 }),
      });
      globalThis.fetch = fetchMock;

      const fetchFace = {
        commands: ["https://api.example.com/*"],
        events: [],
        storeRead: [],
        storeWrite: [],
        navigate: [],
      } as unknown as SkillPermissions;

      const result = await resolveDataSource(
        { type: "api", config: { endpoint: "https://api.example.com/x", method: "fetch" } },
        fetchFace,
      );
      expect(fetchMock).toHaveBeenCalledWith("https://api.example.com/x", {});
      expect(result).toEqual({ data: 1 });
    });

    it("store 端点：storeRead 为空 ⇒ 拒读任意已注册 store（负控）", async () => {
      registryGet.mockClear();
      registryGet.mockReturnValue({ get: () => ({ apiKey: "sk" }), subscribe: () => vi.fn() });

      await expect(
        resolveDataSource({ type: "store", config: { storeName: "providerStore" } }, pluginFace),
      ).rejects.toThrow();
      // 判定必须发生在查 registry 之前，否则 store 实例已经到手
      expect(registryGet).not.toHaveBeenCalled();
    });

    it("store 端点：storeRead 覆盖到该 store 及其字段 ⇒ 放行（正控）", async () => {
      registryGet.mockReturnValue({
        get: () => ({ proxyUrl: "https://p", apiKey: "sk" }),
        subscribe: () => vi.fn(),
      });

      const result = await resolveDataSource(
        { type: "store", config: { storeName: "providerStore", selector: "proxyUrl" } },
        readProviderStore,
      );
      expect(result).toBe("https://p");
    });

    it("store 端点：覆盖某 store 不等于覆盖其全部字段，未声明字段仍拒（负控）", async () => {
      registryGet.mockReturnValue({
        get: () => ({ proxyUrl: "https://p", apiKey: "sk" }),
        subscribe: () => vi.fn(),
      });

      await expect(
        resolveDataSource(
          { type: "store", config: { storeName: "providerStore", selector: "apiKey" } },
          readProviderStore,
        ),
      ).rejects.toThrow();
    });

    it("不传权限面（可信来源）⇒ 行为逐字不变（正控：白名单仍生效，面不生效）", async () => {
      const { invoke } = await import("@/lib/invoke");
      vi.mocked(invoke).mockClear();
      vi.mocked(invoke).mockResolvedValue({ ok: true });

      await expect(
        resolveDataSource({ type: "api", config: { endpoint: "delete_dynamic_ui_schema", method: "invoke" } }),
      ).resolves.toEqual({ ok: true });
      await expect(
        resolveDataSource({ type: "api", config: { endpoint: "get_data", method: "invoke" } }),
      ).rejects.toThrow();
    });

    it("subscribeDataSource：受限面在 store 直订阅分支前就拦（回归：该分支绕过 resolveDataSource）", async () => {
      const onData = vi.fn();
      const onError = vi.fn();
      registryGet.mockClear();
      registryGet.mockReturnValue({ get: () => ({ apiKey: "sk" }), subscribe: () => vi.fn() });

      const subscriber = await subscribeDataSource(
        { type: "store", config: { storeName: "providerStore" } },
        onData,
        onError,
        pluginFace,
      );
      // 初始加载的 reject 只走 onError，流程会继续往下 —— 所以拦截必须发生在订阅之前
      expect(onError).toHaveBeenCalled();
      expect(onData).not.toHaveBeenCalled();
      expect(registryGet).not.toHaveBeenCalled();
      subscriber.unsubscribe();
    });
  });
});
