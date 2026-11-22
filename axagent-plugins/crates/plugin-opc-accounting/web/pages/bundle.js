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
  var listState = document.querySelector("#list-state");
  var table = document.querySelector("#invoices");
  var tbody = table?.querySelector("tbody");
  var createState = document.querySelector("#create-state");
  var customerInput = document.querySelector("#customer-id");
  var descInput = document.querySelector("#item-desc");
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
  async function reloadInvoices() {
    if (!listState || !table || !tbody) {
      return;
    }
    listState.textContent = "\u52A0\u8F7D\u4E2D\u2026";
    listState.className = "state";
    table.hidden = true;
    tbody.replaceChildren();
    try {
      const raw = await client.executeCommand({ name: "opc_list_invoices", input: {} });
      const rows = parseRows(raw);
      if (rows.length === 0) {
        listState.textContent = "\u6682\u65E0\u53D1\u7968";
        return;
      }
      for (const row of rows) {
        const tr = document.createElement("tr");
        for (
          const value of [
            text(row.invoiceNumber ?? row.id),
            text(row.customerId),
            text(row.status),
            text(row.total),
          ]
        ) {
          const td = document.createElement("td");
          td.textContent = value;
          tr.append(td);
        }
        const action = document.createElement("td");
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = "\u6807\u8BB0\u5DF2\u4ED8";
        button.addEventListener("click", () => void transition(String(row.id ?? "")));
        action.append(button);
        tr.append(action);
        tbody.append(tr);
      }
      listState.textContent = `\u5171 ${rows.length} \u6761`;
      table.hidden = false;
    } catch (error) {
      listState.className = "error";
      listState.textContent = `\u52A0\u8F7D\u5931\u8D25\uFF1A${errorText(error)}`;
    }
  }
  async function createInvoice() {
    if (!createState) {
      return;
    }
    const customerId = customerInput?.value.trim() ?? "";
    const description = descInput?.value.trim() ?? "";
    if (!customerId || !description) {
      createState.className = "error";
      createState.textContent = "\u8BF7\u586B\u5BA2\u6237 ID \u4E0E\u9879\u76EE\u63CF\u8FF0";
      return;
    }
    createState.className = "state";
    createState.textContent = "\u521B\u5EFA\u4E2D\u2026";
    try {
      await client.executeCommand({
        name: "opc_create_invoice",
        input: {
          customerId,
          lineItems: [{ description, quantity: 1, unitPrice: 0, taxRate: 0 }],
        },
      });
      createState.textContent = "\u521B\u5EFA\u6210\u529F";
      descInput && (descInput.value = "");
      await reloadInvoices();
    } catch (error) {
      createState.className = "error";
      createState.textContent = `\u521B\u5EFA\u5931\u8D25\uFF1A${errorText(error)}`;
    }
  }
  async function transition(id) {
    if (!listState || !id) {
      return;
    }
    listState.className = "state";
    listState.textContent = "\u72B6\u6001\u53D8\u66F4\u4E2D\u2026";
    try {
      const raw = await client.executeCommand({
        name: "opc_transition_invoice",
        input: { id, status: "paid" },
      });
      const value = parseValue(raw);
      listState.textContent = typeof value === "object" && value !== null && "value" in value
        ? `\u72B6\u6001\u53D8\u66F4\u56DE\u6267\uFF1A${text(value.value)}`
        : "\u72B6\u6001\u53D8\u66F4\u5B8C\u6210";
      await reloadInvoices();
    } catch (error) {
      listState.className = "error";
      listState.textContent = `\u72B6\u6001\u53D8\u66F4\u5931\u8D25\uFF1A${errorText(error)}`;
    }
  }
  document.querySelector("#reload")?.addEventListener("click", () => void reloadInvoices());
  document.querySelector("#create")?.addEventListener("click", () => void createInvoice());
  void reloadInvoices();
})();
