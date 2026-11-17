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
  var BASE_CALLS = [
    { name: "stock_reflection_stats" },
    { name: "stock_reco_ic_stats", input: {} },
    { name: "stock_get_reco_strategy_weights" },
    { name: "stock_get_market_status" },
    { name: "stock_get_vendor_fallback_log" },
    { name: "stock_list_reco_history", input: { limit: 20, offset: 0 } },
    { name: "stock_list_stock_tools" },
  ];
  var PERIOD_VARIANTS = ["ultra_short", "short", "long", "mid"];
  var EMPTY_TEXT = "\u672C\u73AF\u5883\u4E0D\u63D0\u4F9B\u8BE5\u6570\u636E";
  function isEmpty(value) {
    if (value === null || value === void 0) {
      return true;
    }
    if (Array.isArray(value)) {
      return value.length === 0;
    }
    if (typeof value === "object") {
      return Object.keys(value).length === 0;
    }
    return false;
  }
  function parseReceipt(raw) {
    if (typeof raw !== "string") {
      return raw;
    }
    try {
      return JSON.parse(raw);
    } catch {
      return raw;
    }
  }
  function parseRecoRows(raw) {
    const value = parseReceipt(raw);
    if (!Array.isArray(value)) {
      return [];
    }
    return value.filter((row) => row !== null && typeof row === "object");
  }
  function extractStrategyId(raw) {
    const value = parseReceipt(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return null;
    }
    for (const key of Object.keys(value)) {
      for (const period of PERIOD_VARIANTS) {
        const suffix = `_${period}`;
        if (key.endsWith(suffix) && key.length > suffix.length) {
          return key.slice(0, key.length - suffix.length);
        }
      }
    }
    return null;
  }
  function resolveDependentCalls(firstRound) {
    const rows = parseRecoRows(firstRound.get("stock_list_reco_history"));
    const detailRow = rows.find(
      (row) => typeof row.generatedAt === "string" && row.generatedAt !== "",
    );
    const cachedRow = rows.find(
      (row) => typeof row.period === "string" && PERIOD_VARIANTS.includes(row.period),
    );
    const strategyId = extractStrategyId(firstRound.get("stock_get_reco_strategy_weights"));
    return [
      detailRow
        ? {
          call: { name: "stock_get_reco_detail", input: { generatedAt: detailRow.generatedAt } },
          skip: null,
        }
        : {
          call: { name: "stock_get_reco_detail" },
          skip:
            "\u5E93\u4E2D\u6682\u65E0\u8350\u80A1\u8BB0\u5F55\uFF08\u7F3A generatedAt\uFF09\uFF0C\u672A\u53D1\u8C03\u7528",
        },
      cachedRow
        ? {
          call: { name: "stock_get_cached_recommendation", input: { period: cachedRow.period } },
          skip: null,
        }
        : {
          call: { name: "stock_get_cached_recommendation" },
          skip:
            "\u5E93\u4E2D\u6682\u65E0\u5E26\u5408\u6CD5\u6863\u4F4D\uFF08period\uFF09\u7684\u8350\u80A1\u8BB0\u5F55\uFF0C\u672A\u53D1\u8C03\u7528",
        },
      strategyId
        ? {
          call: { name: "stock_get_reco_signal_history", input: { strategyId } },
          skip: null,
        }
        : {
          call: { name: "stock_get_reco_signal_history" },
          skip:
            "\u6743\u91CD\u53F0\u8D26\u4E3A\u7A7A\uFF08\u65E0 {strategy}_{period} \u952E\uFF09\uFF0C\u65E0\u6CD5\u786E\u5B9A\u7B56\u7565 id\uFF0C\u672A\u53D1\u8C03\u7528",
        },
    ];
  }
  async function invokeOne(call) {
    try {
      const raw = await client.executeCommand({
        name: call.name,
        ...call.input === void 0 ? {} : { input: call.input },
      });
      return { kind: "receipt", raw };
    } catch (error) {
      return { kind: "error", message: error instanceof Error ? error.message : String(error) };
    }
  }
  function renderSection(root, name, outcome) {
    const section = document.createElement("section");
    section.style.cssText = "margin:0 0 12px;padding:8px 10px;border:1px solid #ddd;border-radius:6px";
    const title = document.createElement("h2");
    title.style.cssText = "margin:0 0 6px;font-size:13px;font-weight:600";
    title.textContent = name;
    section.appendChild(title);
    const pre = document.createElement("pre");
    pre.style.cssText = "margin:0;white-space:pre-wrap;word-break:break-all;font:12px/1.5 ui-monospace,monospace";
    if (outcome.kind === "error") {
      pre.textContent = `\u8C03\u7528\u5931\u8D25\uFF1A${outcome.message}`;
    } else if (outcome.kind === "skipped") {
      pre.textContent = `\u672A\u53D1\u8C03\u7528\uFF1A${outcome.reason}`;
    } else {
      const value = parseReceipt(outcome.raw);
      pre.textContent = isEmpty(value) ? EMPTY_TEXT : JSON.stringify(value, null, 2);
    }
    section.appendChild(pre);
    root.appendChild(section);
  }
  async function main() {
    const root = document.getElementById("root");
    if (!root) {
      return;
    }
    const firstRound = /* @__PURE__ */ new Map();
    for (const call of BASE_CALLS) {
      const outcome = await invokeOne(call);
      if (outcome.kind === "receipt") {
        firstRound.set(call.name, outcome.raw);
      }
      renderSection(root, call.name, outcome);
    }
    for (const dep of resolveDependentCalls(firstRound)) {
      const outcome = dep.skip === null ? await invokeOne(dep.call) : { kind: "skipped", reason: dep.skip };
      renderSection(root, dep.call.name, outcome);
    }
  }
  void main();
  window.addEventListener("beforeunload", () => client.dispose());
})();
