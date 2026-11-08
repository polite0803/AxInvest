// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 沙箱 postMessage 帧协议 —— **唯一词汇表**（任务 #46 第 2 项的载体）。
 *
 * 为什么需要这个模块（不是「再抽一层工具函数」）：
 * 宿主与沙箱内容之间那条 postMessage 协议，今天在仓里有**三份手写副本** ——
 * ① 宿主侧 `src/sdk/rpcBridge.ts` 的 handler 表键名 + 发送/解析里的字段名；
 * ② `srcdoc` 客户端运行时 `src/sdk/sandboxTemplate.ts` 生成的那段 JS；
 * ③ 类型层 `src/sdk/types.ts` 的 `HostToSkillMessage` / `SkillToHostMessage` 判别联合。
 * 三份各自写字面串，任一侧改名另一侧不会红 —— 这正是 `src/__tests__/ci.workflow.test.ts`
 * 「两侧各写一份就会漂 ⇒ 自动提取做对称差集」要消灭的形态，只是这里的两侧是**运行时**而不止文本。
 *
 * 更要命的是本批要补的那条通道：page（asset 协议装插件自带 bundle）粒度下，宿主不再包 `srcdoc`，
 * 第三方 bundle 里没有②，于是「第三份」会由插件作者自己照抄 —— 抄错一个字就是静默失效的桥。
 *
 * 收敛办法（按能否 import 分两类，而不是靠约定）：
 * - **能 import 的一侧**（宿主 `rpcBridge.ts`、ESM 客户端 `pluginClientBridge.ts`）：
 *   字面量一律从本模块取值，构造/解析帧一律走本模块的 builder / guard。
 * - **不能 import 的一侧**（`sandboxTemplate.ts` 生成的内联运行时）：
 *   把 {@link SANDBOX_FRAME_PROTOCOL_TABLE} 整张表注入生成的脚本，脚本内只允许引用
 *   `AX_PROTOCOL.*`，不得出现任何协议字面量。
 * 两类都由 `src/sdk/__tests__/sandboxFrameProtocol.test.ts` 机械校验：
 * 协议字面量**只允许出现在本模块与 `pluginBridgeProtocol.ts`**；生成的脚本内嵌的表必须与
 * 本模块的表深相等；宿主接受的集合与客户端调用的集合做对称差集必须为空。
 *
 * 刻意**不**做的事：不改这条协议的形状（字段名、错误承载方式、超时语义）。
 * 本批是「把已有形状收成一份来源」，不是「顺手升级协议」—— 后者会让 panel 粒度的既有
 * `srcdoc` 面板与已分发的第三方 bundle 同时失配，且需要 Rust 侧配合（本批禁止动 Rust）。
 *
 * @module sdk/sandboxFrameProtocol
 */

// ── 帧类型（判别字段 `type` 的全部取值） ─────────────────────────────

/**
 * 沙箱与宿主之间流动的帧类型。
 *
 * ⚠ `skill:ready` / `skill:error` 前缀是历史命名（沙箱机制最早只服务 Skill），
 * 今天 panel 与 page 两种插件粒度同样发这两帧。改名会同时打断已分发的 `srcdoc` 面板与
 * 第三方 bundle，故此处**按原值收敛**，不借收敛之机改名。
 */
export const SANDBOX_FRAME_TYPES = {
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
} as const;

/** 帧类型取值联合。 */
export type SandboxFrameType = (typeof SANDBOX_FRAME_TYPES)[keyof typeof SANDBOX_FRAME_TYPES];

// ── RPC 方法名（沙箱 → 宿主的调用面） ────────────────────────────────

/**
 * 宿主侧 handler 表的键，也是沙箱侧 `callHost(method, args)` 的实参。
 *
 * 这一组必须与宿主 `createHostApiBridge` 实际接受的集合逐字相同 ——
 * 宿主多一个键 = 沙箱调不到的能力；沙箱多一个调用点 = 运行时收到 `Unknown method`。
 * 两种情况今天都**静默**（无编译期关联），故由测试的对称差集兜住（见模块头）。
 */
export const SANDBOX_RPC_METHODS = {
  apiInvoke: "api.invoke",
  apiEmit: "api.emit",
  uiNavigate: "ui.navigate",
  uiNotify: "ui.notify",
  uiGetTheme: "ui.getTheme",
  uiGetLocale: "ui.getLocale",
  storeRead: "store.read",
  storeWrite: "store.write",
} as const;

/** RPC 方法名联合（宿主 handler 表的键类型）。 */
export type SandboxRpcMethod = (typeof SANDBOX_RPC_METHODS)[keyof typeof SANDBOX_RPC_METHODS];

// ── 字段名 ──────────────────────────────────────────────────────────

/**
 * 帧本体的字段名。两侧构造与解析都从这里取值，字段改名即单点改名。
 *
 * ⚠ 帧本体的字段与 IPC 语义绑定（`callId` 是两侧的关联键），不属于业务参数；
 * 业务参数（如 `plugin_ui_action` 的 `action`）归 `pluginBridgeProtocol.ts` 与后端 DTO，
 * 不放这里，避免把「协议」与「业务」混成一张表。
 */
export const SANDBOX_FRAME_FIELDS = {
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
} as const;

/**
 * `api.*` / `ui.*` / `store.*` 这些方法的**参数帧**字段名。
 *
 * 宿主 handler 逐个按这些键读参数（`args?.command` 等），沙箱按同样的键构造。
 * 单独成表（不与帧本体混）的理由：本体字段由 `Sandbox*Frame` 接口钉形状，
 * 参数字段没有接口保护，是漂移概率最高的一层。
 */
export const SANDBOX_RPC_ARG_KEYS = {
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
} as const;

// ── srcdoc 运行时的 ctx 方法面（数据形态；门 C 的提取源） ────────────

/**
 * `srcdoc` 运行时挂到 `window.ctx` 上的 **RPC 方法面**：一行 = 一个 ctx 方法。
 *
 * 为什么是数据（任务 #78）：门 C 原先从产物文本里扫 `M.<key>` 推断「srcdoc 用了哪些 RPC 方法」，
 * 依赖 `var M = AX_PROTOCOL.m` 这个别名写法 —— 改个变量名提取就失效（虽被分母断言拦成红，
 * 但那是文本耦合而不是结构）。改成这张表之后：
 * - `sandboxTemplate.ts` 把本表整份序列化进产物（`AX_RPC_OPS` 标记块），
 *   **并且 `window.ctx` 的 api/ui/store 分组由它生成** ⇒ 表就是运行时的行为，不是旁挂的说明；
 * - 门 C 从产物里解析这张表（提取不到 / 条数不等于协议方法表长度即红），不再读别名文本。
 *
 * 三个字段各自的身份：
 * - `method` 是 {@link SANDBOX_RPC_METHODS} 的**键**（不是值）—— 运行时经 `M[key]` 取值，
 *   方法名仍只有协议表一处定义；键改名由 `satisfies` 在编译期拦。
 * - `args` 是 {@link SANDBOX_RPC_ARG_KEYS} 的**键**，按位置映射实参；
 *   字段值仍只有协议表一处定义（运行时经 `A[key]` 取）。
 * - `lastDefault` 是 ctx 这一层的 UI 缺省值（仅 `ui.notify` 的 `type`），不是协议值。
 *
 * ⚠ 为什么放本模块而不放 `pluginBridgeProtocol.ts`：本表描述的是 **srcdoc 沙箱运行时**的方法面，
 * 与插件桥白名单无关；且 `pluginBridgeProtocol.ts` 已经 import 本模块（{@link SANDBOX_RPC_METHODS}），
 * 放过去会反向依赖成环。协议值仍然只有本模块与 `pluginBridgeProtocol.ts` 两处定义。
 */
export interface SandboxCtxRpcOp {
  /** ctx 上的分组名。 */
  readonly group: "api" | "ui" | "store";
  /** ctx 分组上的方法名。 */
  readonly name: string;
  /** 协议方法表的键（运行时经 `AX_PROTOCOL.m[key]` 取方法名）。 */
  readonly method: keyof typeof SANDBOX_RPC_METHODS;
  /** 参数键表的键，按位置对应实参（运行时经 `AX_PROTOCOL.a[key]` 取字段名）。 */
  readonly args: readonly (keyof typeof SANDBOX_RPC_ARG_KEYS)[];
  /** 末位实参的缺省值（仅 `ui.notify` 的 `type` 用）。 */
  readonly lastDefault?: string;
}

/**
 * srcdoc 运行时的 ctx 方法面（顺序只影响 `window.ctx` 属性的定义顺序，无业务语义）。
 *
 * 这张表是「运行时调用了哪些 RPC 方法」的**唯一**清单：模板据此生成 ctx，门 C 据此对账宿主。
 * 新增一个 ctx 方法 = 在这里加一行（并确认宿主 handler 表接得住，门 C 会红）。
 */
export const SANDBOX_SRCDOC_RPC_OPS = [
  { group: "api", name: "invoke", method: "apiInvoke", args: ["command", "args"] },
  { group: "api", name: "emit", method: "apiEmit", args: ["event", "payload"] },
  { group: "ui", name: "navigate", method: "uiNavigate", args: ["path"] },
  {
    group: "ui",
    name: "notify",
    method: "uiNotify",
    args: ["message", "type"],
    lastDefault: "info",
  },
  { group: "ui", name: "getTheme", method: "uiGetTheme", args: [] },
  { group: "ui", name: "getLocale", method: "uiGetLocale", args: [] },
  // ⚠ `ui.onThemeChange` 刻意不在表里：它是本地回调注册，不发帧，不是 RPC 方法面。
  { group: "store", name: "read", method: "storeRead", args: ["storeName", "selector"] },
  {
    group: "store",
    name: "write",
    method: "storeWrite",
    args: ["storeName", "value", "selector"],
  },
] as const satisfies readonly SandboxCtxRpcOp[];

// ── 超时与 origin ───────────────────────────────────────────────────

/**
 * RPC 往返超时。此前宿主 `RPC_TIMEOUT_MS` 与模板 `DEFAULT_RPC_TIMEOUT_MS` 各写一份 `15000`；
 * 数值巧合相等不代表语义同源，收成一条。
 */
export const SANDBOX_RPC_TIMEOUT_MS = 15000;

/**
 * 「这一帧是不是失败响应」的判据取值：`typeof error === "string"` 即失败。
 *
 * 为什么这是一个**值**而不只是一段写在两侧的 `if`（审查应修 2）：失败与成功共用响应帧，
 * 靠 `error` 承载区分。此前宿主/客户端按「键在不在」（`error !== undefined`）、
 * `srcdoc` 生成的运行时按真值（`if (msg[F.error])`）—— handler 抛空串、后端回 `Err("")`
 * 时同一次失败一边 reject、一边 `resolve(undefined)`。判据收成一条之后，
 * **空串是失败**（它是字符串），`undefined` / 缺键是成功。
 *
 * 能 import 的一侧走 {@link isFailureFrame}；不能 import 的 `srcdoc` 运行时从注入表的
 * `failureErrorType` 取同一个值自己写 `typeof` 比较 —— 语义仍只有这一处定义。
 */
export const SANDBOX_RPC_FAILURE_ERROR_TYPE = "string";

/**
 * **出向投递**用的 `postMessage` targetOrigin，两个方向恒为同一个 `"*"`。
 *
 * ⚠ 本常量是这条判据的**唯一定义处**（任务 #46 审查应修 1/4 的收口口径），其它文件只引用、不再复述理由。
 *
 * 「出向投递的 targetOrigin」与「入向帧的来源校验」是**两个概念**，不许再混用同一个值：
 *
 * 1. **出向 targetOrigin（就是本常量）**：接收方恒是 opaque origin 的沙箱文档
 *   （`sandbox="allow-scripts"` 无 `allow-same-origin`；`srcdoc` 面板与 asset 协议装的第三方
 *   bundle 两种装法都是）。宿主自己的 origin 在这里**打不进对方** —— Chromium（与 WebView2 同引擎族）
 *   实测 `contentWindow.postMessage(frame, 宿主 origin)` 沙箱收不到、`postMessage(frame, "*")` 收到；
 *   `tauri://localhost`（macOS/Linux）命中前者，Windows/浏览器模式落到 `"*"` 分支只是侥幸。
 *   窄值也留不住：opaque 文档的 `location.origin` 是字符串 `"null"`，而 `"null"` 当 targetOrigin
 *   会**同步抛 SyntaxError**（不是静默丢）。⇒ `"*"` 是唯一对两种 origin 都成立且不静默丢帧的取值，
 *   panel 与 page 因此**共用一组词汇**，不存在「一条通道宽、另一条窄」。
 *   泄露面：出向帧只落到我们自己创建的那块 iframe 文档，载荷是宿主下发的响应/事件与沙箱自己的请求。
 * 2. **入向来源校验（不看 origin，看 `event.source`）**：opaque 文档送进来的 `event.origin` 恒是
 *   `"null"`，与任何别的沙箱文档无法区分 ⇒ origin 不能当身份。判据是「这一帧的 `event.source`
 *   是不是本桥对接的那个 `iframe.contentWindow`」，见 {@link isFrameFromSandboxSource}。
 *
 * 因此本模块对「宿主侧的守卫靠什么」的如实描述是：**来源比对（`event.source`）+ 方法表 +
 * 命令白名单 + 宿主绑定的 pluginId** 四层，第一层缺了后三层就形同虚设；校验点与保证者写在
 * {@link isFrameFromSandboxSource} 的注释里。
 */
export const SANDBOX_MESSAGE_TARGET_ORIGIN = "*";

/** callId 前缀：区分发起方，宿主侧用 `host_`、沙箱侧用 `skill_`/`plugin_`。 */
export const SANDBOX_CALL_ID_PREFIX = {
  /** 宿主向沙箱发起调用（`callSkillMethod`） */
  host: "host_",
  /** `srcdoc` 沙箱内容（Skill 与插件 panel）向宿主发起 RPC */
  sandbox: "skill_",
  /** 走 asset 协议装载的插件 bundle（page 粒度）向宿主发起 RPC */
  plugin: "plugin_",
} as const;

// ── 注入给「不能 import 的一侧」的协议表 ────────────────────────────

/**
 * 整张协议表：给 `sandboxTemplate.ts` 序列化后注入生成的沙箱运行时。
 *
 * ⚠ 表里只放本模块的值。新增协议项先进本模块，再决定是否入表 ——
 * 入表即等于「srcdoc 客户端也认这项」。
 *
 * ⚠ **表里不放超时**（审查稿第 12 条的裁定）：运行时真正用的是 `generateSandboxHtml`
 * 按实例传入的 `rpcTimeoutMs`（缺省 `SANDBOX_RPC_TIMEOUT_MS`）。此前表里另写一份
 * `timeoutMs` 而无人读，等于给「谁超时」留了第二个答案；删掉它，协议值 `15000` 仍只有
 * {@link SANDBOX_RPC_TIMEOUT_MS} 一处定义（宿主 `callSkillMethod` 与 bundle 客户端都 import 它）。
 */
export const SANDBOX_FRAME_PROTOCOL_TABLE = {
  t: SANDBOX_FRAME_TYPES,
  m: SANDBOX_RPC_METHODS,
  f: SANDBOX_FRAME_FIELDS,
  a: SANDBOX_RPC_ARG_KEYS,
  failureErrorType: SANDBOX_RPC_FAILURE_ERROR_TYPE,
  origin: SANDBOX_MESSAGE_TARGET_ORIGIN,
  callIdPrefix: SANDBOX_CALL_ID_PREFIX,
} as const;

// ── 帧接口（单一来源的「形状」） ────────────────────────────────────

/** 沙箱 → 宿主：RPC 请求。 */
export interface SandboxRpcRequestFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.request;
  readonly callId: string;
  readonly method: string;
  readonly args?: Record<string, unknown>;
}

/** 宿主 → 沙箱：RPC 响应。失败与成功共用这一帧，靠 `error` 有无区分。 */
export interface SandboxRpcResponseFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.response;
  readonly callId: string;
  readonly result?: unknown;
  readonly error?: string;
}

/** 沙箱 → 宿主：内容就绪。 */
export interface SandboxReadyFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.ready;
}

/** 沙箱 → 宿主：内容内部错误（`source` / `line` / `col` 为可选定位信息）。 */
export interface SandboxContentErrorFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.contentError;
  readonly error: string;
  readonly source?: string;
  readonly line?: number;
  readonly col?: number;
}

/** 宿主 → 沙箱：事件推送。 */
export interface SandboxHostEventFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.hostEvent;
  readonly event: string;
  readonly payload?: unknown;
}

/** 宿主 → 沙箱：生命周期通知。 */
export interface SandboxHostLifecycleFrame {
  readonly type: typeof SANDBOX_FRAME_TYPES.hostLifecycle;
  readonly phase: "mount" | "unmount";
  readonly props?: Record<string, unknown>;
}

/** 宿主 → 沙箱方向的全部帧（历史名 `HostToSkillMessage`，在 `sdk/types.ts` 保持同名转出）。 */
export type SandboxFrameToContent =
  | SandboxRpcResponseFrame
  | SandboxHostEventFrame
  | SandboxHostLifecycleFrame;

/** 沙箱 → 宿主方向的全部帧（历史名 `SkillToHostMessage`）。 */
export type SandboxToHostFrame =
  | SandboxRpcRequestFrame
  | SandboxReadyFrame
  | SandboxContentErrorFrame;

// ── 构造与解析（两侧都只走这几个函数） ──────────────────────────────

/** 失败载荷的承载形状：**字符串**。宿主把任何异常收成 `String(e)` 后放进 `error` 字段。 */
export function rpcErrorText(error: unknown): string {
  return String(error);
}

/** 构造 RPC 请求帧。`args` 为 `undefined` 时不带该字段（与既有宿主/模板行为一致）。 */
export function buildRpcRequestFrame(
  callId: string,
  method: string,
  args?: Record<string, unknown>,
): SandboxRpcRequestFrame {
  const f = SANDBOX_FRAME_FIELDS;
  const frame: Record<string, unknown> = {
    [f.type]: SANDBOX_FRAME_TYPES.request,
    [f.callId]: callId,
    [f.method]: method,
  };
  if (args !== undefined) {
    frame[f.args] = args;
  }
  return frame as unknown as SandboxRpcRequestFrame;
}

/** 构造成功响应帧。 */
export function buildRpcResultFrame(callId: string, result?: unknown): SandboxRpcResponseFrame {
  return {
    [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.response,
    [SANDBOX_FRAME_FIELDS.callId]: callId,
    [SANDBOX_FRAME_FIELDS.result]: result,
  } as unknown as SandboxRpcResponseFrame;
}

/** 构造失败响应帧（错误形状见 {@link rpcErrorText}）。 */
export function buildRpcErrorFrame(callId: string, error: unknown): SandboxRpcResponseFrame {
  return {
    [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.response,
    [SANDBOX_FRAME_FIELDS.callId]: callId,
    [SANDBOX_FRAME_FIELDS.error]: rpcErrorText(error),
  } as unknown as SandboxRpcResponseFrame;
}

/** 构造宿主事件帧。 */
export function buildHostEventFrame(event: string, payload?: unknown): SandboxHostEventFrame {
  return {
    [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.hostEvent,
    [SANDBOX_FRAME_FIELDS.event]: event,
    [SANDBOX_FRAME_FIELDS.payload]: payload,
  } as unknown as SandboxHostEventFrame;
}

/** 构造宿主生命周期帧。 */
export function buildHostLifecycleFrame(
  phase: "mount" | "unmount",
  props?: Record<string, unknown>,
): SandboxHostLifecycleFrame {
  return {
    [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.hostLifecycle,
    [SANDBOX_FRAME_FIELDS.phase]: phase,
    [SANDBOX_FRAME_FIELDS.props]: props,
  } as unknown as SandboxHostLifecycleFrame;
}

/** 构造内容就绪帧。 */
export function buildSandboxReadyFrame(): SandboxReadyFrame {
  return { [SANDBOX_FRAME_FIELDS.type]: SANDBOX_FRAME_TYPES.ready } as unknown as SandboxReadyFrame;
}

/** 构造内容错误帧（定位信息可选，缺省不带）。 */
export function buildSandboxContentErrorFrame(
  error: unknown,
  location?: { source?: string; line?: number; col?: number },
): SandboxContentErrorFrame {
  const f = SANDBOX_FRAME_FIELDS;
  const frame: Record<string, unknown> = {
    [f.type]: SANDBOX_FRAME_TYPES.contentError,
    [f.error]: rpcErrorText(error),
  };
  if (location?.source !== undefined) {
    frame[f.source] = location.source;
  }
  if (location?.line !== undefined) {
    frame[f.line] = location.line;
  }
  if (location?.col !== undefined) {
    frame[f.col] = location.col;
  }
  return frame as unknown as SandboxContentErrorFrame;
}

/**
 * 组装 callId：`<前缀><自增序号>_<毫秒时间戳>`。
 *
 * 宿主与两种沙箱客户端历史上各写了一份同样形状的模板串；帧里它是**不透明关联键**
 * （宿主只把它原样回带，从不解析内部结构，见 `pluginPageBridge.test.ts` 的陌生 callId 断言），
 * 所以这里统一只为「两侧格式同一」，不承担语义。
 */
export function formatCallId(prefix: string, sequence: number): string {
  return `${prefix}${sequence}_${Date.now()}`;
}

/** 取出任意对象上的帧类型判别值（非对象 / 无该字段一律 `undefined`，供 guard 复用）。 */
function frameTypeOf(data: unknown): string | undefined {
  if (!data || typeof data !== "object") {
    return undefined;
  }
  const value = (data as Record<string, unknown>)[SANDBOX_FRAME_FIELDS.type];
  return typeof value === "string" ? value : undefined;
}

/** 是否 RPC 请求帧（宿主接收侧唯一的类型收窄入口）。 */
export function isRpcRequestFrame(data: unknown): data is SandboxRpcRequestFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.request;
}

/** 是否 RPC 响应帧（沙箱接收侧唯一的类型收窄入口）。 */
export function isRpcResponseFrame(data: unknown): data is SandboxRpcResponseFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.response;
}

/** 是否内容就绪帧。 */
export function isSandboxReadyFrame(data: unknown): data is SandboxReadyFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.ready;
}

/** 是否内容错误帧。 */
export function isSandboxContentErrorFrame(data: unknown): data is SandboxContentErrorFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.contentError;
}

/** 是否宿主事件帧。 */
export function isHostEventFrame(data: unknown): data is SandboxHostEventFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.hostEvent;
}

/** 是否宿主生命周期帧。 */
export function isHostLifecycleFrame(data: unknown): data is SandboxHostLifecycleFrame {
  return frameTypeOf(data) === SANDBOX_FRAME_TYPES.hostLifecycle;
}

/**
 * 这一帧是不是**失败**响应 —— 成功/失败判定的唯一函数（审查应修 2）。
 *
 * 判据是「`error` 是不是字符串」，不是真值：空串 `""` 是失败（handler 抛空串、后端
 * `Err(String)` 空消息都会走到这里），`undefined` / 不带该键才是成功。写成类型守卫是为了
 * 让消费方拿到 `frame.error` 时仍是 `string`，不必再补一个「不可能出现」的兜底分支。
 *
 * 三处消费（宿主 `callSkillMethod`、bundle 客户端 `handleMessage`、`srcdoc` 生成的运行时）
 * 只许用这一条；`srcdoc` 侧不能 import，故从注入表的 `failureErrorType` 取同一个值。
 */
export function isFailureFrame(frame: SandboxRpcResponseFrame): frame is SandboxRpcResponseFrame & {
  readonly error: string;
} {
  const raw = (frame as unknown as Record<string, unknown>)[SANDBOX_FRAME_FIELDS.error];
  return typeof raw === SANDBOX_RPC_FAILURE_ERROR_TYPE;
}

/**
 * 读取响应帧的错误文本：`undefined` 表示这是一次成功响应。
 *
 * 形状读取用这一条，**判定**用 {@link isFailureFrame} —— 两者同源于
 * {@link SANDBOX_RPC_FAILURE_ERROR_TYPE}，「空串算不算失败」这类边界只许有一处定义。
 */
export function readRpcResponseError(frame: SandboxRpcResponseFrame): string | undefined {
  return isFailureFrame(frame) ? frame.error : undefined;
}

/** 读取响应帧的 `callId`（无有效值返回 `undefined`，调用方据此丢弃陌生帧）。 */
export function readRpcResponseCallId(frame: SandboxRpcResponseFrame): string | undefined {
  const raw = (frame as unknown as Record<string, unknown>)[SANDBOX_FRAME_FIELDS.callId];
  return typeof raw === "string" ? raw : undefined;
}

/** 读取请求帧的参数字典（缺省空对象；不返回 `undefined`，省掉调用点的空值分支）。 */
export function readRpcRequestArgs(frame: SandboxRpcRequestFrame): Record<string, unknown> {
  const raw = (frame as unknown as Record<string, unknown>)[SANDBOX_FRAME_FIELDS.args];
  return raw && typeof raw === "object" ? raw as Record<string, unknown> : {};
}

// ── 入向帧的来源判定（宿主接收侧唯一的身份判据） ────────────────────

/**
 * 宿主/沙箱接收侧用到的 `message` 事件最小面。
 *
 * 刻意不写成 DOM 的 `MessageEvent`：真实事件结构上就满足这两个字段，而窄声明让测试可以直接
 * 喂对象、也让本模块不必依赖 DOM 类型（bundle 侧那三件套要进第三方构建图）。
 */
export interface SandboxInboundMessageEvent {
  readonly data: unknown;
  readonly source: unknown;
}

/**
 * 这一帧是不是**本桥对接的那块沙箱**发来的 —— 宿主接收侧唯一的身份判据（审查应修 4）。
 *
 * 为什么只能判 `source`（判据本体见 {@link SANDBOX_MESSAGE_TARGET_ORIGIN}，此处不复述）：
 * 沙箱文档是 opaque origin，`event.origin` 恒为 `"null"`，任何 iframe 都一样 ⇒ origin 分不出
 * 「谁的帧」。`event.source` 是发送方文档的 WindowProxy，与 `iframe.contentWindow` **同一对象**，
 * 是这里唯一可用、且不依赖 origin 的强判据。
 *
 * 缺了它会发生什么（本仓的真实形态，不是假想）：应用内任一可脚本化的 iframe
 * （HtmlPreview / ChartPreview / 已导航到远端的 iframe）都能对宿主 window 伪造
 * `{type:"rpc:request", method:"api.invoke", args:{command:...}}`，借**当前挂载 skill 的权限面**
 * 执行 —— 命令白名单 + pluginId 绑定只把危害限制在「本包两条命令」，skill 的权限面是
 * store / ui / api，宽得多。
 *
 * 校验点在哪、由谁保证（认符号不认号；行号会随编辑漂，函数名是锚）：
 * - `src/sdk/rpcBridge.ts` 的 `createHostApiBridge().handleRpcRequest` —— 入参是**整个事件**
 *   而非裸帧，就是为了让这一判据长在宿主自己身上，不靠调用方自觉；不符即**丢弃且不回包**；
 * - `src/components/shared/SkillSandboxContainer.tsx` 的 `messageHandler`（ready / contentError）
 *   与 `handleRpc` —— 容器在把事件交给桥之前先判一次（ready 帧会创建桥、撤加载超时，
 *   这一步本身不能被外来帧触发）；
 * - `src/sdk/rpcBridge.ts` 的 `createHostRpcBridge`：`handleMessage` 与 `callSkillMethod`
 *   的响应监听（伪造响应帧会串改宿主→沙箱方向的在途 Promise）。
 * 证据用例：`src/components/plugin/__tests__/pluginPageBridge.test.ts` 的
 * 「伪造 source 的帧：宿主不调 IPC、不回包」。
 */
export function isFrameFromSandboxSource(event: SandboxInboundMessageEvent, sandboxWindow: unknown): boolean {
  // 期望来源为空（iframe 尚未挂载 / 已脱离文档）时**一帧都不认**：不写这条，
  // 「两侧都是 null」会把 null === null 判成真，等于沙箱没挂上却放行了外来帧。
  if (sandboxWindow === null || sandboxWindow === undefined) {
    return false;
  }
  return event.source === sandboxWindow;
}

// ── 沙箱装载超时（panel 与 page 两条粒度共用） ──────────────────────

/**
 * 宿主等「沙箱上报 ready」的时间上限：`srcdoc` 面板与 asset 页面**同一条**。
 *
 * 为什么收成单点（与 {@link SANDBOX_RPC_TIMEOUT_MS} 同一个理由）：这条超时决定的是
 * 「内容没就绪时用户看到什么」，两种粒度必须是同一种症状；写两份就会漂成
 * 「面板 30 s 报错、页面 60 s 才报错」。权威消费方：
 * - `src/components/shared/SkillSandboxContainer.tsx`（panel / skill 的 srcdoc 通道）
 * - `src/components/plugin/PluginPageHost.tsx`（page 的 asset 通道）
 *
 * ⚠ 刻意**不进** {@link SANDBOX_FRAME_PROTOCOL_TABLE}：这是宿主自己的计时，沙箱侧读了它
 * 反而会长出第二个「谁先放弃」的答案（与 `timeoutMs` 不入表是同一条裁定，见那张表的注释）。
 */
export const SANDBOX_LOAD_TIMEOUT_MS = 30000;
