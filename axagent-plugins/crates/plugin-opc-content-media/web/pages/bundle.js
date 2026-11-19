"use strict";
(() => {
  // vendor/sdk/sandboxFrameProtocol.ts
  var SANDBOX_FRAME_TYPES = {
    /** 沙箱 → 宿主：一次 RPC 请求 */
    request: "rpc:request",
    /** 宿主 → 沙箱：一次 RPC 的响应（成功或失败共用这一帧） */
    response: "rpc:response",
    /** 沙箱 → 宿主：内容已就绪，宿主可以开始接它的请求 */
    ready: "skill:ready",
    /** 沙箱 → 宿主：内容内部报错（运行期异常 / 未捕获拒绝） */
    contentError: "skill:error",
    /** 宿主 → 沙箱：事件推送 */
    hostEvent: "host:event",
    /** 宿主 → 沙箱：生命周期（挂载 / 卸载） */
    hostLifecycle: "host:lifecycle",
  };
  var SANDBOX_RPC_METHODS = {
    apiInvoke: "api.invoke",
    apiEmit: "api.emit",
    uiNavigate: "ui.navigate",
    uiNotify: "ui.notify",
    uiGetTheme: "ui.getTheme",
    uiGetLocale: "ui.getLocale",
    storeRead: "store.read",
    storeWrite: "store.write",
  };
  var SANDBOX_FRAME_FIELDS = {
    type: "type",
    callId: "callId",
    method: "method",
    args: "args",
    result: "result",
    error: "error",
    event: "event",
    payload: "payload",
    phase: "phase",
    props: "props",
    source: "source",
    line: "line",
    col: "col",
  };
  var SANDBOX_RPC_ARG_KEYS = {
    command: "command",
    args: "args",
    event: "event",
    payload: "payload",
    path: "path",
    message: "message",
    type: "type",
    storeName: "storeName",
    selector: "selector",
    value: "value",
  };
  var SANDBOX_RPC_TIMEOUT_MS = 15e3;
  var SANDBOX_RPC_FAILURE_ERROR_TYPE = "string";
  var SANDBOX_MESSAGE_TARGET_ORIGIN = "*";
  var SANDBOX_CALL_ID_PREFIX = {
    /** 宿主向沙箱发起调用（`callSkillMethod`） */
    host: "host_",
    /** `srcdoc` 沙箱内容（Skill 与插件 panel）向宿主发起 RPC */
    sandbox: "skill_",
    /** 走 asset 协议装载的插件 bundle（page 粒度）向宿主发起 RPC */
    plugin: "plugin_",
  };
  function buildRpcRequestFrame(callId, method, args) {
    const f = SANDBOX_FRAME_FIELDS;
    const frame = {
      [f.type]: SANDBOX_FRAME_TYPES.request,
      [f.callId]: callId,
      [f.method]: method,
    };
    if (args !== void 0) {
      frame[f.args] = args;
    }
    return frame;
  }
  function buildSandboxReadyFrame() {
    return { [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.ready };
  }
  function formatCallId(prefix, sequence) {
    return `${prefix}${sequence}_${Date.now()}`;
  }
  function frameTypeOf(data) {
    if (!data || typeof data !== "object") {
      return void 0;
    }
    const value = data[SANDBOX_FRAME_FIELDS.type];
    return typeof value === "string" ? value : void 0;
  }
  function isRpcResponseFrame(data) {
    return frameTypeOf(data) === SANDBOX_FRAME_TYPES.response;
  }
  function isFailureFrame(frame) {
    const raw = frame[SANDBOX_FRAME_FIELDS.error];
    return typeof raw === SANDBOX_RPC_FAILURE_ERROR_TYPE;
  }
  function readRpcResponseCallId(frame) {
    const raw = frame[SANDBOX_FRAME_FIELDS.callId];
    return typeof raw === "string" ? raw : void 0;
  }
  function isFrameFromSandboxSource(event, sandboxWindow) {
    if (sandboxWindow === null || sandboxWindow === void 0) {
      return false;
    }
    return event.source === sandboxWindow;
  }

  // vendor/sdk/pluginBridgeProtocol.ts
  var PLUGIN_BRIDGE_COMMAND_NAMES = {
    /** 动态 UI / 沙箱内容把用户动作回流给**该插件自己的** worker（后端 `plugin_ui_action`）。 */
    uiAction: "plugin_ui_action",
    /** 按名字调用该插件声明的命令（后端 `plugin_execute_command`）。 */
    executeCommand: "plugin_execute_command",
  };
  var PLUGIN_BRIDGE_COMMANDS = [
    PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
    PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
  ];
  function isPluginBridgeCommand(command) {
    return PLUGIN_BRIDGE_COMMANDS.includes(command);
  }
  var HOST_BOUND_PLUGIN_ID_FIELD = "pluginId";
  function omitHostBoundPluginId(args) {
    if (!args) {
      return {};
    }
    const { [HOST_BOUND_PLUGIN_ID_FIELD]: _ignored, ...rest } = args;
    return rest;
  }
  var PLUGIN_UI_ACTION_ARG_KEY = "action";
  var PLUGIN_BRIDGE_CLIENT_METHODS = [SANDBOX_RPC_METHODS.apiInvoke];

  // vendor/sdk/pluginClientBridge.ts
  function adaptParentWindow(parent) {
    return {
      postMessage: (message, targetOrigin) => {
        parent.postMessage(message, targetOrigin);
      },
    };
  }
  function adaptWindowSource(win) {
    return {
      addEventListener: (type, listener) => {
        win.addEventListener(type, listener);
      },
      removeEventListener: (type, listener) => {
        win.removeEventListener(type, listener);
      },
    };
  }
  function createAxAgentPluginClient(options = {}) {
    const globalWindow = typeof window === "undefined" ? void 0 : window;
    const rawTarget = options.postTarget ?? (globalWindow ? adaptParentWindow(globalWindow.parent) : void 0);
    const rawSource = options.messageSource ?? (globalWindow ? adaptWindowSource(globalWindow) : void 0);
    if (!rawTarget || !rawSource) {
      throw new Error(
        "createAxAgentPluginClient: \u9700\u8981\u6D4F\u89C8\u5668\u7A97\u53E3\u73AF\u5883\uFF08window.parent / window\uFF09\u6216\u663E\u5F0F\u6CE8\u5165 postTarget \u4E0E messageSource",
      );
    }
    const target = rawTarget;
    const source = rawSource;
    const targetOrigin = options.targetOrigin ?? SANDBOX_MESSAGE_TARGET_ORIGIN;
    const timeoutMs = options.timeoutMs ?? SANDBOX_RPC_TIMEOUT_MS;
    const hostWindow = options.hostWindow ?? (globalWindow ? globalWindow.parent : void 0);
    const pending = /* @__PURE__ */ new Map();
    let sequence = 0;
    let disposed = false;
    function handleMessage(event) {
      if (disposed) {
        return;
      }
      if (!isFrameFromSandboxSource({ data: event.data, source: event.source }, hostWindow)) {
        return;
      }
      const frame = event.data;
      if (!isRpcResponseFrame(frame)) {
        return;
      }
      const callId = readRpcResponseCallId(frame);
      if (!callId) {
        return;
      }
      const entry = pending.get(callId);
      if (!entry) {
        return;
      }
      clearTimeout(entry.timer);
      pending.delete(callId);
      if (isFailureFrame(frame)) {
        entry.reject(new Error(frame.error));
      } else {
        entry.resolve(frame.result);
      }
    }
    source.addEventListener("message", handleMessage);
    function request(command, args) {
      if (disposed) {
        return Promise.reject(
          new Error(
            "\u63D2\u4EF6\u6865\u5BA2\u6237\u7AEF\u5DF2\u91CA\u653E\uFF08dispose \u540E\u4E0D\u53EF\u518D\u8C03\u7528\uFF09",
          ),
        );
      }
      const callId = formatCallId(SANDBOX_CALL_ID_PREFIX.plugin, ++sequence);
      const envelope = { [SANDBOX_RPC_ARG_KEYS.command]: command };
      if (args !== void 0) {
        envelope[SANDBOX_RPC_ARG_KEYS.args] = omitHostBoundPluginId(args);
      }
      const frame = buildRpcRequestFrame(
        callId,
        SANDBOX_RPC_METHODS.apiInvoke,
        envelope,
      );
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(callId);
          reject(
            new Error(
              `\u63D2\u4EF6\u6865\u8C03\u7528\u300C${command}\u300D\u8D85\u65F6\uFF08${timeoutMs}ms \u672A\u6536\u5230\u5BBF\u4E3B\u54CD\u5E94\uFF09`,
            ),
          );
        }, timeoutMs);
        pending.set(callId, { resolve, reject, timer });
        try {
          target.postMessage(frame, targetOrigin);
        } catch (error) {
          clearTimeout(timer);
          pending.delete(callId);
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
    }
    return {
      invoke: (command, args) => {
        if (!isPluginBridgeCommand(command)) {
          return Promise.reject(
            new Error(
              `\u63D2\u4EF6\u6865\u767D\u540D\u5355\u5916\u7684\u547D\u4EE4\u300C${
                String(command)
              }\u300D\uFF1A\u53EF\u7528\u6E05\u5355\u89C1 AX_AGENT_PLUGIN_BRIDGE_COMMANDS\uFF0C\u65B0\u589E\u901A\u9053\u9700\u6539 pluginBridgeProtocol.ts \u5E76\u7ECF\u5BBF\u4E3B\u4FA7\u8BC4\u5BA1`,
            ),
          );
        }
        return request(command, args);
      },
      uiAction: (action) =>
        request(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction, {
          [PLUGIN_UI_ACTION_ARG_KEY]: action,
        }),
      executeCommand: (args) => request(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand, args),
      announceReady() {
        if (disposed) {
          return;
        }
        const frame = buildSandboxReadyFrame();
        try {
          target.postMessage(frame, targetOrigin);
        } catch {
        }
      },
      dispose() {
        disposed = true;
        source.removeEventListener("message", handleMessage);
        for (const entry of pending.values()) {
          clearTimeout(entry.timer);
          entry.reject(new Error("\u63D2\u4EF6\u6865\u5BA2\u6237\u7AEF\u5DF2\u91CA\u653E\uFF08dispose\uFF09"));
        }
        pending.clear();
      },
    };
  }

  // src-web/main.ts
  var client = createAxAgentPluginClient();
  client.announceReady();
  function text(value) {
    return value === null || value === void 0 ? "-" : String(value);
  }
  function parseValue(raw) {
    if (typeof raw !== "string") {
      return raw;
    }
    try {
      return JSON.parse(raw);
    } catch {
      return raw;
    }
  }
  function parseRows(raw) {
    const value = parseValue(raw);
    if (!Array.isArray(value)) {
      return [];
    }
    return value.filter((row) => row !== null && typeof row === "object");
  }
  function errorText(error) {
    return error instanceof Error ? error.message : String(error);
  }
  function formattedTime(value) {
    if (typeof value !== "number") {
      return "-";
    }
    const millis = value < 1e10 ? value * 1e3 : value;
    return new Date(millis).toLocaleString();
  }
  function setState(element, value, isError = false) {
    if (!element) {
      return;
    }
    element.className = isError ? "error" : "state";
    element.textContent = value;
  }
  function fillRows(tableSelector, emptyText, state, rows) {
    const table = document.querySelector(tableSelector);
    const tbody = table?.querySelector("tbody");
    if (!table || !tbody) {
      return;
    }
    tbody.replaceChildren();
    if (rows.length === 0) {
      table.hidden = true;
      setState(state, emptyText);
      return;
    }
    for (const cells of rows) {
      const tr = document.createElement("tr");
      for (const value of cells) {
        const td = document.createElement("td");
        if (value instanceof Node) {
          td.append(value);
        } else {
          td.textContent = text(value);
        }
        tr.append(td);
      }
      tbody.append(tr);
    }
    table.hidden = false;
  }
  function actionButton(label, handler) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", () => void handler());
    return button;
  }
  var blogState = document.querySelector("#blog-list-state");
  var assetState = document.querySelector("#asset-list-state");
  var scheduleState = document.querySelector("#schedule-list-state");
  var blogCreateState = document.querySelector("#blog-state");
  var assetCreateState = document.querySelector("#asset-create-state");
  var scheduleCreateState = document.querySelector("#schedule-create-state");
  async function loadBlogs() {
    setState(blogState, "\u52A0\u8F7D\u4E2D\u2026");
    try {
      const rows = parseRows(
        await client.executeCommand({ name: "opc_list_blog_posts", input: {} }),
      );
      fillRows(
        "#blog-posts",
        "\u6682\u65E0\u535A\u5BA2",
        blogState,
        rows.map((row) => [
          row.title,
          row.slug,
          row.published === true ? "\u5DF2\u53D1\u5E03" : "\u8349\u7A3F",
          row.viewCount,
          formattedTime(row.updatedAt),
        ]),
      );
      if (rows.length > 0) {
        setState(blogState, `\u5171 ${rows.length} \u7BC7\u535A\u5BA2`);
      }
    } catch (error) {
      setState(blogState, `\u52A0\u8F7D\u535A\u5BA2\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  async function loadAssets() {
    setState(assetState, "\u52A0\u8F7D\u4E2D\u2026");
    try {
      const rows = parseRows(
        await client.executeCommand({ name: "opc_list_content_assets", input: {} }),
      );
      fillRows(
        "#content-assets",
        "\u6682\u65E0\u5185\u5BB9\u8D44\u4EA7",
        assetState,
        rows.map((row) => {
          const id = String(row.id ?? "");
          return [
            row.title,
            row.contentType,
            row.status,
            Array.isArray(row.tags) ? row.tags.join(", ") : "-",
            formattedTime(row.createdAt),
            actionButton("\u5220\u9664", async () => {
              if (!id) {
                return;
              }
              setState(assetState, "\u5220\u9664\u4E2D\u2026");
              try {
                await client.executeCommand({ name: "opc_delete_content_asset", input: { id } });
                await loadAssets();
              } catch (error) {
                setState(assetState, `\u5220\u9664\u5931\u8D25\uFF1A${errorText(error)}`, true);
              }
            }),
          ];
        }),
      );
      if (rows.length > 0) {
        setState(assetState, `\u5171 ${rows.length} \u6761\u5185\u5BB9\u8D44\u4EA7`);
      }
    } catch (error) {
      setState(assetState, `\u52A0\u8F7D\u5185\u5BB9\u8D44\u4EA7\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  async function loadSchedules() {
    setState(scheduleState, "\u52A0\u8F7D\u4E2D\u2026");
    try {
      const rows = parseRows(
        await client.executeCommand({ name: "opc_list_publish_schedules", input: {} }),
      );
      fillRows(
        "#publish-schedules",
        "\u6682\u65E0\u53D1\u5E03\u8BA1\u5212",
        scheduleState,
        rows.map((row) => {
          const id = String(row.id ?? "");
          return [
            `${text(row.contentRefType)} / ${text(row.contentRefId)}`,
            row.status,
            formattedTime(row.scheduledAt),
            formattedTime(row.publishedAt),
            actionButton("\u53D6\u6D88", async () => {
              if (!id) {
                return;
              }
              setState(scheduleState, "\u53D6\u6D88\u4E2D\u2026");
              try {
                await client.executeCommand({
                  name: "opc_cancel_publish_schedule",
                  input: { id },
                });
                await loadSchedules();
              } catch (error) {
                setState(scheduleState, `\u53D6\u6D88\u5931\u8D25\uFF1A${errorText(error)}`, true);
              }
            }),
          ];
        }),
      );
      if (rows.length > 0) {
        setState(scheduleState, `\u5171 ${rows.length} \u6761\u53D1\u5E03\u8BA1\u5212`);
      }
    } catch (error) {
      setState(scheduleState, `\u52A0\u8F7D\u53D1\u5E03\u8BA1\u5212\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  async function createBlog(form) {
    const title = document.querySelector("#blog-title")?.value.trim() ?? "";
    const slug = document.querySelector("#blog-slug")?.value.trim() ?? "";
    const excerpt = document.querySelector("#blog-excerpt")?.value.trim() ?? "";
    const content = document.querySelector("#blog-content")?.value ?? "";
    if (!title || !slug || !excerpt || !content.trim()) {
      setState(
        blogCreateState,
        "\u8BF7\u586B\u5199\u6807\u9898\u3001\u8DEF\u5F84\u3001\u6458\u8981\u4E0E\u6B63\u6587",
        true,
      );
      return;
    }
    setState(blogCreateState, "\u521B\u5EFA\u4E2D\u2026");
    try {
      await client.executeCommand({
        name: "opc_create_blog_post",
        input: { title, slug, excerpt, content, tags: [] },
      });
      setState(blogCreateState, "\u535A\u5BA2\u521B\u5EFA\u6210\u529F");
      form.reset();
      await loadBlogs();
    } catch (error) {
      setState(blogCreateState, `\u521B\u5EFA\u535A\u5BA2\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  async function createAsset(form) {
    const title = document.querySelector("#asset-title")?.value.trim() ?? "";
    const contentType = document.querySelector("#asset-type")?.value.trim() ?? "";
    const body = document.querySelector("#asset-body")?.value ?? "";
    if (!title || !contentType || !body.trim()) {
      setState(assetCreateState, "\u8BF7\u586B\u5199\u6807\u9898\u3001\u7C7B\u578B\u4E0E\u6B63\u6587", true);
      return;
    }
    setState(assetCreateState, "\u521B\u5EFA\u4E2D\u2026");
    try {
      await client.executeCommand({
        name: "opc_create_content_asset",
        input: { title, contentType, body, tags: [] },
      });
      setState(assetCreateState, "\u5185\u5BB9\u8D44\u4EA7\u521B\u5EFA\u6210\u529F");
      form.reset();
      await loadAssets();
    } catch (error) {
      setState(assetCreateState, `\u521B\u5EFA\u5185\u5BB9\u8D44\u4EA7\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  async function createSchedule(form) {
    const contentRefType = document.querySelector("#schedule-type")?.value.trim() ?? "";
    const contentRefId = document.querySelector("#schedule-id")?.value.trim() ?? "";
    const rawTime = document.querySelector("#schedule-time")?.value ?? "";
    if (!contentRefType || !contentRefId || !rawTime) {
      setState(
        scheduleCreateState,
        "\u8BF7\u586B\u5199\u5173\u8054\u7C7B\u578B\u3001\u5173\u8054 ID \u4E0E\u8BA1\u5212\u65F6\u95F4",
        true,
      );
      return;
    }
    const millis = new Date(rawTime).getTime();
    if (!Number.isFinite(millis)) {
      setState(scheduleCreateState, "\u8BA1\u5212\u65F6\u95F4\u683C\u5F0F\u65E0\u6548", true);
      return;
    }
    setState(scheduleCreateState, "\u521B\u5EFA\u4E2D\u2026");
    try {
      await client.executeCommand({
        name: "opc_create_publish_schedule",
        input: {
          contentRefType,
          contentRefId,
          scheduledAt: Math.floor(millis / 1e3),
        },
      });
      setState(scheduleCreateState, "\u53D1\u5E03\u8BA1\u5212\u521B\u5EFA\u6210\u529F");
      form.reset();
      await loadSchedules();
    } catch (error) {
      setState(scheduleCreateState, `\u521B\u5EFA\u53D1\u5E03\u8BA1\u5212\u5931\u8D25\uFF1A${errorText(error)}`, true);
    }
  }
  document.querySelector("#blog-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    if (form instanceof HTMLFormElement) {
      void createBlog(form);
    }
  });
  document.querySelector("#asset-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    if (form instanceof HTMLFormElement) {
      void createAsset(form);
    }
  });
  document.querySelector("#schedule-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    if (form instanceof HTMLFormElement) {
      void createSchedule(form);
    }
  });
  void loadBlogs();
  void loadAssets();
  void loadSchedules();
})();
