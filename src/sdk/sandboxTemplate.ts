// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Skill 沙箱 HTML 模板生成器
 *
 * 为每个 Skill 生成一个自包含的 HTML 页面，在 sandbox iframe 中运行。
 * 该页面包含：
 * 1. RPC 通信层（postMessage）— 双向：skill→host 和 host→skill
 * 2. ctx 对象（api, ui, store）
 * 3. Skill 组件代码占位
 * 4. 全局错误上报
 *
 * ⚠ 协议词汇不在本文件：生成的运行时是**不能 import 的那一侧**（内联在 `srcdoc` 里），
 * 所以协议表由 `SANDBOX_FRAME_PROTOCOL_TABLE` 序列化后注入，脚本内只允许引用 `AX_PROTOCOL.*`。
 * 这条约束由 `src/sdk/__tests__/sandboxFrameProtocol.test.ts` 机械校验（本文件出现任何协议
 * 字面量即红；渲染结果内嵌的表与协议表深相等即绿）。
 * 走 asset 协议装载的第三方 bundle 用的是另一份客户端运行时（`pluginClientBridge.ts`），
 * 它 import 同一张表 —— 两条投递路径、一份协议，不存在第二套桥。
 *
 * ⚠ 入向**身份判据**同样不在本文件写第二份：宿主侧用的是
 * `sandboxFrameProtocol.ts` 的 `isFrameFromSandboxSource(event, 本桥的 contentWindow)`，
 * 沙箱这一侧的期望来源是宿主文档 = `window.parent`。这段不能 import 的运行时靠
 * **原样内联该函数的实现**（`Function.prototype.toString()` 取的就是协议模块里那一条）
 * 来保证「两侧同一个谓词、同一套词汇」—— 任务 #72 的落点。判据本体（为什么 origin 在这里
 * 分不出「谁的帧」）只写在 `SANDBOX_MESSAGE_TARGET_ORIGIN` 与 `isFrameFromSandboxSource` 两处。
 */

import {
  isFrameFromSandboxSource,
  SANDBOX_FRAME_PROTOCOL_TABLE,
  SANDBOX_RPC_TIMEOUT_MS,
  SANDBOX_SRCDOC_RPC_OPS,
} from "./sandboxFrameProtocol";
import type { SkillPermissions } from "./types";

/** RPC 调用默认超时（毫秒）—— 取自帧协议单一来源，此处保留历史名供既有调用面使用。 */
export const DEFAULT_RPC_TIMEOUT_MS = SANDBOX_RPC_TIMEOUT_MS;

export interface SandboxTemplateOptions {
  skillName: string;
  skillId: string;
  props: Record<string, unknown>;
  permissions: SkillPermissions;
  htmlContent: string;
  entryScript?: string;
  devMode?: boolean;
  /** RPC 调用超时，默认 15000ms */
  rpcTimeoutMs?: number;
}

/**
 * 生成 Skill 沙箱 HTML 完整页面
 */
export function generateSandboxHtml(options: SandboxTemplateOptions): string {
  const {
    skillName,
    htmlContent,
    rpcTimeoutMs = DEFAULT_RPC_TIMEOUT_MS,
  } = options;

  const runtimeScript = generateRuntimeScript(rpcTimeoutMs);

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none';">
  <title>Skill: ${skillName}</title>
  <style>
    *, *::before, *::after {
      box-sizing: border-box;
      margin: 0;
      padding: 0;
    }
    html, body {
      width: 100%;
      height: 100%;
      overflow: auto;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      font-size: 14px;
      line-height: 1.5;
      color: inherit;
      background: transparent;
    }
    #app {
      width: 100%;
      min-height: 100%;
    }
    .ax-skill-error {
      padding: 24px;
      color: #e74c3c;
      text-align: center;
      font-family: monospace;
    }
    .ax-skill-loading {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 100%;
      color: #999;
    }
    .ax-skill-loading::after {
      content: "";
      width: 24px;
      height: 24px;
      border: 2px solid #eee;
      border-top-color: #1677ff;
      border-radius: 50%;
      animation: ax-spin 0.8s linear infinite;
      margin-left: 12px;
    }
    @keyframes ax-spin {
      to { transform: rotate(360deg); }
    }
  </style>
  <script>
    // 禁用危险 API（Object.defineProperty 比 delete 更可靠）
    Object.defineProperty(window, 'fetch', { value: undefined, configurable: false, writable: false });
    Object.defineProperty(window, 'XMLHttpRequest', { value: undefined, configurable: false, writable: false });
    // 禁止顶层导航（sandbox 属性已限制，此处作为额外防护）
    window.addEventListener("beforeunload", function(e) {
      e.preventDefault();
      e.returnValue = "";
    });
  </script>
</head>
<body>
  <div id="app">${htmlContent}</div>

  <script>
${runtimeScript}
  </script>
</body>
</html>`;
}

/**
 * 生成 Skill 沙箱运行时脚本。
 *
 * 提供：
 * - ctx 对象（api, ui, store）通过 postMessage RPC 实现
 * - 双向 RPC：skill→host 和 host→skill（callSkillMethod）
 * - 宿主消息监听（host:event, host:lifecycle, rpc:request）
 * - 全局错误上报
 *
 * ⚠ 这段字符串是「第三方插件作者看不到、也改不动」的那份客户端运行时（panel 粒度）。
 * 里面**一个协议字面量都不许出现**：帧类型、方法名、字段名全部来自注入的 `AX_PROTOCOL` 表，
 * 表来自 `sandboxFrameProtocol.ts`。要改协议，改那张表，然后 panel 与 page 两条通道一起变。
 */
function generateRuntimeScript(rpcTimeoutMs: number): string {
  // 注入整张协议表（编译期值，运行期只读）。JSON.stringify 保证与来源逐字一致，
  // 不需要在这里再写一遍任何一项。
  const protocolJson = JSON.stringify(SANDBOX_FRAME_PROTOCOL_TABLE);

  // ctx 方法面（任务 #78）：与协议表同一来源模块，作为**数据**注入并生成下面的 window.ctx。
  // 门 C 从产物里解析这份数据对账宿主 —— 不再依赖 `var M = AX_PROTOCOL.m` 的文本别名。
  const rpcOpsJson = JSON.stringify(SANDBOX_SRCDOC_RPC_OPS);

  // 入向身份判据：把协议模块里**那一条**函数的实现原样内联（任务 #72）。
  // 为什么用 toString 而不是在模板里再写一次 `event.source === window.parent`：
  // 写第二次就不是「同一个谓词」了 —— 宿主侧那条函数改了边界（例如再补一条「空期望一律拒」的
  // 变体），模板这一份不会跟着红，正是审查应修 4 要消灭的形态。
  // ⚠ 代价（已知并锁定）：该函数必须是**自包含**的（不引用模块作用域的常量），否则内联体在
  // 沙箱文档里 ReferenceError。这条由门 E 的行为用例兜住 —— 它真跑产物文本，一跑就红。
  const sourceGuardSource = isFrameFromSandboxSource.toString();

  return `
// ── AxAgent Skill Sandbox Runtime ──────────────────────────────────
(function() {
  "use strict";

  // 协议表（唯一来源：宿主 src/sdk/sandboxFrameProtocol.ts，生成本文档时注入）。
  // t = 帧类型 / m = RPC 方法名 / f = 帧本体字段 / a = 方法参数字段。
  var AX_PROTOCOL = ${protocolJson};
  var T = AX_PROTOCOL.t;
  var M = AX_PROTOCOL.m;
  var F = AX_PROTOCOL.f;
  var A = AX_PROTOCOL.a;

  // ctx 的 RPC 方法面（**数据形态**，唯一来源：宿主 src/sdk/sandboxFrameProtocol.ts 的
  // SANDBOX_SRCDOC_RPC_OPS）。下面的 api / ui / store 分组由这张表生成 ⇒
  // 「运行时调用了哪些方法」只有这一份清单，不再靠手写 M.xxx 调用点表达。
  // 门 C 从这段标记之间解析数据（提取不到即红），标记本身是产物与测试之间的稳定契约。
  /* AX_RPC_OPS_BEGIN */
  var AX_RPC_OPS = ${rpcOpsJson};
  /* AX_RPC_OPS_END */

  // 入向身份判据（唯一来源：宿主 src/sdk/sandboxFrameProtocol.ts 的 isFrameFromSandboxSource，
  // 生成本文档时整条函数原样内联）。沙箱这一侧的期望来源 = 宿主文档 = window.parent。
  // 与宿主接收侧共用同一条谓词，也共用它的边界语义：**期望来源为空 ⇒ 一帧都不认**。
  var AX_IS_FRAME_FROM_SOURCE = ${sourceGuardSource};
  var HOST_SOURCE_WINDOW = window.parent;

  /** 这一帧是不是宿主文档发来的（不是 ⇒ 不执行、不回包）。 */
  function axIsFromHost(event) {
    return AX_IS_FRAME_FROM_SOURCE(event, HOST_SOURCE_WINDOW);
  }

  var SKILL_NAME = ${JSON.stringify("__SKILL_NAME__")};
  var SKILL_ID = ${JSON.stringify("__SKILL_ID__")};
  var INITIAL_PROPS = __INITIAL_PROPS__;
  var PERMISSIONS = __PERMISSIONS__;
  // 超时是**按实例**传入的（generateSandboxHtml 的 rpcTimeoutMs，缺省取协议的 SANDBOX_RPC_TIMEOUT_MS），
  // 所以协议表里没有 timeoutMs —— 表里再放一份就是第二个答案。
  var RPC_TIMEOUT_MS = ${rpcTimeoutMs};

  // ── 注册的 RPC 方法（供宿主通过 callSkillMethod 调用） ──
  var registeredMethods = {};
  var themeChangeCallbacks = [];

  // ── RPC 基础设施 ──

  var pendingCalls = new Map();
  var TARGET_ORIGIN = window.location.origin;
  var callIdCounter = 0;

  function isValidOrigin(origin) {
    // ⚠ 这条**不是**身份判据：opaque 沙箱文档的 location.origin 恒为字符串 "null"，此处一律放行；
    // 而应用内任何 iframe 的 origin 也是同一个 "null" ⇒ origin 分不出「这帧是谁发的」。
    // 身份判据是上面那条 axIsFromHost（任务 #72：与宿主接收侧同一个谓词、同一套词汇）。
    // origin 只在文档非 opaque 时多留一层，纵深防御另有 sandbox 属性 + CSP + fetch/XHR 禁用。
    if (TARGET_ORIGIN === "null") {
      return true;
    }
    return origin === TARGET_ORIGIN;
  }

  /** 组装 { [字段名]: 值 } 的键值对表（生成的 ES5 风格代码里代替计算属性名）。 */
  function axObj(pairs) {
    var o = {};
    for (var i = 0; i < pairs.length; i++) { o[pairs[i][0]] = pairs[i][1]; }
    return o;
  }

  /** 组装一帧：判别字段 + 若干字段对。字段名一律取自协议表。 */
  function axFrame(typeValue, pairs) {
    var frame = axObj(pairs || []);
    frame[F.type] = typeValue;
    return frame;
  }

  /**
   * 失败判定：**唯一判据**是协议表的 failureErrorType（= 宿主 src/sdk/sandboxFrameProtocol.ts
   * 的 SANDBOX_RPC_FAILURE_ERROR_TYPE）。「error 是不是那个类型」而不是「error 真不真」——
   * 空串是失败，与宿主 callSkillMethod / bundle 客户端两通道同口径。
   * 这里不许写 typeof 的比较值本身，也不许退回收成「只看 error 真值」那种判定。
   * ⚠ 本注释在生成的模板字符串**内部**，所以整段不许出现反引号。
   */
  function axIsFailure(msg) {
    return typeof msg[F.error] === AX_PROTOCOL.failureErrorType;
  }

  /** 交给宿主。targetOrigin 用协议表的值：沙箱文档是 opaque origin，自身 origin 打不进宿主。 */
  function axSend(frame) {
    window.parent.postMessage(frame, AX_PROTOCOL.origin);
  }

  function callHost(method, args) {
    return new Promise(function(resolve, reject) {
      var callId = AX_PROTOCOL.callIdPrefix.sandbox + (++callIdCounter) + "_" + Date.now();
      var timer = setTimeout(function() {
        pendingCalls.delete(callId);
        reject(new Error('RPC call "' + method + '" timed out'));
      }, RPC_TIMEOUT_MS);

      var responseHandler = function(event) {
        // 身份先于内容（#72，与宿主接收侧同一条谓词）：应用内任何别的文档都能对本沙箱 window
        // 投一帧带猜得到的 callId 的 rpc:response，把在途 Promise 提前 settle 掉。
        if (!axIsFromHost(event)) { return; }
        if (!isValidOrigin(event.origin)) { return; }
        var msg = event.data;
        if (msg && msg[F.type] === T.response && msg[F.callId] === callId) {
          window.removeEventListener("message", responseHandler);
          var pending = pendingCalls.get(callId);
          if (!pending) { return; }
          clearTimeout(pending.timer);
          pendingCalls.delete(callId);
          if (axIsFailure(msg)) {
            pending.reject(new Error(msg[F.error]));
          } else {
            pending.resolve(msg[F.result]);
          }
        }
      };

      window.addEventListener("message", responseHandler);
      pendingCalls.set(callId, { resolve: resolve, reject: reject, timer: timer });

      try {
        axSend(axFrame(T.request, [[F.callId, callId], [F.method, method], [F.args, args]]));
      } catch (e) {
        window.removeEventListener("message", responseHandler);
        clearTimeout(timer);
        pendingCalls.delete(callId);
        reject(e);
      }
    });
  }

  function sendResponse(callId, result, error) {
    try {
      axSend(axFrame(T.response, [[F.callId, callId], [F.result, result], [F.error, error]]));
    } catch(e) { /* postMessage to parent may fail if iframe is detached */ }
  }

  // ── ctx 对象（api / ui / store 三个分组由 AX_RPC_OPS 生成，不手写调用点） ──

  /** 由一条 op 生成一个 RPC 方法：实参按 op.args 的顺序映射成参数字典。 */
  function axMakeRpc(op) {
    var methodName = M[op.method];
    var argKeys = op.args;
    var lastDefault = op.lastDefault;
    return function() {
      // 无参方法（ui.getTheme / ui.getLocale）不带 args —— 与协议构造器同口径。
      if (argKeys.length === 0) {
        return callHost(methodName);
      }
      var pairs = [];
      for (var i = 0; i < argKeys.length; i++) {
        var value = arguments[i];
        if (i === argKeys.length - 1 && lastDefault !== undefined && !value) {
          value = lastDefault;
        }
        pairs.push([A[argKeys[i]], value]);
      }
      return callHost(methodName, axObj(pairs));
    };
  }

  /** 生成一个 ctx 分组；是否冻结由调用方决定（ui 还要追加本地回调 onThemeChange）。 */
  function axRpcGroup(groupName) {
    var group = {};
    for (var i = 0; i < AX_RPC_OPS.length; i++) {
      var op = AX_RPC_OPS[i];
      if (op.group === groupName) {
        group[op.name] = axMakeRpc(op);
      }
    }
    return group;
  }

  // onThemeChange 不是 RPC 方法（不发帧），故不在 AX_RPC_OPS 里，单独追加。
  var uiGroup = axRpcGroup("ui");
  uiGroup.onThemeChange = function(callback) {
    if (typeof callback === "function") {
      themeChangeCallbacks.push(callback);
    }
  };

  window.ctx = Object.freeze({
    get skillName() { return SKILL_NAME; },
    get skillId()   { return SKILL_ID; },
    get props()     { return INITIAL_PROPS; },

    api: Object.freeze(axRpcGroup("api")),
    ui: Object.freeze(uiGroup),
    store: Object.freeze(axRpcGroup("store"))
  });

  // ── 供 Skill 注册 RPC 方法的 API ──
  window.registerSkillMethod = function(name, fn) {
    if (typeof fn !== "function") {
      throw new Error("registerSkillMethod: second argument must be a function");
    }
    registeredMethods[name] = fn;
  };

  // ── 宿主消息监听 ──

  window.addEventListener("message", function(event) {
    // 身份先于内容（#72）：来源不是宿主文档 ⇒ 事件帧 / 生命周期帧 / 请求帧一律不执行、不回包。
    if (!axIsFromHost(event)) { return; }
    if (!isValidOrigin(event.origin)) { return; }
    var msg = event.data;
    if (!msg || typeof msg[F.type] !== "string") { return; }
    var frameType = msg[F.type];

    if (frameType === T.hostEvent) {
      var eventName = msg[F.event];
      var eventPayload = msg[F.payload];
      if (window.dispatchEvent) {
        window.dispatchEvent(new CustomEvent("ax:" + eventName, { detail: eventPayload }));
      }
      if (eventName === "theme-change" && eventPayload && eventPayload.theme) {
        document.documentElement.setAttribute("data-theme", eventPayload.theme);
        for (var ti = 0; ti < themeChangeCallbacks.length; ti++) {
          try { themeChangeCallbacks[ti](eventPayload.theme); } catch(e) { try { axSend(axFrame(T.contentError, [[F.error, "Theme callback error: " + String(e)]])); } catch(e2) {} }
        }
      }
    } else if (frameType === T.hostLifecycle) {
      if (msg[F.phase] === "mount" && typeof window.onSkillMount === "function") {
        window.onSkillMount(window.ctx, msg[F.props] || {});
      } else if (msg[F.phase] === "unmount" && typeof window.onSkillUnmount === "function") {
        window.onSkillUnmount();
      }
    } else if (frameType === T.request) {
      // 处理宿主对 skill 的 RPC 调用（callSkillMethod）
      var method = registeredMethods[msg[F.method]];
      var reqCallId = msg[F.callId];
      if (!method) {
        sendResponse(reqCallId, undefined, "Unknown method: " + msg[F.method]);
        return;
      }
      try {
        var result = method(msg[F.args] || {});
        if (result && typeof result.then === "function") {
          result.then(
            function(v) { sendResponse(reqCallId, v); },
            function(e) { sendResponse(reqCallId, undefined, String(e)); }
          );
        } else {
          sendResponse(reqCallId, result);
        }
      } catch (e) {
        sendResponse(reqCallId, undefined, String(e));
      }
    }
  });

  // ── 全局错误上报 ──
  window.addEventListener("error", function(event) {
    try {
      axSend(axFrame(T.contentError, [
        [F.error, event.message || "Unhandled error"],
        [F.source, event.filename],
        [F.line, event.lineno],
        [F.col, event.colno]
      ]));
    } catch(e) { /* error reporting postMessage failed, nothing we can do */ }
  });

  window.addEventListener("unhandledrejection", function(event) {
    try {
      axSend(axFrame(T.contentError, [[F.error, "Unhandled rejection: " + String(event.reason)]]));
    } catch(e) { /* error reporting postMessage failed, nothing we can do */ }
  });

  // ── 向宿主报告就绪 ──
  try {
    axSend(axFrame(T.ready));
  } catch(e) { /* skill:ready postMessage failed */ }

  callHost(M.uiGetTheme).then(function(theme) {
    if (theme) {
      document.documentElement.setAttribute("data-theme", theme);
    }
  });

  if (typeof window.onSkillInit === "function") {
    try {
      window.onSkillInit(window.ctx);
    } catch(e) {
      try {
        axSend(axFrame(T.contentError, [[F.error, String(e)]]));
      } catch(e2) { /* nested error report postMessage failed */ }
    }
  }
})();
`;
}
