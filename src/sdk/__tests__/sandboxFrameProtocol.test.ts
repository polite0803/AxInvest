// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 沙箱帧协议的**单一来源对账门**（任务 #46 验收条 2/3）。
 *
 * 手法沿用 `src/__tests__/ci.workflow.test.ts` 的「两侧各写一份就会漂 ⇒ 自动提取做对称差集」，
 * 但这里两侧是**运行时**，所以提取的是「谁写了协议字面量」与「谁实际接受/引用哪些方法」：
 *
 * - **门 A（字面量独占）**：协议词汇（帧类型串、RPC 方法名、两条命令名）**加三面检测**
 *   —— ① 帧词汇串；② 帧字段名/参数键名**当字面量键写死**（`obj["callId"]`、`{"callId": …}`）；
 *   ③ 把 `"*"` **写在 `postMessage` 的 targetOrigin 位**。只允许出现在 `sandboxFrameProtocol.ts`
 *   与 `pluginBridgeProtocol.ts` 里。任何一个消费方（宿主 `rpcBridge.ts`、`srcdoc` 模板、
 *   bundle 客户端、面板宿主）自己写回一份字面串 ⇒ 该文件立刻红。这就是植入实验的落点：
 *   改一侧不改另一侧**必然**命中这一条。检测面本身另有「植入必命中 / 正确形态不误伤」自证用例
 *   —— 只扫真实文件会有一种静默绿：正则写偏 + 消费方恰好没写。
 * - **门 B（注入表同源）**：`srcdoc` 运行时不能 import，故协议表被序列化进生成的 HTML；
 *   这里把渲染结果里的表**解析回来**与来源深相等 —— 模板里若有人手写第二份，红。
 *   产物文本还断言「错误判定只引用共享判据」（`AX_PROTOCOL.failureErrorType`），
 *   真值判定 `if (msg[F.error])` 与写死 `"string"` 都算第二份判据 ⇒ 红。
 * - **门 C（能力面对称差集）**：宿主实际接受的 RPC 方法集合（行为探测，不看源码怎么写）
 *   与两个客户端运行时的 RPC 方法集合做差集。宿主多一个 / 客户端多一个 ⇒ 红。
 *   srcdoc 侧自任务 #78 起从产物里**解析 `AX_RPC_OPS` 数据块**（`SANDBOX_SRCDOC_RPC_OPS`），
 *   不再扫 `M.<key>` 的别名文本；提取不到 / 条数不等即红。
 * - **门 D（分母非零 + 边界形状）**：每条提取都断言最小条数；响应帧的成功/失败边界由
 *   `isFailureFrame` 单点定义，空串是失败。提取器写坏 ⇒ 差集恒空 = 空转守卫，
 *   这是 `pr-ci.workflow.test.ts` 的教训，绝不复用。
 * - **门 E（srcdoc 产物的真实行为）**：把生成的运行时喂给假 window 跑起来，
 *   证明「空串错误 ⇒ 面板 reject」在**产物**上成立，而不只在协议层的函数上成立。
 * - **门 F（两个概念各自落地）**：宿主三条出向投递恒取 `SANDBOX_MESSAGE_TARGET_ORIGIN`
 *   （文件里不残留 `window.location.origin` 分支），入向身份只认 `event.source` 比对。
 *
 * ⚠ 本文件用 `node:fs` 读源码文本（与 `ci.workflow.test.ts` 同形态），
 * 所以它校验的是**仓库现状**，不是运行时行为；门 C/E/F 的行为探测部分除外。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

import {
  PLUGIN_BRIDGE_CLIENT_METHODS,
  PLUGIN_BRIDGE_COMMAND_NAMES,
  PLUGIN_BRIDGE_COMMANDS,
} from "@/sdk/pluginBridgeProtocol";
import { AX_AGENT_PLUGIN_BRIDGE_COMMANDS } from "@/sdk/pluginClientBridge";
import { createHostApiBridge, createHostRpcBridge } from "@/sdk/rpcBridge";
import {
  buildRpcErrorFrame,
  buildRpcRequestFrame,
  buildRpcResultFrame,
  isFailureFrame,
  isFrameFromSandboxSource,
  isRpcResponseFrame,
  readRpcResponseCallId,
  readRpcResponseError,
  SANDBOX_FRAME_FIELDS,
  SANDBOX_FRAME_PROTOCOL_TABLE,
  SANDBOX_FRAME_TYPES,
  SANDBOX_MESSAGE_TARGET_ORIGIN,
  SANDBOX_RPC_ARG_KEYS,
  SANDBOX_RPC_FAILURE_ERROR_TYPE,
  SANDBOX_RPC_METHODS,
  SANDBOX_RPC_TIMEOUT_MS,
  SANDBOX_SRCDOC_RPC_OPS,
  type SandboxRpcResponseFrame,
} from "@/sdk/sandboxFrameProtocol";
import { DEFAULT_RPC_TIMEOUT_MS, generateSandboxHtml } from "@/sdk/sandboxTemplate";

/** 仓内相对路径 → 源码文本。 */
function sourceOf(repoPath: string): string {
  return readFileSync(resolve(process.cwd(), repoPath), "utf8");
}

/** 去掉 `//` 行注释与 `/* *\/` 块注释 —— 注释里引用协议串是**说明**，不是第二份实现。 */
function stripComments(source: string): string {
  const noBlock = source.replace(/\/\*[\s\S]*?\*\//g, " ");
  return noBlock
    .split("\n")
    .map((line) => {
      const match = /(^|[^:])\/\//.exec(line);
      return match ? line.slice(0, (match.index ?? 0) + (match[1] ? match[1].length : 0)) : line;
    })
    .join("\n");
}

/**
 * 协议词汇的字面量形态：帧类型串 / RPC 方法名 / 插件桥两条命令名。
 *
 * 只匹配**被引号包住**的取值（`"api.invoke"`、`'plugin_ui_action'`、模板串 `` `rpc:request` ``），
 * 属性访问（`api.emit(...)`）与标识符不参与 —— 它们不是「又抄了一份协议」。
 */
const PROTOCOL_LITERAL_RE =
  /(["'`])(?:rpc:(?:request|response)|host:(?:event|lifecycle)|skill:(?:ready|error)|api\.(?:invoke|emit)|ui\.(?:navigate|notify|getTheme|getLocale)|store\.(?:read|write)|plugin_ui_action|plugin_execute_command)\1/g;

/**
 * 第二条检测面（审查应修 3）：**帧字段名 / RPC 参数键名当字面量键写死**的两种形态。
 *
 * `obj["callId"]` 取下标 与 `{"callId": …}` 字符串键 —— 这两处一旦自写，改名的一侧不会让另一侧红。
 * 属性访问（`msg.callId`）不参与：它的形状由 `Sandbox*Frame` 接口钉住，改名编译期就红；
 * `F.callId` / `ARG.command` 这种「从表里取键」正是我们要的形态，不能误伤。
 */
const FIELD_NAME_ALT = [
  ...new Set([
    ...Object.values(SANDBOX_FRAME_FIELDS),
    ...Object.values(SANDBOX_RPC_ARG_KEYS),
  ]),
].join("|");

/**
 * 引号字符类：双引号 / 单引号 / 反引号。
 * 反引号用 96 号字符拼进来 —— 写在模板字符串里的反引号会提前终结模板，正则字面量才允许裸反引号。
 */
const QUOTE_CLASS = `["'${String.fromCharCode(96)}]`;

const FIELD_KEY_LITERAL_RE = new RegExp(
  `(?:\\[\\s*${QUOTE_CLASS}(${FIELD_NAME_ALT})${QUOTE_CLASS}\\s*\\])|(?:${QUOTE_CLASS}(${FIELD_NAME_ALT})${QUOTE_CLASS}\\s*:)`,
  "g",
);

/**
 * 第三条检测面（审查应修 1/3）：`"*"` **写在 `postMessage` 的 targetOrigin 位**。
 *
 * 出向 targetOrigin 只许取 `SANDBOX_MESSAGE_TARGET_ORIGIN`（判据的唯一定义处），
 * 消费点自写 `"*"` 就是第二份判据 —— 引擎语义改了它不会跟着改。
 */
const TARGET_ORIGIN_LITERAL_RE = /\.postMessage\s*\([^;{]*?,\s*["'`]\*["'`]\s*\)/g;

/** 从源码文本提取协议字面量（已剥注释）。 */
function protocolLiterals(text: string): string[] {
  const out: string[] = [];
  const re = new RegExp(PROTOCOL_LITERAL_RE.source, "g");
  let match: RegExpExecArray | null;
  while ((match = re.exec(stripComments(text))) !== null) {
    out.push(match[0].slice(1, -1));
  }
  return out;
}

/** 从源码文本提取「自写的字段名键」（已剥注释）。 */
function fieldKeyLiterals(text: string): string[] {
  const out: string[] = [];
  const re = new RegExp(FIELD_KEY_LITERAL_RE.source, "g");
  let match: RegExpExecArray | null;
  while ((match = re.exec(stripComments(text))) !== null) {
    out.push(match[1] ?? match[2]);
  }
  return out;
}

/** 从源码文本提取「自写的 targetOrigin 字面量投递点」（已剥注释）。 */
function targetOriginLiterals(text: string): string[] {
  const re = new RegExp(TARGET_ORIGIN_LITERAL_RE.source, "g");
  return stripComments(text).match(re) ?? [];
}

/** 消费方（不允许出现协议字面量的文件）。 */
const CONSUMER_FILES = [
  "src/sdk/rpcBridge.ts",
  "src/sdk/sandboxTemplate.ts",
  "src/sdk/types.ts",
  "src/sdk/pluginClientBridge.ts",
  "src/components/plugin/pluginSandboxBridge.ts",
  "src/components/skill/SkillSandboxContainer.tsx",
  "src/components/plugin/PluginPanelHost.tsx",
  // 页面（bundle 粒度）容器（任务 #64）：它接的是同一条桥，抄一份帧词汇就等于长出第二套协议
  "src/components/plugin/PluginPageHost.tsx",
] as const;

/** 协议唯一定义处。 */
const CONTRACT_FILES = ["src/sdk/sandboxFrameProtocol.ts", "src/sdk/pluginBridgeProtocol.ts"] as const;

/** 渲染出的 srcdoc 里内嵌的协议表 JSON。 */
function embeddedTableFrom(html: string): unknown {
  const match = /var AX_PROTOCOL = (\{[\s\S]*?\});\n/.exec(html);
  expect(match, "渲染出的沙箱运行时里没有 AX_PROTOCOL 注入点 ⇒ 模板不再从协议表取词汇（空转守卫）").not.toBeNull();
  return JSON.parse((match as RegExpExecArray)[1]);
}

/** 产物里 ctx 方法面数据块的两端标记（与 `sandboxTemplate.ts` 的生成点逐字一致）。 */
const RPC_OPS_BEGIN = "/* AX_RPC_OPS_BEGIN */";
const RPC_OPS_END = "/* AX_RPC_OPS_END */";

/** 产物里解析出的 op 行（与 `sandboxFrameProtocol.ts` 的 `SandboxCtxRpcOp` 同形）。 */
interface SrcdocRpcOp {
  readonly group: "api" | "ui" | "store";
  readonly name: string;
  readonly method: string;
  readonly args: readonly string[];
  readonly lastDefault?: string;
}

/**
 * 从渲染产物里解析 ctx 方法面数据（门 C 的唯一提取方式，任务 #78）。
 *
 * 提取失败一律抛断言错误，**不许返回空集**：空集进入对称差集等于「宿主多出的方法全算漂移」
 * 或（两边都空）静默通过。所以标记缺失、标记之间没有数据字面量、解析结果不是数组，
 * 三处都当场红；条数是否等于协议方法表长度由调用方再断言一次。
 */
function srcdocRpcOpsFrom(html: string): SrcdocRpcOp[] {
  const start = html.indexOf(RPC_OPS_BEGIN);
  const end = html.indexOf(RPC_OPS_END);
  expect(start, `渲染产物里没有 ${RPC_OPS_BEGIN} 标记 ⇒ 数据形态提取点丢了`).toBeGreaterThanOrEqual(0);
  expect(end, `渲染产物里没有 ${RPC_OPS_END} 标记（或顺序颠倒）`).toBeGreaterThan(start);
  const block = html.slice(start + RPC_OPS_BEGIN.length, end);
  const assign = /=\s*(\[[\s\S]*?\]);/.exec(block);
  expect(assign, "AX_RPC_OPS 标记之间没有 `= [ ... ];` 数据字面量 ⇒ 提取失败").not.toBeNull();
  const parsed: unknown = JSON.parse((assign as RegExpExecArray)[1]);
  expect(Array.isArray(parsed), "AX_RPC_OPS 不是数组 ⇒ 数据形态不成立").toBe(true);
  return parsed as SrcdocRpcOp[];
}

describe("门 A：协议字面量只允许住在两个协议模块", () => {
  it.each(CONSUMER_FILES)("%s 内不出现任何协议字面量（帧词汇 / 字段名当键 / 自写 targetOrigin）", (repoPath) => {
    const text = sourceOf(repoPath);
    const found = [...protocolLiterals(text), ...fieldKeyLiterals(text), ...targetOriginLiterals(text)];
    expect(
      found,
      `${repoPath} 里抄了一份协议字面量：${
        found.join(", ")
      } —— 请改为从 @/sdk/sandboxFrameProtocol 或 @/sdk/pluginBridgeProtocol 取值`,
    ).toEqual([]);
  });

  /**
   * 检测面自证（审查应修 3 的「一添加就报红」是硬承诺）：植入必命中、正确形态不误伤。
   *
   * 只测真实文件会有一种静默绿 —— 检测器写偏（正则漏一个括号、剥注释剥多了），
   * 消费方恰好也没写，于是门永远不红。这里用最小合成串把每个检测面的**正反面**都钉住。
   */
  it("三条检测面各自植入即命中，且不吃「从表里取键 / 用常量投帧」的正确形态", () => {
    expect(protocolLiterals(`const m = { type: "rpc:request" };`)).toEqual(["rpc:request"]);
    expect(fieldKeyLiterals(`const v = frame["callId"];`)).toEqual(["callId"]);
    expect(fieldKeyLiterals(`const v = { "storeName": n };`)).toEqual(["storeName"]);
    expect(targetOriginLiterals(`w.postMessage(frame, "*");`)).toHaveLength(1);
    // 反例：属性访问由接口钉形状、取键走常量、投帧用协议常量 ⇒ 都不算第二份协议
    expect(protocolLiterals(`api.emit("theme-change");`)).toEqual([]);
    expect(fieldKeyLiterals(`const v = frame[F.callId]; args?.[ARG.command];`)).toEqual([]);
    expect(targetOriginLiterals(`w.postMessage(frame, SANDBOX_MESSAGE_TARGET_ORIGIN);`)).toEqual([]);
    // 注释里的引用是说明，不是实现（豁免机制逐字可见：三个提取器一律先过 stripComments）
    expect(protocolLiterals(`// 历史串 "rpc:request" 只是说明\nconst a = 1;`)).toEqual([]);
    expect(fieldKeyLiterals(`/** 别再写 ["callId"] */\nconst a = 1;`)).toEqual([]);
    expect(targetOriginLiterals(`// 曾经写成 postMessage(frame, "*")\nconst a = 1;`)).toEqual([]);
  });

  it("协议模块确实持有全部词汇（提取器没写偏：分母非零）", () => {
    const contractLiterals = new Set(CONTRACT_FILES.flatMap((p) => protocolLiterals(sourceOf(p))));
    // 6 个帧类型 + 8 个方法名 + 2 条命令名 = 16 个必须能在协议模块源码里被提取到
    for (
      const value of [
        ...Object.values(SANDBOX_FRAME_TYPES),
        ...Object.values(SANDBOX_RPC_METHODS),
        ...PLUGIN_BRIDGE_COMMANDS,
      ]
    ) {
      expect(contractLiterals, `协议字面量「${value}」不在协议模块源码里 ⇒ 提取器或定义处出了问题`).toContain(value);
    }
    expect(contractLiterals.size).toBeGreaterThanOrEqual(16);
  });

  it("运行时取值与源文件字面量同源（不是两处各写一份恰好相等）", () => {
    // 命令名两侧的唯一来源
    expect([...PLUGIN_BRIDGE_COMMANDS].sort()).toEqual(Object.values(PLUGIN_BRIDGE_COMMAND_NAMES).sort());
    // 宿主再导出与 panel 别名与客户端导出：同一个数组对象
    expect(AX_AGENT_PLUGIN_BRIDGE_COMMANDS).toBe(PLUGIN_BRIDGE_COMMANDS);
  });
});

describe("门 B：srcdoc 运行时内嵌的协议表 == 协议模块的表", () => {
  const html = generateSandboxHtml({
    skillName: "parity-probe",
    skillId: "parity-probe:main",
    props: { entry: "index.html" },
    permissions: { commands: [...PLUGIN_BRIDGE_COMMANDS] },
    htmlContent: "<p>probe</p>",
  });

  it("注入表与来源深相等（模板手写第二份即红）", () => {
    expect(embeddedTableFrom(html)).toEqual(SANDBOX_FRAME_PROTOCOL_TABLE);
  });

  it("注入表不再带超时：运行时用按实例的 rpcTimeoutMs，表里那份无人读（审查稿第 12 条：删字段）", () => {
    const table = embeddedTableFrom(html) as {
      timeoutMs?: number;
      failureErrorType?: string;
      m: Record<string, string>;
      t: Record<string, string>;
    };
    // 表里没有第二个超时值；协议值 15000 仍只有 SANDBOX_RPC_TIMEOUT_MS 一处定义
    expect(Object.keys(table)).not.toContain("timeoutMs");
    expect(table.failureErrorType).toBe(SANDBOX_RPC_FAILURE_ERROR_TYPE);
    // 模板对外的历史常量名也必须跟着协议走，否则「谁超时」又是两份口径
    expect(DEFAULT_RPC_TIMEOUT_MS).toBe(SANDBOX_RPC_TIMEOUT_MS);
    expect(table.m.apiInvoke).toBe(SANDBOX_RPC_METHODS.apiInvoke);
    expect(table.t.request).toBe(SANDBOX_FRAME_TYPES.request);
  });

  it("生成的运行时只引用协议表，不含裸帧类型串", () => {
    // 渲染结果里出现的 `rpc:request` 等串只允许来自注入的 JSON 表本体
    const withoutTable = html.replace(/var AX_PROTOCOL = \{[\s\S]*?\};\n/, "");
    expect(protocolLiterals(withoutTable)).toEqual([]);
  });

  /**
   * 生成的运行时的**错误判定**（审查应修 2 的模板侧）：
   * 门 D 只测 builder/reader，测不到模板字符串里的判定，所以这里对**产物文本**断言：
   * 判定只引用协议表里的共享判据（`AX_PROTOCOL.failureErrorType`），
   * 既不含裸真值判定 `if (msg[F.error])`，也不把比较值 `"string"` 写死第二份。
   */
  it("生成的运行时：错误判定只引用共享判据，不含裸真值判定", () => {
    const runtime = html.replace(/var AX_PROTOCOL = \{[\s\S]*?\};\n/, "");
    // 判据函数存在且从表里取值
    expect(/\bfunction axIsFailure\(msg\)/.test(runtime), "运行时没有 axIsFailure ⇒ 判定又散回各分支").toBe(true);
    expect(runtime, "axIsFailure 没读协议表的 failureErrorType").toContain("AX_PROTOCOL.failureErrorType");
    // 失败分支只经由它
    expect(/if \(axIsFailure\(msg\)\)/.test(runtime), "失败分支没有走 axIsFailure（真值判定复发）").toBe(true);
    // 反向：裸真值判定与写死的比较值都不许出现
    expect(/if\s*\(\s*msg\[F\.error\]\s*\)/.test(runtime), "运行时退回收成 `if (msg[F.error])`").toBe(false);
    expect(/typeof\s+msg\[F\.error\]\s*[!=]==\s*["']string["']/.test(runtime), '运行时把判据值 "string" 写死了第二份')
      .toBe(false);
  });
});

describe("门 C：宿主接受集 ↔ 两个客户端引用集 的对称差集为空", () => {
  /** 排空宿主 handler 的微任务链（层数是实现细节，测试不数层数）。 */
  const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

  /** 用行为探测问宿主「这个方法你接不接」，不看源码怎么写。 */
  async function probeHostAccepted(methods: readonly string[]): Promise<string[]> {
    const sent: unknown[] = [];
    const contentWindow = {
      postMessage: (message: unknown): void => {
        sent.push(message);
      },
    } as never;
    // 权限面全空 ⇒ api.*/ui.*/store.* 一律「已注册但被拒」，与「未注册（Unknown method）」可区分；
    // 桩一律用未标注的 vi.fn()：宿主接口是泛型（`<T>() => Promise<T>`），带返回值的实现反而不匹配。
    const bridge = createHostApiBridge({
      api: { invoke: vi.fn(), emit: vi.fn() },
      ui: { navigate: vi.fn(), notify: vi.fn(), getTheme: vi.fn(), getLocale: vi.fn() },
      store: { read: vi.fn(), write: vi.fn() },
      permissions: {},
      contentWindow,
    });

    const accepted: string[] = [];
    for (const method of methods) {
      sent.length = 0;
      // 新签名：宿主接收的是**整个事件**（来源判定要用 event.source），
      // 这里的 source 必须就是 contentWindow，否则会被来源判定丢弃（那条丢弃本身另有用例）。
      bridge.handleRpcRequest({
        data: buildRpcRequestFrame(`probe-${method}`, method, {}),
        source: contentWindow,
      });
      await flush();
      const frame = sent[0];
      expect(isRpcResponseFrame(frame), `宿主对「${method}」没有回响应帧`).toBe(true);
      const error = readRpcResponseError(frame as SandboxRpcResponseFrame);
      // `Unknown method:` = 未注册；其它（含 Permission denied）= 已注册
      if (error === undefined || !error.startsWith("Unknown method:")) {
        accepted.push(method);
      }
    }
    bridge.destroy();
    return accepted;
  }

  /**
   * 从渲染出的 srcdoc 产物里**解析数据块**，得到它调用的 RPC 方法（任务 #78）。
   *
   * 与旧版（扫 `M.<key>` + 断言 `var M = AX_PROTOCOL.m` 别名）的区别：现在不读运行时的
   * 代码写法，只读 `sandboxTemplate.ts` 注入的那份 op 数据（表本身就是生成 ctx 的源）。
   * 提取失败 / 条数不等 ⇒ 在下两处断言里红，不会落成空集。
   */
  function methodsUsedBySrcdoc(): string[] {
    const html = generateSandboxHtml({
      skillName: "probe",
      skillId: "probe:main",
      props: {},
      permissions: {},
      htmlContent: "",
    });
    const ops = srcdocRpcOpsFrom(html);
    const tableKeys = Object.keys(SANDBOX_RPC_METHODS) as Array<keyof typeof SANDBOX_RPC_METHODS>;
    expect(
      ops.length,
      "从 srcdoc 运行时提取到的 op 数 ≠ 协议方法表长度 ⇒ 提取失效（空集不得当通过）",
    ).toBe(tableKeys.length);
    return ops.map((op) => {
      expect(tableKeys, `srcdoc 的 op 引用了协议表里没有的方法键 ${op.method}`).toContain(op.method);
      return SANDBOX_RPC_METHODS[op.method as keyof typeof SANDBOX_RPC_METHODS];
    });
  }

  /**
   * 数据形态本身的自证（任务 #78 的硬要求）：源表 == 产物里的数据；标记缺失 / 数据被清空
   * **必须**让提取失败或让条数断言不成立 —— 反过来证明上面的分母不是装饰。
   */
  it("srcdoc 的 op 表是数据形态：源表 == 产物数据；标记缺失或数据清空即红", () => {
    const html = generateSandboxHtml({
      skillName: "ops-probe",
      skillId: "ops-probe:main",
      props: {},
      permissions: {},
      htmlContent: "",
    });
    expect(srcdocRpcOpsFrom(html)).toEqual(SANDBOX_SRCDOC_RPC_OPS);
    // 标记被拿掉：提取器必须失败，而不是静默返回空集
    expect(() => srcdocRpcOpsFrom(html.replace(RPC_OPS_BEGIN, "/* marker-removed */"))).toThrow();
    // 数据被清成空数组：解析得到空集，但条数断言必须不成立（空集 ≠ 通过）
    const emptied = html.replace(/var AX_RPC_OPS = \[[\s\S]*?\];/, "var AX_RPC_OPS = [];");
    const emptyOps = srcdocRpcOpsFrom(emptied);
    expect(emptyOps).toHaveLength(0);
    expect(emptyOps.length, "空 op 集被当成通过 ⇒ 分母断言失效").not.toBe(Object.keys(SANDBOX_RPC_METHODS).length);
  });

  /** 从 bundle 客户端源码提取它引用的 RPC 方法键（`SANDBOX_RPC_METHODS.<key>` 形态）。 */
  function methodsUsedByBundleClient(): string[] {
    const code = stripComments(sourceOf("src/sdk/pluginClientBridge.ts"));
    const keys = new Set<string>();
    const re = /SANDBOX_RPC_METHODS\.([A-Za-z][A-Za-z0-9]*)/g;
    let match: RegExpExecArray | null;
    while ((match = re.exec(code)) !== null) {
      keys.add(match[1]);
    }
    expect(keys.size, "bundle 客户端没引用任何 RPC 方法 ⇒ 提取器写偏").toBeGreaterThan(0);
    const tableKeys = Object.keys(SANDBOX_RPC_METHODS) as Array<keyof typeof SANDBOX_RPC_METHODS>;
    for (const key of keys) {
      expect(tableKeys, `bundle 客户端引用了协议表里没有的方法键 ${key}`).toContain(key);
    }
    return [...keys].map((key) => SANDBOX_RPC_METHODS[key as keyof typeof SANDBOX_RPC_METHODS]);
  }

  const ALL_METHODS = Object.values(SANDBOX_RPC_METHODS);

  it("分母非零：协议方法表、srcdoc 引用集、bundle 引用集都不得为空", () => {
    expect(ALL_METHODS.length).toBeGreaterThanOrEqual(8);
    expect(methodsUsedBySrcdoc().length).toBeGreaterThanOrEqual(8);
    expect(methodsUsedByBundleClient().length).toBeGreaterThanOrEqual(1);
  });

  it("宿主接受的方法 == 两个客户端引用的方法（对称差集为空）", async () => {
    const clientSide = new Set([...methodsUsedBySrcdoc(), ...methodsUsedByBundleClient()]);
    const hostSide = new Set(await probeHostAccepted(ALL_METHODS));
    const hostOnly = [...hostSide].filter((m) => !clientSide.has(m)).sort();
    const clientOnly = [...clientSide].filter((m) => !hostSide.has(m)).sort();
    expect(
      { hostOnly, clientOnly },
      "宿主与沙箱客户端的能力面漂移：宿主多出的方法无人可调，客户端多出的方法必收到 Unknown method",
    ).toEqual({ hostOnly: [], clientOnly: [] });
  });

  it("bundle 客户端只被允许引用 api.invoke（刻意收窄，扩面需显式改这条）", () => {
    expect([...new Set(methodsUsedByBundleClient())].sort()).toEqual([...PLUGIN_BRIDGE_CLIENT_METHODS].sort());
  });
});

describe("门 D：响应帧的边界形状（错误与成功只有一处定义）", () => {
  const FIELD = SANDBOX_FRAME_PROTOCOL_TABLE.f;

  it("空串错误仍是失败：判据是「error 是不是字符串」，不是真值也不是键在不在", () => {
    const frame = buildRpcErrorFrame("c1", "");
    // 判据函数（三通道共用）与形状读取函数必须给同一个答案
    expect(isFailureFrame(frame)).toBe(true);
    expect(readRpcResponseError(frame)).toBe("");
    expect(readRpcResponseCallId(frame)).toBe("c1");
  });

  it("成功帧不带 error 键，且 isFailureFrame 判它成功", () => {
    const frame = buildRpcResultFrame("c2", { ok: 1 });
    expect(isFailureFrame(frame)).toBe(false);
    expect(readRpcResponseError(frame)).toBeUndefined();
    expect(Object.keys(frame)).not.toContain(FIELD.error);
  });

  it("`error: undefined` 的帧不算失败（srcdoc 的 sendResponse 恒带该键、值可为 undefined）", () => {
    expect(isFailureFrame({ type: SANDBOX_FRAME_TYPES.response, callId: "c2b", error: undefined } as never)).toBe(
      false,
    );
    expect(isFailureFrame({ type: SANDBOX_FRAME_TYPES.response, callId: "c2c", error: "" } as never)).toBe(true);
  });

  it("错误载荷是字符串（宿主把任何异常收成 String(e)）", () => {
    expect(readRpcResponseError(buildRpcErrorFrame("c3", new Error("boom")))).toBe("Error: boom");
    expect(readRpcResponseError(buildRpcErrorFrame("c4", { code: "X" }))).toBe("[object Object]");
  });

  it("请求帧在 args 缺省时不带 args 键（客户端与宿主共用同一构造器）", () => {
    const frame = buildRpcRequestFrame("c5", SANDBOX_RPC_METHODS.apiInvoke);
    expect(Object.keys(frame)).not.toContain(FIELD.args);
    expect(frame.method).toBe(SANDBOX_RPC_METHODS.apiInvoke);
  });
});

/**
 * 门 E：**把生成的 srcdoc 运行时真跑起来**。
 *
 * 门 D 只能证明「协议层的判据对了」，证明不了模板侧（那是一段字符串）。审查应修 2 明确要求
 * 行为级证据：空串错误在**两条通道**上都得判失败。bundle 通道在
 * `pluginClientBridge.test.ts`；这里补 srcdoc 通道。
 */
describe("门 E：srcdoc 运行时的真实行为", () => {
  /**
   * 用假 window 实例化产物脚本，返回它的对外触点。
   *
   * 不借 jsdom 的 window：要断的是**产物文本被解释后做什么**，自投假 window 更贴近
   * panel 粒度真实链路（sandbox="allow-scripts" 的文档本身就是与宿主隔离的一方）。
   *
   * ⚠ `hostWindow` 必须**同时**是 `fakeWindow.parent` 与投帧事件里的 `event.source`：
   * 产物自 #72 起按宿主侧同一个谓词判来源（`isFrameFromSandboxSource(event, window.parent)`），
   * 二者不是同一对象就等于「宿主自己发的帧被沙箱当成外来帧」—— 那是攻击形态，不是正常通路。
   */
  function instantiateSrcdocRuntime(): {
    /** 未替换占位符的渲染产物（数据形态提取用；与运行时同一份模板）。 */
    html: string;
    posted: Array<{ message: unknown; targetOrigin: string }>;
    deliver: (frame: unknown, source?: unknown) => void;
    ctxApi: { invoke: (command: string, args?: unknown) => Promise<unknown> };
    /** 运行时挂出的完整 ctx（api / ui / store 三个分组）。 */
    ctx: Record<"api" | "ui" | "store", Record<string, (...args: unknown[]) => unknown>>;
    /** 把方法注册进沙箱（宿主 `callSkillMethod` 的那一侧），供「伪造来源 ⇒ 不执行」用例使用。 */
    registerMethod: (name: string, fn: (args: unknown) => unknown) => void;
    /** 与 `hostWindow` 不是同一对象的一块「别的文档」，即伪造来源用例要用的那个 source。 */
    foreignWindow: unknown;
  } {
    const html = generateSandboxHtml({
      skillName: "behavior-probe",
      skillId: "behavior-probe:main",
      props: {},
      permissions: {},
      htmlContent: "",
    });
    const rendered = html
      .replace('"__SKILL_NAME__"', '"behavior-probe"')
      .replace('"__SKILL_ID__"', '"behavior-probe:main"')
      .replace("__INITIAL_PROPS__", "{}")
      .replace("__PERMISSIONS__", "{}");
    // 产物里有**两个** script 块（head 里禁 fetch/XHR 的那个 + body 末尾的运行时）。
    // 取「最后一个 <script> 到最后一个 </script>」是确定性的；正则的非贪婪会从第一个块起匹配，
    // 把中间的 HTML 一起吞进来得到的脚本（实测 SyntaxError: Unexpected token '<'）。
    const scriptOpen = rendered.lastIndexOf("<script>");
    const scriptStart = scriptOpen + "<script>".length;
    const scriptEnd = rendered.lastIndexOf("</script>");
    expect(scriptOpen, "产物里没有 script 块 ⇒ 模板结构变了，这条行为证据失效").toBeGreaterThan(0);
    expect(scriptEnd, "产物里的 script 块没有闭合").toBeGreaterThan(scriptStart);
    const scriptText = rendered.slice(scriptStart, scriptEnd);
    expect(scriptText, "取到的不是运行时那段（模板结构变了）").toContain("AxAgent Skill Sandbox Runtime");

    const posted: Array<{ message: unknown; targetOrigin: string }> = [];
    const listeners: Array<(event: { origin: string; data: unknown; source: unknown }) => void> = [];
    /** 沙箱眼里的「宿主文档」：既是 `window.parent`，也是宿主那几帧的 `event.source`。 */
    const hostWindow = {
      postMessage: (message: unknown, targetOrigin: string): void => {
        posted.push({ message, targetOrigin });
      },
    };
    /** 应用里另一块可脚本化文档 —— 与 `hostWindow` 不是同一对象，就是 #72 要拒的那个来源。 */
    const foreignWindow = { postMessage: (): void => {} };
    const fakeWindow: Record<string, unknown> = {
      parent: hostWindow,
      addEventListener: (
        type: string,
        listener: (event: { origin: string; data: unknown; source: unknown }) => void,
      ): void => {
        if (type === "message") {
          listeners.push(listener);
        }
      },
      removeEventListener: (
        type: string,
        listener: (event: { origin: string; data: unknown; source: unknown }) => void,
      ): void => {
        if (type !== "message") {
          return;
        }
        const index = listeners.indexOf(listener);
        if (index >= 0) {
          listeners.splice(index, 1);
        }
      },
      location: { origin: "null" },
      dispatchEvent: (): boolean => true,
    };
    const fakeDocument = { documentElement: { setAttribute: (): void => {} } };

    // 产物文本交给 Function 执行；window / document 由参数注入，其余（Promise/Date/setTimeout）用宿主全局
    new Function("window", "document", scriptText)(fakeWindow, fakeDocument);

    const ctx = fakeWindow.ctx as Record<
      "api" | "ui" | "store",
      Record<string, (...args: unknown[]) => unknown>
    >;
    const ctxApi = ctx.api as { invoke: (command: string, args?: unknown) => Promise<unknown> };
    expect(ctxApi, "运行时没挂出 window.ctx.api ⇒ 模板产物结构变了").toBeTruthy();

    /** 宿主 → 沙箱投一帧：`source` 缺省是宿主自己，传 `foreignWindow` 即「别的文档伪造的一帧」。 */
    const deliver = (frame: unknown, source: unknown = hostWindow): void => {
      for (const listener of listeners.slice()) {
        listener({ origin: "null", data: frame, source });
      }
    };

    // 运行时会自行上报 ready 并请求 ui.getTheme；回掉它，免得留一个 15s 的悬挂定时器
    const themeRequest = posted[1]?.message as { callId?: string };
    expect(themeRequest?.callId, "运行时启动时没请求 ui.getTheme ⇒ 启动序变了，请复核这条证据").toBeTruthy();
    deliver(buildRpcResultFrame(themeRequest.callId as string, "light"));

    return {
      html,
      posted,
      deliver,
      ctxApi,
      ctx,
      registerMethod: (name: string, fn: (args: unknown) => unknown): void => {
        (fakeWindow.registerSkillMethod as (n: string, f: (a: unknown) => unknown) => void)(name, fn);
      },
      foreignWindow,
    };
  }

  /** 最近一次 srcdoc → 宿主的请求帧。 */
  function lastRequest(sandbox: ReturnType<typeof instantiateSrcdocRuntime>) {
    const frame = sandbox.posted[sandbox.posted.length - 1]!.message as { callId: string; type: string };
    expect(frame.type).toBe(SANDBOX_FRAME_TYPES.request);
    return frame;
  }

  it("宿主回空串错误 ⇒ srcdoc 面板 reject（审查应修 2：不再 resolve(undefined)）", async () => {
    const sandbox = instantiateSrcdocRuntime();
    const promise = sandbox.ctxApi.invoke("some_command", {});
    const request = lastRequest(sandbox);

    sandbox.deliver(buildRpcErrorFrame(request.callId, ""));
    const settled = await promise.then((): null => null).catch((e: unknown) => e as Error);
    expect(settled, "空串错误被 srcdoc 运行时当成成功").toBeInstanceOf(Error);
    expect((settled as Error).message).toBe("");
  });

  it("非空错误同样 reject；成功帧 resolve 结果（同一通道两侧对称）", async () => {
    const sandbox = instantiateSrcdocRuntime();
    const boom = sandbox.ctxApi.invoke("some_command");
    sandbox.deliver(buildRpcErrorFrame(lastRequest(sandbox).callId, "boom"));
    await expect(boom).rejects.toThrow("boom");

    const ok = sandbox.ctxApi.invoke("some_command");
    sandbox.deliver(buildRpcResultFrame(lastRequest(sandbox).callId, { handled: true }));
    await expect(ok).resolves.toEqual({ handled: true });
  });

  it("srcdoc 的出向投递用协议表里的 targetOrigin（产物里不写死第二份 origin）", () => {
    const sandbox = instantiateSrcdocRuntime();
    expect(sandbox.posted.length).toBeGreaterThanOrEqual(2);
    for (const frame of sandbox.posted) {
      expect(frame.targetOrigin).toBe(SANDBOX_MESSAGE_TARGET_ORIGIN);
    }
  });

  /**
   * 数据即行为（任务 #78）：`AX_RPC_OPS` 若不是「运行时真正会调用的方法面」，
   * 门 C 的数据提取就成了装饰（与已删的 `timeoutMs` 装饰值同类）。逐条调用 ctx 方法，
   * 核对发出的 method 与参数字段 —— 表 = 生成源 ⇒ 这一条按构造成立，但仍要在**产物**上证一次。
   */
  it("AX_RPC_OPS 每一行都真的落到 ctx 上：method / 参数字段与表逐条一致", async () => {
    const sandbox = instantiateSrcdocRuntime();
    const ops = srcdocRpcOpsFrom(sandbox.html);
    expect(ops.length, "op 表解析为空 ⇒ 本用例失去判据").toBe(Object.keys(SANDBOX_RPC_METHODS).length);

    for (const op of ops) {
      const fn = sandbox.ctx[op.group][op.name];
      expect(typeof fn, `ctx.${op.group}.${op.name} 不存在 ⇒ 数据表与运行时行为脱节`).toBe("function");

      const before = sandbox.posted.length;
      const probeArgs = ["a0", "a1", "a2"].slice(0, op.args.length);
      const pending = fn(...probeArgs) as Promise<unknown>;
      expect(sandbox.posted.length, `ctx.${op.group}.${op.name} 没有发出帧`).toBe(before + 1);

      const frame = sandbox.posted[before]!.message as {
        type: string;
        callId: string;
        method: string;
        args?: Record<string, unknown>;
      };
      expect(frame.type).toBe(SANDBOX_FRAME_TYPES.request);
      expect(frame.method, `ctx.${op.group}.${op.name} 发出的方法名与 op 表不符`).toBe(
        SANDBOX_RPC_METHODS[op.method as keyof typeof SANDBOX_RPC_METHODS],
      );

      if (op.args.length === 0) {
        // 无参方法不带 args —— 与协议构造器（buildRpcRequestFrame 缺省不带 args）同口径
        expect(frame.args, `ctx.${op.group}.${op.name} 无参却带了 args 字段`).toBeUndefined();
      } else {
        const expectedArgs = Object.fromEntries(
          op.args.map((key, index) => [
            SANDBOX_RPC_ARG_KEYS[key as keyof typeof SANDBOX_RPC_ARG_KEYS],
            probeArgs[index],
          ]),
        );
        expect(frame.args, `ctx.${op.group}.${op.name} 的参数字段与 op 表不符`).toEqual(expectedArgs);
      }

      sandbox.deliver(buildRpcResultFrame(frame.callId, null));
      await expect(pending).resolves.toBeNull();
    }

    // 反向：ctx 上不许有表外的隐藏 RPC 方法（否则「表 = 方法面」只覆盖了一个方向）
    for (const group of ["api", "ui", "store"] as const) {
      const expected = ops.filter((op) => op.group === group).map((op) => op.name);
      if (group === "ui") {
        // 唯一的非 RPC 成员：本地回调注册（不发帧，故不在 op 表里）
        expected.push("onThemeChange");
      }
      expect(Object.keys(sandbox.ctx[group]).sort(), `ctx.${group} 的方法面与 op 表不一致`).toEqual(expected.sort());
    }

    // 末位缺省值（op.lastDefault）仍按原样生效：ui.notify(message) 的 type 缺省 "info"
    const beforeNotify = sandbox.posted.length;
    const notify = sandbox.ctx.ui.notify("hello") as Promise<unknown>;
    const notifyFrame = sandbox.posted[beforeNotify]!.message as {
      callId: string;
      args?: Record<string, unknown>;
    };
    expect(notifyFrame.args).toEqual({
      [SANDBOX_RPC_ARG_KEYS.message]: "hello",
      [SANDBOX_RPC_ARG_KEYS.type]: "info",
    });
    sandbox.deliver(buildRpcResultFrame(notifyFrame.callId, null));
    await notify;
  });

  /**
   * #72：沙箱**收**宿主帧那一侧也按来源判身份，且用的是宿主侧同一条谓词。
   *
   * 断言全部打在拒绝的**行为**上（注册方法有没有被调用、有没有回包），不去回头调用
   * `isFrameFromSandboxSource` 自证 —— 那条函数自己绿不代表产物认它（门 E 的价值就在这）。
   * 两条放在一起是为了防「一律拒绝」式假绿：同一帧只是来源不同，一拒一放。
   */
  it("外来来源的 rpc:request ⇒ 沙箱不执行注册方法、不回包；同一帧由宿主来源投出即照常执行", () => {
    const sandbox = instantiateSrcdocRuntime();
    let calls = 0;
    sandbox.registerMethod("probe", () => {
      calls += 1;
      return "done";
    });
    const frame = buildRpcRequestFrame("host_probe_1", "probe", {});
    const postedBefore = sandbox.posted.length;

    sandbox.deliver(frame, sandbox.foreignWindow);
    expect(calls, "外来帧在沙箱文档里被执行了 —— 沙箱这一侧没在判来源").toBe(0);
    expect(sandbox.posted.length, "沙箱给来历不明的帧回了包（回包本身就是「这帧被认了」这条信息）").toBe(postedBefore);

    sandbox.deliver(frame);
    expect(calls, "宿主自己发的帧被当成外来帧 —— 期望来源接错了").toBe(1);
    const reply = sandbox.posted[sandbox.posted.length - 1]!.message as { callId: string };
    expect((reply as { callId: string }).callId).toBe("host_probe_1");
  });

  it("外来来源的响应帧不再提前 settle 在途调用；同 callId 换宿主来源才结算", async () => {
    const sandbox = instantiateSrcdocRuntime();
    const inFlight = sandbox.ctxApi.invoke("some_command", {});
    const request = lastRequest(sandbox);

    let settled = false;
    inFlight.then(
      () => {
        settled = true;
      },
      () => {
        settled = true;
      },
    );
    sandbox.deliver(buildRpcResultFrame(request.callId, "stolen"), sandbox.foreignWindow);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(settled, "陌生来源用猜得到的 callId 结算了在途调用").toBe(false);

    sandbox.deliver(buildRpcResultFrame(request.callId, "from-host"));
    await expect(inFlight).resolves.toBe("from-host");
  });

  /**
   * 产物里的来源判据必须是协议模块**那一条函数**的原文（内联进去的），不是模板里另写的比较。
   *
   * 为什么锁文本而不是只跑行为：`null`/`undefined` 期望一律拒（#67 定死的那条边界）在真实
   * 沙箱文档里几乎不可达（`window.parent` 总是个 WindowProxy），行为用例覆盖不到它；
   * 这条文本断言保证「模板侧悄悄写一份更宽松的比较」当场红 —— 判据改了只能改在协议模块里。
   */
  it("生成的运行时内联的就是 isFrameFromSandboxSource 那条实现（没有第二份判据）", () => {
    const html = generateSandboxHtml({
      skillName: "source-probe",
      skillId: "source-probe:main",
      props: {},
      permissions: {},
      htmlContent: "",
    });
    expect(html).toContain(`var AX_IS_FRAME_FROM_SOURCE = ${isFrameFromSandboxSource.toString()};`);
    // 反面对照：模板文本里没有「只看 origin 就当身份」那种写法被留下来当唯一判据
    expect(stripComments(html)).toContain("axIsFromHost(event)");
  });
});

/**
 * 门 F：出向投递的 targetOrigin 与入向帧的来源判定是**两个概念**（审查应修 1 + 4）。
 * 判据本体住在 `sandboxFrameProtocol.ts`，这里只验两侧确实按它办事。
 */
describe("门 F：出向 targetOrigin 与入向 source 判定", () => {
  /** 记录宿主 → 沙箱每一次投递用的 targetOrigin。 */
  function spyContentWindow(): {
    window: unknown;
    posted: Array<{ message: unknown; targetOrigin: string }>;
  } {
    const posted: Array<{ message: unknown; targetOrigin: string }> = [];
    const win = {
      postMessage: (message: unknown, targetOrigin: string): void => {
        posted.push({ message, targetOrigin });
      },
    };
    return { window: win, posted };
  }

  it("宿主三条出向投递恒用协议常量（sendMessage / callSkillMethod / 回包）", async () => {
    const sandbox = spyContentWindow();
    const bridge = createHostRpcBridge(sandbox.window as never);
    bridge.sendLifecycle("mount", {});
    bridge.emitEvent("theme-change", { theme: "light" });
    const inFlight = bridge.callSkillMethod("ping");
    expect(sandbox.posted.length).toBe(3);
    for (const frame of sandbox.posted) {
      expect(frame.targetOrigin).toBe(SANDBOX_MESSAGE_TARGET_ORIGIN);
    }
    // 无人应答 ⇒ destroy 把在途请求拒掉（顺手锁「不悬挂」）
    bridge.destroy();
    await expect(inFlight).rejects.toThrow(/destroyed/i);
  });

  it("宿主 API 桥的回包同样用协议常量", async () => {
    const sandbox = spyContentWindow();
    const apiBridge = createHostApiBridge({
      api: { invoke: vi.fn(), emit: vi.fn() },
      ui: { navigate: vi.fn(), notify: vi.fn(), getTheme: vi.fn(), getLocale: vi.fn() },
      store: { read: vi.fn(), write: vi.fn() },
      permissions: {},
      contentWindow: sandbox.window as never,
    });
    apiBridge.handleRpcRequest({
      data: buildRpcRequestFrame("c-host-1", SANDBOX_RPC_METHODS.apiInvoke, { command: "x" }),
      source: sandbox.window,
    });
    await new Promise((r) => setTimeout(r, 0));
    apiBridge.destroy();
    expect(sandbox.posted.length).toBe(1);
    expect(sandbox.posted[0]!.targetOrigin).toBe(SANDBOX_MESSAGE_TARGET_ORIGIN);
  });

  it("rpcBridge 源码：每个 postMessage 调用点都取协议常量，且不残留 origin 分支", () => {
    const code = stripComments(sourceOf("src/sdk/rpcBridge.ts"));
    const sites = code.match(/\.postMessage\s*\(/g) ?? [];
    const withConst = code.match(/\.postMessage\s*\([^;{]*?,\s*SANDBOX_MESSAGE_TARGET_ORIGIN\s*\)/g) ?? [];
    expect(sites.length, "宿主出向投递点数量变了，这条断言要跟着复核").toBeGreaterThanOrEqual(3);
    expect(withConst.length, "有出向投递点没用协议常量 ⇒ 又长出第二份 origin 判据").toBe(sites.length);
    // 被删掉的缺陷形态：按 window.location.protocol 取宿主自身 origin 当 targetOrigin
    expect(code).not.toMatch(/location\.origin/);
    expect(code).not.toContain("defaultAllowedOrigin");
  });

  it("入向身份判据：source 不是本桥的沙箱 ⇒ 判假；contentWindow 未就绪 ⇒ 一帧都不认", () => {
    const sandboxWindow = {};
    const foreignWindow = {};
    const frame = buildRpcRequestFrame("c-fake", SANDBOX_RPC_METHODS.apiInvoke, { command: "x" });
    expect(isFrameFromSandboxSource({ data: frame, source: sandboxWindow }, sandboxWindow)).toBe(true);
    expect(isFrameFromSandboxSource({ data: frame, source: foreignWindow }, sandboxWindow)).toBe(false);
    expect(isFrameFromSandboxSource({ data: frame, source: null }, null)).toBe(false);
  });
});
