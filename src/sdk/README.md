# AxAgent SDK

AxAgent 的 ACP 协议客户端 SDK，支持 TypeScript 和 Python。

## 安装

### TypeScript / JavaScript

```bash
npm install @axagent/sdk
```

### Python

```bash
pip install axagent-sdk
```

或者从源码安装：

```bash
cd src/sdk/python
pip install .
```

## 快速开始

### TypeScript

```typescript
import { AxAgentClient } from "@axagent/sdk";

const client = new AxAgentClient("http://localhost:9876");

// 创建会话
const session = await client.createSession({
  workDir: "/path/to/project",
  model: "deepseek-v4-pro",
});

// 发送 prompt
const result = await client.sendPrompt(session.sessionId, "分析项目结构");
console.log(result.content);

// 关闭会话
await client.closeSession(session.sessionId);
```

使用会话上下文管理器：

```typescript
import { AxAgentClient, AxAgentSession } from "@axagent/sdk";

const client = new AxAgentClient("http://localhost:9876");

await using (const session = new AxAgentSession(client, "/path/to/project")) {
  const result = await session.send("分析项目结构");
  console.log(result.content);
}
// 会话自动关闭
```

### Python

```python
from axagent_sdk import AxAgentClient

client = AxAgentClient("http://localhost:9876")

# 创建会话
session = client.create_session(work_dir="/path/to/project")

# 发送 prompt
result = client.send_prompt(session["sessionId"], "分析项目结构")
print(result["content"])

# 关闭会话
client.close_session(session["sessionId"])
```

使用会话上下文管理器：

```python
from axagent_sdk import AxAgentClient, AxAgentSession

client = AxAgentClient("http://localhost:9876")

with AxAgentSession(client, "/path/to/project") as session:
    result = session.send("分析项目结构")
    print(result["content"])
# 会话自动关闭
```

## API 参考

| 方法                                             | 说明                   |
| ------------------------------------------------ | ---------------------- |
| `create_session(work_dir, ...)`                  | 创建新会话             |
| `get_session(session_id)`                        | 获取会话状态           |
| `list_sessions()`                                | 列出所有会话           |
| `send_prompt(session_id, prompt)`                | 发送 prompt 并获取响应 |
| `interrupt(session_id)`                          | 中断执行               |
| `close_session(session_id)`                      | 关闭会话               |
| `register_hook(session_id, event, callback_url)` | 注册 hook 回调         |
| `health_check()`                                 | 健康检查               |

## 插件前端 bundle 客户端（第三方插件必读）

上面两节是 **ACP 客户端 SDK**（外部程序驱动 AxAgent 会话）。本节是另一件事：
**插件自带的前端产物**（L1 功能包的页面）在沙箱 iframe 里运行时，怎么调用宿主开放的那两条通道。

走 asset 协议装载的 bundle **不由宿主包 `srcdoc`**，所以产物里必须自带这段客户端运行时，
否则它对不上宿主的 `postMessage` 协议 —— 桥就只是「宿主半边」。

### 要复制的文件（三个，原样 vendored，不要改）

| 文件                              | 作用                                                                              |
| --------------------------------- | --------------------------------------------------------------------------------- |
| `src/sdk/sandboxFrameProtocol.ts` | postMessage 帧协议的唯一词汇表（帧类型、RPC 方法名、字段名、超时、构造/解析函数） |
| `src/sdk/pluginBridgeProtocol.ts` | 插件面契约：白名单两条命令名 + `pluginId` 不可传的实现                            |
| `src/sdk/pluginClientBridge.ts`   | 客户端运行时：`createAxAgentPluginClient()`                                       |

三个文件对模块的引用**只指向彼此**，这条性质由 `src/sdk/__tests__/pluginClientBridge.test.ts` 的
「沙箱客户端可携带性」用例钉住。门实际认的是五种写法（不是只看行首 import）：
静态 `import … from`（含 `import type`）、副作用 `import "…"`、re-export
（`export {a} from` / `export * from` / `export * as ns from`）、动态 `import("…")`、`require("…")`；
另有一条更粗的兜底：`@/lib/invoke`、`@tauri-apps`、`@/` 这些**字面串**即使在代码里（注释已剥离）也不许出现，
所以把说明符拆进变量或拼串也躲不掉。用例还自带「四种形态植入即命中、合法形态不误伤」的自证 ——
没有那条自证，正则写偏 + 文件恰好没写就会静默长绿，「一添加内部依赖就报红」就还是空话。
⇒ 有人往这三个文件里加一条应用内部依赖，该用例即红，第三方不会被一个装不出来的 SDK 骗到。

### 用法

```ts
import { createAxAgentPluginClient } from "./vendor/pluginClientBridge";

const plugin = createAxAgentPluginClient();

// 上报「内容已就绪」：宿主侧容器收到这一帧才开始接 RPC 并撤掉加载超时
// （面板走 srcdoc 时由宿主注入的运行时自动发，页面走 asset 协议时要产物自己发）
plugin.announceReady();

// 用户动作回流到本插件自己的 worker
const state = await plugin.uiAction({ type: "refresh" });

// 按名字调用本插件声明的命令（参数形状看该插件 manifest 的 command schema）
await plugin.executeCommand({ name: "greet", input: { 名字: "世界" } });

// 组件卸载时释放（解绑 message 监听并拒绝在途调用）
plugin.dispose();
```

### 必须知道的三条边界

1. **只有两条命令可调用**：`plugin_ui_action` 与 `plugin_execute_command`。
   `invoke()` 的参数类型就是这两条；运行期再判一次，白名单外**一帧都不发**（宿主侧同样会拒）。
   想要第三条通道是宿主侧的改动，不是插件侧的绕法。
2. **不要传 `pluginId`**。归属由宿主按「这份内容属于哪个包」绑定，外部传值一律丢弃：
   类型上该键是 `never`（TS 调用点直接编译报错），运行期客户端也会剥掉再发帧。
   传别人的 id 不会让你调到别人的 worker，只会让你的参数少一个键。
3. **宿主未接帧时会等到超时再拒绝**（默认 15000ms，与宿主同一条超时值），不会永久悬挂。
   页面首帧早于宿主监听是正常时序，请把它当成可重试的失败处理。

### `announceReady()` 与 `dispose()` 是硬要求，不是建议（任务 #64 / §20.4 A/B）

上面「用法」那两行调用看着像可选的礼貌，实际是**这条通道唯一的开闸信号与唯一的回收口**。
两条都会静默失效，且症状都会把人引到错误的方向上：

| 没做                       | 真实症状                                                                                                                                                            | 会被误判成            | 怎么排除                                                                                                                      |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| 入口没调 `announceReady()` | 宿主容器（`src/components/plugin/PluginPageHost.tsx`）停在 loading，`SANDBOX_LOAD_TIMEOUT_MS`（30 s）后落「沙箱加载超时，请重试」并出错误态 —— **页面白一块并报错** | 「桥不通 / IPC 坏了」 | 这段时间里 IPC **一次都没发**（桥还没建）。先查产物入口有没有在顶层调 `announceReady()`，再查有没有把测试注入点当真参数传进去 |
| 卸载时没调 `dispose()`     | 该文档内的 `message` 监听器与在途 `setTimeout` 不回收                                                                                                               | 「一次没什么影响」    | 泄漏**按重试次数累计**：宿主的重试是重挂一块新 iframe（旧文档随卸载回收），但同一个文档里反复建客户端而不释放就会堆起来       |

- **生产用法就是无参**：`createAxAgentPluginClient()` —— 缺省投递面是 `window.parent` / `window`。
  `postTarget` / `messageSource` / `hostWindow` / `timeoutMs` / `targetOrigin` 五个键**是测试注入点**，
  产物里传它们等于把测试替身投进真实投递面（宿主容器也不传，见该文件的 A/B 注释）。
  `hostWindow` 是入向帧的**期望来源**（缺省 `window.parent`，与 srcdoc 运行时同一条
  `isFrameFromSandboxSource` 谓词）：来源不对的响应帧一律丢弃 —— 沙箱文档里再嵌一块可脚本化文档，
  也无法用猜到的 `callId` 提前结算在途调用。
- **时机**：入口模块顶层求值即创建、随即调一次 `announceReady()`。重复调用无害（宿主对就绪帧是幂等的，
  只会建一座桥）。
- **`dispose()` 之后不可复用**：新调用、在途 Promise、迟到消息三条一律以「已释放」收口；
  需要继续用请新建客户端（不提供 reset / 复活语义）。
- **对称的那一半在宿主**：容器卸载路径拆桥 + 摘监听 + 清超时，与产物侧的 `dispose()` 是同一处的两半，
  缺任一半都会留下一侧不回收的监听或在途 Promise。

表单/列表类页面不需要 bundle：在 `plugin.json` 的 `pages[]` 里声明 `loading: "schema"`，
内容走动态 UI Schema 通道（宿主侧由同一个页面容器分派到 `useDynamicUIStore` 那条，
**不会**调 `plugin_resolve_page_asset`），本节的客户端与那条通道无关。

## 依赖

- **Python SDK**: 仅使用 Python 标准库，无第三方依赖，要求 Python >= 3.8
- **TypeScript SDK**: 仅使用 Node.js 内置模块，无第三方依赖
- **插件前端 bundle 客户端**: 三个 `.ts` 源文件零外部依赖（见上节；不需要 `@tauri-apps/*`，
  沙箱内只有 postMessage，IPC 一律发生在宿主侧）
