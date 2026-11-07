> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# AFSIM-AxAgent 分布式集成架构设计

> 版本: 4.0
> 日期: 2026-05-09
> 基于: AFSIM 2.9.0 + AxAgent Skill 架构
> 扩展: Rust 高性能计算重构 + 模型-引擎解耦 + 算法-数据分离

---

## 目录

1. [AFSIM 模块集成评估](#1-afsim-模块集成评估)
2. [8 类组件谱系](#2-8-类组件谱系)
3. [能力环补全策略](#3-能力环补全策略)
4. [分布式 Skill 通信架构](#4-分布式-skill-通信架构)
5. [实时性能保证](#5-实时性能保证)
6. [Tool 提取与部署](#6-tool-提取与部署)
7. [Service 部署方案](#7-service-部署方案)
8. [完整部署拓扑](#8-完整部署拓扑)
9. [模块与组件完整对应关系](#9-模块与组件完整对应关系)
10. [8 类组件部署汇总](#10-8-类组件部署汇总)
11. [实现优先级与工作量评估](#11-实现优先级与工作量评估)
12. [Rust 高性能计算引擎架构](#12-rust-高性能计算引擎架构)
13. [模型与引擎完全解耦设计](#13-模型与引擎完全解耦设计)
14. [算法与数据分离架构](#14-算法与数据分离架构)
15. [Rust 重构实施路线图](#15-rust-重构实施路线图)

---

## 1. AFSIM 模块集成评估

### 1.1 源码规模

| 指标 | 数值 |
|------|------|
| 总 .hpp 文件数 | 4,421 |
| 总类数 | 16,460 |
| 可调用方法总数 | 56,678 |

### 1.2 按模块可调用方法分布

#### tools/ 目录

| 子模块 | 可调用方法数 |
|--------|-----------|
| dis (DIS协议) | 5,576 |
| util (工具库) | 4,034 |
| wkf (框架) | 3,626 |
| vespatk (VESPATK) | 1,390 |
| utilosg (OSG工具) | 985 |
| genio (通用IO) | 984 |
| util_script (脚本) | 766 |
| utilqt (Qt工具) | 605 |
| geodata (地理数据) | 367 |
| packetio (包IO) | 286 |

#### core/ 目录

| 子模块 | 可调用方法数 |
|--------|-----------|
| wsf (核心框架) | 8,034 |
| wsf_mil (军事) | 3,555 |
| wsf_space (太空) | 1,597 |
| wsf_parser (解析) | 1,061 |
| wsf_l16 (Link16) | 673 |
| wsf_cyber (网络战) | 500 |
| wsf_nx | 262 |
| wsf_util | 191 |
| wsf_ripr | 169 |
| sensor_plot_lib | 145 |
| wsf_mtt (多目标跟踪) | 105 |

#### 其他

| 子模块 | 可调用方法数 |
|--------|-----------|
| wsf_plugins | 8,110 |
| wizard | 3,654 |
| warlock | 2,228 |
| mover_creator | 2,059 |
| mystic | 1,466 |
| engage | 202 |
| post_processor | 174 |
| weapon_tools | 112 |

### 1.3 Skill 集成适合度分级

#### 高度适合（直接 Skill 集成）

| 模块 | 方法数 | 理由 |
|------|--------|------|
| engage | 202 | 交战管理，有完整生命周期和事件流 |
| mission | ~150 | 任务规划，有状态机和执行流程 |
| weapon_tools | 112 | 武器计算，可混合为 Skill + Tool |
| wsf (核心) | 8,034 | 仿真核心框架，最强模块特征 |
| wsf_mtt | 105 | 多目标跟踪，有航迹管理状态 |
| wizard | 3,654 | 场景编辑器，有完整 UI 交互 |

#### 中度适合（Service 或混合部署）

| 模块 | 方法数 | 理由 |
|------|--------|------|
| wsf_space | 1,597 | 轨道计算有持续状态和事件循环，升级为 Skill |
| wsf_cyber | 500 | 网络战效果，被动响应，适合 Service |
| sensor_plot | 145 | 可视化组件，需要 UI 绑定 |
| mystic | 1,466 | 后处理器有自己的 UI 和分析流程，升级为 Skill |

#### 不适合直接 Skill 集成（需 Tool/Service/Adapter 封装）

| 模块 | 方法数 | 障碍类型 |
|------|--------|---------|
| dis | 5,576 | 协议栈，有状态 UDP 连接 |
| util | 4,034 | 纯计算库，无 UI 无状态 |
| genio | 984 | I/O 抽象，序列化管道 |
| wsf_l16 | 673 | 战术数据链，有状态协议 |
| wsf_mil | 3,555 | 军事领域逻辑，状态复杂 |
| eventout | ~100 | 事件流管道 |
| wsfcgi | ~50 | Web 协议桥接 |
| geodata | 367 | 只读数据仓库 |

---

## 2. 8 类组件谱系

### 2.1 定义与特征矩阵

| 维度 | Tool | Adapter | Driver | Service | Module | Plugin | Pipeline | Repository |
|------|------|---------|--------|---------|--------|--------|----------|-----------|
| **状态** | 无 | 无 | 连接 | 连接 | 持久 | 共享宿主 | 无 | 只读缓存 |
| **生命周期** | 无 | 无 | open→close | start→stop | init→destroy | 随宿主 | build→run | load→unload |
| **驱动模式** | 被动 | 流式 | 主动 | 被动 | 主动 | 回调 | 数据流 | 被动 |
| **独立性** | 独立 | 嵌入 | 独立 | 独立 | 独立 | 嵌入 | 嵌入 | 独立 |
| **部署形态** | WASM/FFI | 嵌入宿主 | 独立进程 | MCP子进程 | Skill | Skill子目录 | Action Chain | MCP Resource |
| **网络需求** | 无 | 无 | 必须 | 可选 | 可选 | 无 | 无 | 可选 |
| **健康检查** | 不需要 | 不需要 | 必须 | 必须 | 必须 | 不需要 | 不需要 | 可选 |
| **故障恢复** | 重试 | 跳过 | 重连 | 重启 | 场景回滚 | 降级 | 重放 | 重新加载 |
| **典型延迟** | < 1ms | < 0.1ms | 5-30ms | 5-30ms | < 1ms | < 0.1ms | 各阶段之和 | 1-50ms |

### 2.2 核心区分原则

- **Tool vs Adapter**: Tool 是离散的"调用-返回"，Adapter 是连续的"流-转换"
- **Service vs Driver**: Service 被动等待调用，Driver 主动维护连接并持续工作
- **Module vs Plugin**: Module 独立运行有完整生命周期，Plugin 嵌入宿主共享状态
- **Tool vs Pipeline**: Tool 是单步操作，Pipeline 是多阶段有序链
- **Tool vs Repository**: Tool 是"计算"产生结果，Repository 是"查询"已有数据

### 2.3 AxAgent 对应机制

| 组件类型 | AxAgent 对应机制 | 注册方式 | 调用方式 |
|---------|-----------------|---------|---------|
| Tool | `actionRouter.invoke` | `DistributedToolRegistry.register_embedded()` | 声明式 Action `invoke` |
| Adapter | `skillEventBus` + `actionRouter.emit` | Skill manifest lifecycle 钩子 | 自动挂载在事件流上 |
| Driver | `TransportHandler` + `MessageGateway` | `MessageGateway.register_transport()` | `MessageGateway.send_message()` |
| Service | `McpToolRegistry` | `McpToolRegistry.register_server()` | `McpToolRegistry.call_tool()` |
| Module | Skill (iframe sandbox) | `skillStore.createSkill()` | Capability 系统 |
| Plugin | Skill Capability + 子 Skill | 宿主 Skill manifest plugins 字段 | 宿主 Action Chain 回调 |
| Pipeline | `actionRouter.chain` | Skill manifest pipelines 字段 | 声明式 Action `chain` |
| Repository | `McpToolRegistry` Resource | `McpToolRegistry.register_server()` + resources | `McpToolRegistry.read_resource()` |

---

## 3. 能力环补全策略

不适合直接 Skill 集成的模块，通过 Tool/Service/Driver/Adapter/Repository 五层封装间接暴露能力。

### 3.1 封装策略

| 模块 | 障碍类型 | 封装策略 | 理由 |
|------|---------|---------|------|
| util (UtEarth/UtMath/UtUnits/UtVec3/UtTable) | 纯计算、无状态 | Tool 提取 | 无副作用纯函数 |
| dis (DisIO/DisPdu) | 协议栈、有状态连接 | Service + Driver | 需要维护 UDP 连接和 PDU 编解码 |
| genio (GenIO/GenI/GenO/GenMsg) | I/O 抽象、序列化 | Adapter 适配 | 数据管道中间层 |
| wsf_l16 (Interface/Message) | 战术数据链协议 | Service + Driver | 有状态协议，需要消息队列 |
| wsf_mil (WsfMil) | 军事领域逻辑 | Service 封装 | 涉及平台/传感器/武器管理 |
| eventout (EventOut) | 事件流管道 | Adapter 适配 | 将 AFSIM 事件流转为 Skill 事件总线格式 |
| wsfcgi (Wsfcgi) | Web 协议 | Adapter 适配 | HTTP/CGI 协议转换 |
| geodata | 只读数据 | Repository | 高程/土地利用/水深数据查询 |

### 3.2 能力环补全路径

```
AFSIM C++ 底层模块
    │
    ├── Tool 提取（无状态纯函数）
    │   ├── UtEarth → earth.coordinates, earth.gravity, earth.atmosphere
    │   ├── UtMath → math.linear, math.rotation, math.interpolation
    │   ├── UtUnits → units.conversion, units.angle, units.speed
    │   ├── UtVec3/UtMat3 → math.linear
    │   └── UtTable → math.interpolation
    │
    ├── Service + Driver 封装（有状态协议/引擎）
    │   ├── dis → dis_driver (Driver) + dis_gateway (Service)
    │   ├── wsf_l16 → l16_driver (Driver) + l16_gateway (Service)
    │   ├── wsf_mil → mil_domain (Service)
    │   ├── wsf_mtt → mtt_tracker (Service)
    │   └── wsf_cyber → cyber_service (Service)
    │
    ├── Adapter 适配（数据格式转换）
    │   ├── genio → genio_serializer, genio_deserializer
    │   ├── eventout → eventout_bridge
    │   └── wsfcgi → wsfcgi_adapter
    │
    └── Repository（只读数据查询）
        └── geodata → geodata_elevation, geodata_landuse, geodata_bathymetry
```

---

## 4. 分布式 Skill 通信架构

### 4.1 现有通信基础设施

| 组件 | 位置 | 能力 | 限制 |
|------|------|------|------|
| MessageGateway | Rust Runtime | 可插拔传输(WS/SSE/HTTP/Stdio/IPC)、协议协商、消息路由 | 仅 Agent 级别 |
| SessionRouter | Rust Runtime | 平台会话路由、会话复用 | 仅平台→Agent 路由 |
| Swarm | Rust Runtime | 多 Agent 团队协作、权限同步、断线重连 | 仅进程间通信 |
| skillEventBus | 前端 | 命名空间隔离事件系统 | 仅进程内 |
| rpcBridge | 前端 | postMessage RPC、请求-响应模式 | 仅 iframe↔Host |
| storeRegistry | 前端 | Zustand Store 注册表 | 仅进程内 |
| actionRouter | 前端 | 7 种声明式执行器 + Agentic 模式 | 仅进程内 |

### 4.2 三层通信协议

#### 层 1: Skill → 本节点（已有）

- `rpcBridge`: iframe ↔ Host 的 postMessage RPC
- `skillEventBus`: 进程内事件总线，`skillName:event` 命名空间隔离
- `actionRouter`: 7 种声明式执行器 + Agentic 模式
- `storeRegistry`: 进程内 Zustand Store 读写

#### 层 2: Skill → 远端 Skill（需扩展 DistributedBridge）

在 `skillEventBus` 和 `MessageGateway` 之间插入 DistributedBridge：

```typescript
interface DistributedBridge {
  bridgeEventToRemote(skillName: string, event: string, payload: unknown): void;
  onRemoteEvent(handler: (from: string, skillName: string, event: string, payload: unknown) => void): void;
  invokeRemoteAction(targetNode: string, action: SkillCommandAction, context: ActionContext): Promise<ActionResult>;
  readRemoteStore(targetNode: string, storeName: string, selector?: string): Promise<unknown>;
}
```

传输层映射（复用 MessageGateway 的 TransportHandler trait）：

| 场景 | TransportType | Protocol | 说明 |
|------|--------------|----------|------|
| 同机房节点 | WebSocket | A2A | 低延迟双向通信 |
| 跨网络节点 | HTTP + SSE | MCP | 防火墙友好 |
| 同主机多实例 | IPC | Custom | 零网络开销 |
| 边缘设备 | Stdio | Custom | 嵌入式场景 |

#### 层 3: Store 跨节点同步（CRDT）

基于现有 `CrdtEngine` 的增量同步：

```
Node A Store 写入 → 本地 Zustand setState
                  → DistributedBridge.broadcastDelta(delta)
                                      │
Network ──────────────────────────────┤
                                      ▼
Node B Store 同步 ← DistributedBridge.applyDelta(delta)
                  → 本地 Zustand setState
```

### 4.3 通信模式与 Skill Action 映射

| 通信模式 | Skill Action 类型 | 分布式实现 |
|---------|------------------|-----------|
| 同步请求-响应 | `invoke` | `DistributedBridge.invokeRemoteAction()` → 远端 `actionRouter.execute()` |
| 异步事件通知 | `emit` | `skillEventBus.emit()` → `DistributedBridge.bridgeEventToRemote()` |
| 状态读写 | `store` | 本地走 `storeRegistry`，远端走 `DistributedBridge.readRemoteStore()` |
| 页面导航 | `navigate` | 仅本地生效（UI 操作不可跨节点） |
| 链式调用 | `chain` | 逐个执行，若含远端 Action 则透明路由 |

---

## 5. 实时性能保证

### 5.1 现有性能参数

| 组件 | 参数 | 默认值 |
|------|------|--------|
| rpcBridge | RPC 超时 | 15,000 ms |
| WebSocket | Ping 间隔 | 25,000 ms |
| WebSocket | Pong 超时 | 5,000 ms |
| WebSocket | 连接超时 | 10,000 ms |
| WebSocket | 最大重连次数 | 5 |
| WebSocket | 发送缓冲区 | 100 条 |
| HTTP | 请求超时 | 30,000 ms |
| HTTP | 最大重试 | 3 次 |
| HTTP | 重试延迟 | 1,000 ms（指数退避） |
| SSE | 重连延迟 | 2,000 ms |
| SSE | 最大重连次数 | 10 |
| Swarm | 心跳间隔 | 10,000 ms |
| Swarm | 心跳超时 | 30,000 ms |
| Swarm | 任务超时 | 600,000 ms (10min) |
| 连接池 | 最大连接数 | 10 |
| 连接池 | 获取超时 | 5,000 ms |
| 批处理 | 最大批次大小 | 100 条 |
| 批处理 | 最大批次延迟 | 50 ms |
| 批处理 | 队列容量 | 10,000 条 |
| 限流器 | 每秒请求数 | 10 req/s |
| 限流器 | 突发容量 | 20 tokens |
| 调度器 | 最大并发任务 | 4 |
| 调度器 | 饥饿阈值 | 30,000 ms |

### 5.2 端到端延迟分解

```
Skill A (iframe)
  │ ① postMessage RPC → Host A          ~0.1-0.5 ms
  ▼
Host A (前端进程)
  │ ② skillEventBus.emit()              ~0.01 ms
  │ ③ DistributedBridge 桥接             ~0.5-2 ms
  ▼
MessageGateway (Rust Runtime A)
  │ ④ RateLimiter.check()               ~0.1 ms
  │ ⑤ BatchProcessor (可选)             0-50 ms
  │ ⑥ TransportHandler.send()           ──────────────
  ▼                                        │
Network Transport                         │ 网络延迟
  │                                        │
  ▼                                        ▼
MessageGateway (Rust Runtime B)          ──────────────
  │ ⑦ 消息反序列化+路由                   ~0.1-0.5 ms
  │ ⑧ DistributedBridge 注入               ~0.5-2 ms
  ▼
Host B → Skill B (iframe)
  │ ⑨ skillEventBus.emit()              ~0.01 ms
  │ ⑩ postMessage RPC → Skill B          ~0.1-0.5 ms
  ▼
Skill B
```

### 5.3 各场景延迟保证

| 部署场景 | 网络延迟 | 批处理 | P50 | P99 | 可靠性 |
|---------|---------|--------|-----|-----|--------|
| 同主机 IPC | ~0.05 ms | 跳过 | 3-5 ms | 8-15 ms | 最多 5 次重连 |
| 同机房 WebSocket | 0.1-0.5 ms | 跳过 | 5-10 ms | 15-30 ms | Ping 25s + Pong 5s |
| 同机房 HTTP | 0.1-0.5 ms | 跳过 | 8-15 ms | 30-60 ms | 3 次重试 |
| 跨机房 WebSocket | 5-20 ms | 可选 | 15-30 ms | 50-100 ms | 持久队列 |
| 广域网 SSE | 20-100 ms | 开启 | 50-150 ms | 150-500 ms | 10 次重连 |

### 5.4 AFSIM 场景实时性评估

| AFSIM 场景 | 事件频率 | 推荐传输 | P99 延迟 | 能否保证 |
|-----------|---------|---------|---------|---------|
| DIS 实体位置更新 | 5-30 Hz/实体 | 同机房 WS | 15-30 ms | ✅ |
| DIS 批量实体同步 | 100+ Hz | WS + 批处理 | 50-100 ms | ✅ |
| Link16 战术消息 | 1-10 msg/s | 同机房 WS | 10-25 ms | ✅ |
| 武器交战事件 | < 1 Hz | 任意 | 5-50 ms | ✅ |
| 传感器探测事件 | 1-50 Hz | 同机房 WS | 10-30 ms | ✅ |
| 仿真状态快照 | 0.1-1 Hz | HTTP + CRDT | 100-500 ms | ✅ |
| 多目标航迹更新 | 10-100 Hz | WS + 批处理 | 30-80 ms | ⚠️ 需调优限流 |
| 仿真后处理 | 离线 | HTTP | 秒级 | ✅ |

### 5.5 性能瓶颈与优化

| 瓶颈 | 当前值 | 建议值 | 影响 |
|------|--------|--------|------|
| 限流器默认 10 req/s | 10 | 100 (Skill 通道) | DIS PDU 每秒可达数百条 |
| 批处理 50ms 窗口 | 50 ms | 实时通道跳过 | 实时场景增加 50ms 延迟 |
| WebSocket 发送缓冲区 100 条 | 100 | 1,000 (DIS 场景) | 高频仿真容易背压 |
| CRDT 无向量时钟 | 递增 ID | HLC | 并发写入可能丢失顺序 |

---

## 6. Tool 提取与部署

### 6.1 Tool 全局命名规范

```
{domain}.{subdomain}.{function}

示例:
earth.coordinates.geodetic_to_geocentric
earth.atmosphere.get_density
math.linear.dot_product
units.conversion.convert
dis.entity.serialize
l16.message.encode_j_series
mil.weapon.get_max_range
```

### 6.2 Tool 清单（23 域，~1,105 方法）

| 工具域 | 来源模块 | 方法数 | 示例方法 |
|--------|---------|--------|---------|
| earth.coordinates | UtEarth | ~40 | geodetic_to_geocentric, great_circle_distance, geodesic_distance |
| earth.gravity | UtEarth | ~10 | get_gravity, get_gravity_acceleration |
| earth.magnetic | UtEarth | ~8 | get_magnetic_field |
| earth.atmosphere | UtEarth | ~15 | get_density, get_pressure, get_temperature, get_speed_of_sound |
| math.linear | UtVec3, UtVec2, UtMat3 | ~120 | dot_product, cross_product, normalize, magnitude, determinant |
| math.rotation | UtDCM, UtQuat | ~60 | multiply, transpose, rotate, extract_euler_angles |
| math.interpolation | UtMath, UtTable | ~30 | linear_interpolate, cubic_interpolate, lookup |
| math.statistics | UtMath | ~15 | mean, standard_deviation, correlation |
| math.noise | UtNoise | ~20 | gaussian_random, uniform_random, perlin_noise |
| units.conversion | UtUnits | ~60 | convert, convert_to_standard, convert_from_standard |
| units.angle | UtAngle | ~25 | convert, normalize, difference |
| units.speed | UtSpeed | ~15 | convert, mach_to_speed, speed_to_mach |
| units.time | UtTime | ~20 | convert, format, parse |
| dis.encoding | DisPdu 系列 | ~200 | serialize, deserialize, get_pdu_length |
| dis.entity | DisEntityId 系列 | ~50 | get_entity, set_entity, compare |
| dis.radio | DisRadio 系列 | ~80 | set_transmitter_frequency, set_radio_type |
| dis.warfare | DisWarfare 系列 | ~100 | set_fuse_type, set_warhead_type |
| l16.encoding | Interface/Message | ~30 | encode_j_series, decode_j_series, validate_message |
| weapon.ballistic | weapon_tools | ~40 | ballistic_launch_compute, trajectory_calculate |
| weapon.guidance | weapon_tools | ~72 | guidance_gain_compute, proportional_nav |
| geo.elevation | geodata | ~30 | query_elevation, query_slope, query_aspect |
| geo.landuse | geodata | ~20 | query_land_use, query_land_cover |
| geo.bathymetry | geodata | ~15 | query_depth, query_sea_floor |
| script.engine | util_script | ~200 | execute, compile, get_variable |

### 6.3 Tool 路由协议

扩展 `actionRouter` 的 `invoke` executor 实现三级路由：

```
① 本地嵌入式 Tool (WASM/FFI)
   → localToolRegistry.has(command) → 直接调用

② 本地 MCP 服务 Tool
   → mcpToolRegistry.hasTool(command) → MCP stdio 调用

③ 远程 Tool (通过 MessageGateway)
   → toolRouter.resolve(command) → targetNode
   → DistributedBridge.invokeRemoteTool(targetNode, command, args)
```

### 6.4 节点上线 Tool 广播协议

```
节点 A 启动
  ├── 1. 加载本地嵌入式 Tools (util → WASM)
  ├── 2. 连接 MCP 服务 Tools (dis → MCP stdio)
  └── 3. 广播 Tool 清单
        MessageGateway.broadcast(AgentMessage {
            payload: Event {
                name: "tool:announce",
                params: { node_id, tools: ["earth.*", "math.*"], capabilities }
            }
        })

其他节点收到
  └── on "tool:announce" → register_remote_node(node_id, tools)
```

---

## 7. Service 部署方案

### 7.1 服务生命周期管理

新增 `ServiceOrchestrator`，基于现有 `McpToolRegistry` 和 `HealthCheckRegistry`：

```
启动流程:
  ① register_server(name, Connecting, [], [])
  ② 启动 MCP stdio 子进程
  ③ 等待 ReadinessCheck 就绪
  ④ set_auth_status(name, Connected)
  ⑤ 广播 service:online 事件

关闭流程:
  ① set_auth_status(name, Disconnected)
  ② shutdown MCP stdio 子进程
  ③ disconnect(name)
  ④ 广播 service:offline 事件
```

### 7.2 服务依赖图与启动顺序

```
启动层 0 (无依赖):
  └── compute_tools (WASM 嵌入式)

启动层 1 (基础协议):
  ├── dis_gateway
  ├── l16_gateway
  └── cyber_service

启动层 2 (领域服务):
  ├── mil_domain      (依赖: dis_gateway)
  ├── mtt_tracker     (依赖: dis_gateway)
  └── space_domain

启动层 3 (仿真引擎):
  ├── afsim-engine    (依赖: mil_domain, dis_gateway, compute_tools)
  ├── engage          (依赖: afsim-engine, mil_domain)
  └── mission         (依赖: afsim-engine, mil_domain)
```

### 7.3 服务故障恢复

```
HealthMonitor 定时检查 (每 10s)
  │
  ├── LivenessCheck 失败 → 子进程已死
  │   ├── 1. set_auth_status(name, Error)
  │   ├── 2. queue_message() 缓存发往该服务的消息
  │   ├── 3. 重启 MCP stdio 子进程
  │   ├── 4. 等待 ReadinessCheck 就绪
  │   ├── 5. set_auth_status(name, Connected)
  │   └── 6. flush_queue() 重放缓存消息
  │
  └── ReadinessCheck 失败 → 服务未就绪
      ├── 1. set_auth_status(name, Connecting)
      └── 2. 等待恢复或超时后重启
```

### 7.4 服务降级策略

| 服务 | 降级策略 | 实现方式 |
|------|---------|---------|
| dis_gateway | 本地回环模式 | PDU 不发送到网络，仅本地记录 |
| l16_gateway | 离线模式 | Link16 消息仅记录不发送 |
| mil_domain | 缓存模式 | 使用最近一次查询结果快照 |
| space_domain | 简化模式 | 二体轨道近似代替高精度轨道 |
| mtt_tracker | 手动模式 | 航迹由操作员手动输入 |
| cyber_service | 跳过 | 网络战效果不生效 |

---

## 8. 完整部署拓扑

### 8.1 按域分节点部署（生产环境推荐）

```
┌─────────────────────────────────────────────────────────────────────┐
│                     AxAgent 分布式集群                                │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Compute-1 (每个仿真节点都部署)                         │   │
│  │  嵌入式 Tools: earth.*, math.*, units.*, geo.*, weapon.*     │   │
│  │  Skills: engage, mission, wizard                              │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: DIS-Gateway (1-2 实例)                                 │   │
│  │  Driver: dis_driver (UDP 多播)                                │   │
│  │  Adapter: genio_serializer, genio_deserializer                │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: L16-Gateway (1 实例)                                   │   │
│  │  Driver: l16_driver (JTIDS)                                   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Mil-Domain (1-2 实例)                                  │   │
│  │  Service: mil_domain, classification_service                  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Space-Domain (1 实例)                                  │   │
│  │  Service: space_domain                                        │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Cyber-MTT (1 实例)                                     │   │
│  │  Service: cyber_service, mtt_tracker, fusion_center           │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Sim-Engine (1-2 实例)                                  │   │
│  │  Module: afsim-engine, engage, mission                        │   │
│  │  Plugin: wsf_brawler, wsf_air_combat, wsf_iads_c2, ...       │   │
│  │  Pipeline: sensor_detection, weapon_engagement, mission_plan │   │
│  │  Adapter: eventout_bridge, wsfcgi_adapter                    │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Wizard (1 实例)                                        │   │
│  │  Module: wizard                                               │   │
│  │  Plugin: ScenarioAnalyzer, TaskList, MysticLauncher          │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Mystic (1 实例)                                        │   │
│  │  Module: mystic                                               │   │
│  │  Plugin: ResultTimeController, TaskAssigner, Log             │   │
│  │  Pipeline: post_processing                                   │   │
│  │  Adapter: mystic_result_adapter, warlock_event_adapter       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Node: Data-Repository (1 实例)                               │   │
│  │  Repository: geodata_elevation, geodata_landuse,             │   │
│  │             geodata_bathymetry, geodata_imagery,              │   │
│  │             result_db, project_workspace                       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  通信: MessageGateway (WS/HTTP/IPC) + DistributedToolRegistry       │
└─────────────────────────────────────────────────────────────────────┘
```

### 8.2 节点进程数汇总

| 节点 | 部署的组件 | 进程数 |
|------|-----------|--------|
| Compute Node | Tool (WASM), Adapter (嵌入) | 1 |
| DIS-Gateway | dis_driver, genio Adapter | 2 |
| L16-Gateway | l16_driver | 2 |
| Mil-Domain | mil_domain, classification_service | 3 |
| Space-Domain | space_domain | 2 |
| Cyber-MTT | cyber_service, mtt_tracker, fusion_center | 4 |
| Sim-Engine | afsim-engine, engage, mission + Plugin/Adapter/Pipeline | 1 + N iframe |
| Wizard | wizard + Plugin | 1 + N iframe |
| Mystic | mystic + Plugin/Adapter/Pipeline | 1 + N iframe |
| Data-Repository | 6 个 Repository | 2 |

---

## 9. 模块与组件完整对应关系

### 9.1 AFSIM 核心仿真循环时间步分析

#### 9.1.1 WsfFrameStepSimulation::AdvanceFrame 源码级调用顺序

根据 [WsfFrameStepSimulation.cpp:111-199](file:///d:/Programe/Afsim 2.9/afsim-2.9.0-win64/swdev/src/core/wsf/source/WsfFrameStepSimulation.cpp#L111)，每个仿真帧的执行顺序如下：

```
WsfFrameStepSimulation::AdvanceFrame()
│
├── ① FrameStarting 观察者通知
│     WsfObserver::FrameStarting(this)(currentFrameTime)
│
├── ② Platform 更新 (所有平台串行或并行)
│     if (MultiThreaded()) {
│         GetMultiThreadManager().UpdatePlatforms(currentFrameTime)
│     } else {
│         for (i = 0; i < GetPlatformCount(); ++i)
│             GetPlatformEntry(i)->Update(currentFrameTime)
│     }
│     WsfObserver::FramePlatformsUpdated(this)(currentFrameTime)
│     │
│     │ WsfPlatform::Update() 内部调用链：
│     │   ├── WsfMover::Update()
│     │   │   └── [WsfBrawlerMover::Update()] ← 空战机动 (Plugin)
│     │   │       └── UtEarth::GetGravityAcceleration() ← 调用 Tool
│     │   │       └── UtVec3::Distance/Magnitude()
│     │   ├── WsfWeapon::Update()
│     │   │   └── [WsfLaunchComputer::Compute()] ← 发射计算
│     │   │       └── UtMath::LinearInterpolate()
│     │   │       └── UtVec3::Distance()
│     │   └── WsfComm::Update()
│     │
│     └── WsfObserver::FramePlatformsUpdated
│
├── ③ Comm 子系统更新
│     for (i = 0; i < mComms.size(); ++i)
│         mComms[i]->Update(currentFrameTime)
│     │ WsfComm::Update() 内部调用链：
│     │   ├── Link16 消息处理
│     │   └── 通信网络路由
│
├── ④ Processor 子系统更新 ← Plugin 主要注入点
│     for (i = 0; i < mProcessors.size(); ++i)
│         mProcessors[i]->Update(currentFrameTime)
│     │
│     │ WsfProcessor::Update() 调用链：
│     ├── [WsfBrawlerProcessor::Update()] ← wsf_brawler Plugin
│     │     ├── WsfThreatProcessor::Update()
│     │     ├── WsfPerceptionProcessor::Update()
│     │     ├── WsfSensor::Update()
│     │     └── WsfWeapon::Update()
│     ├── [WsfSA_Processor::Update()] ← wsf_air_combat Plugin
│     │     ├── WsfSensor::Update() ← 探测
│     │     └── WsfTrackManager::Update() ← 航迹关联
│     ├── [WsfBattleManager::Update()] ← wsf_iads_c2 Plugin
│     │     ├── WsfWeapon::SelectTarget()
│     │     └── WsfComm::SendMessage()
│     └── [WsfClassificationProcessor::Update()] ← 目标分类
│
├── ⑤ Sensor 子系统更新
│     if (MultiThreaded()) {
│         GetMultiThreadManager().UpdateSensors(currentFrameTime)
│     } else {
│         for (i = 0; i < mSensors.size(); ++i)
│             mSensors[i]->Update(currentFrameTime)
│     }
│     │
│     │ WsfSensor::Update() 调用链：
│     ├── WsfRadarSensor::Update()
│     │     ├── UtNoise::GaussianRandom() ← 噪声注入
│     │     └── UtMath::Probability() ← 检测概率
│     ├── WsfIRST_Sensor::Update()
│     └── WsfCompositeSensor::Update()
│
├── ⑥ AdvanceFrameObjects
│     │ 其他对象更新（环境、天气等）
│
└── ⑦ 事件调度
      while (peekEventPtr && (peekEventPtr->GetTime() < mNextFrameTime)) {
          eventPtr->Execute()
      }
```

#### 9.1.2 仿真帧时间预算分析

| 仿真帧率 | 每帧时间预算 | 说明 |
|---------|------------|------|
| 60 Hz | 16.67 ms | 实时仿真 (硬件在环) |
| 30 Hz | 33.33 ms | 高保真仿真 |
| 10 Hz | 100 ms | 低保真/快速仿真 |
| 1 Hz | 1000 ms | 慢速仿真 |

**关键约束**: 若任何 Tool 调用产生 5-30ms 网络延迟，对于 60Hz 仿真，每次延迟都会直接导致帧超时。

#### 9.1.3 每帧 Tool 调用频率实测估算

基于源码分析，每帧调用频率如下：

| 调用方 | Tool 方法 | 每帧每实例次数 | 平台数 | 总次数/帧 |
|--------|----------|--------------|-------|---------|
| WsfBrawlerMover | UtEarth::GetGravityAcceleration | 5 | ×100 | 500 |
| WsfBrawlerMover | UtVec3::Magnitude | 10 | ×100 | 1,000 |
| WsfBrawlerMover | UtVec3::Distance | 5 | ×100 | 500 |
| WsfRadarSensor | UtNoise::GaussianRandom | 3 | ×500 | 1,500 |
| WsfWeapon | UtMath::LinearInterpolate | 2 | ×200 | 400 |
| WsfLaunchComputer | UtVec3::Distance | 50 | ×20导弹 | 1,000 |
| WsfDCM | UtDCM::Multiply | 8 | ×100 | 800 |

**总计**: 每帧约 5,700 次 Tool 调用，若全部远端化 (5ms/调用)，需要 28,500ms = 28.5 秒完成一帧 → 完全不可行。

---

### 9.2 Tool-Module 精确对应关系矩阵

#### 9.2.1 afsim-engine Module → Tool 调用矩阵

| Tool 域 | 具体方法 | 调用源码位置 | 调用频率/帧 | 调用耗时 | 能否远端 | 远端化后果 |
|---------|---------|------------|-----------|---------|---------|-----------|
| **earth.coordinates** | | | | | | |
| | UtEarth::GetGravityAcceleration | WsfBrawlerMover::Update L89 | 每平台 5 次 | ~0.01ms | ❌ | 5×100平台×5ms=2500ms |
| | UtEarth::GetDensity | WsfBrawlerMover::Update L102 | 每平台 3 次 | ~0.005ms | ❌ | 3×100平台×5ms=1500ms |
| | UtEarth::GetPressure | 大气模型 | 每平台 2 次 | ~0.005ms | ❌ | 2×100×5ms=1000ms |
| | UtEarth::GetTemperature | 大气模型 | 每平台 2 次 | ~0.005ms | ❌ | 2×100×5ms=1000ms |
| | UtEarth::GetSpeedOfSound | 声速计算 | 每平台 1 次 | ~0.005ms | ❌ | 100×5ms=500ms |
| | UtEarth::ConvertGeodeticToGeocentric | WsfMover::Update | 每平台 2 次 | ~0.02ms | ❌ | 200×5ms=1000ms |
| | UtEarth::GreatCircleDistance | WsfRouteMover | 路径规划时 | ~0.1ms | ✅ | 延迟容忍 100ms |
| | UtEarth::GeodesicDistance | WsfRouteMover | 路径规划时 | ~0.1ms | ✅ | 延迟容忍 100ms |
| **math.linear** | | | | | | |
| | UtVec3::Magnitude | WsfMover::Update | 每平台 20 次 | ~0.001ms | ❌ | 2000×5ms=10000ms |
| | UtVec3::Distance | WsfWeapon::Update | 每武器 5 次 | ~0.001ms | ❌ | 1000×5ms=5000ms |
| | UtVec3::DotProduct | WsfBrawlerMover | 每平台 10 次 | ~0.001ms | ❌ | 1000×5ms=5000ms |
| | UtVec3::CrossProduct | WsfBrawlerMover | 每平台 5 次 | ~0.001ms | ❌ | 500×5ms=2500ms |
| | UtVec3::Normalize | WsfBrawlerMover | 每平台 10 次 | ~0.001ms | ❌ | 1000×5ms=5000ms |
| | UtMat3::Multiply | WsfMover::Update | 每平台 8 次 | ~0.001ms | ❌ | 800×5ms=4000ms |
| | UtMat3::Determinant | 初始化时 | 0 次/帧 | ~0.01ms | ❌ | 初始化时无影响 |
| **math.rotation** | | | | | | |
| | UtDCM::Multiply | WsfMover::Update | 每平台 8 次 | ~0.001ms | ❌ | 800×5ms=4000ms |
| | UtDCM::Transpose | WsfMover::Update | 每平台 3 次 | ~0.001ms | ❌ | 300×5ms=1500ms |
| | UtDCM::Rotate | 向量旋转 | 每平台 5 次 | ~0.001ms | ❌ | 500×5ms=2500ms |
| | UtDCM::ToEulerAngles | WsfWeapon | 每平台 3 次 | ~0.001ms | ❌ | 300×5ms=1500ms |
| | UtQuat::Multiply | WsfBrawlerMover | 每平台 5 次 | ~0.001ms | ❌ | 500×5ms=2500ms |
| | UtQuat::ToDCM | WsfBrawlerMover | 每平台 2 次 | ~0.001ms | ❌ | 200×5ms=1000ms |
| **math.interpolation** | | | | | | |
| | UtMath::LinearInterpolate | WsfLaunchComputer | 发射时 20 次 | ~0.01ms | ❌ | 发射时不可接受 |
| | UtTable::Lookup | 性能表查询 | 配置时 | ~0.005ms | ✅ | 非实时 |
| **math.noise** | | | | | | |
| | UtNoise::GaussianRandom | WsfRadarSensor | 每传感器 3 次 | ~0.005ms | ❌ | 1500×5ms=7500ms |
| **units.conversion** | | | | | | |
| | UtUnits::Convert | WsfSensor::Update | 每传感器 5 次 | ~0.001ms | ❌ | 2500×5ms=12500ms |
| | UtSpeed::MachToSpeed | WsfMover | 每平台 1 次 | ~0.001ms | ❌ | 100×5ms=500ms |
| **units.angle** | | | | | | |
| | UtAngle::Convert | WsfMover | 每平台 5 次 | ~0.001ms | ❌ | 500×5ms=2500ms |
| | UtAngle::Normalize | WsfWeapon | 每武器 2 次 | ~0.001ms | ❌ | 400×5ms=2000ms |

**结论**: afsim-engine Module 内的所有 earth.*、math.*、units.* Tool 必须嵌入式部署，不能远端化。

#### 9.2.2 engage Module → Tool 调用矩阵

| Tool 域 | 具体方法 | 调用源码位置 | 调用频率 | 调用耗时 | 能否远端 | 远端化后果 |
|---------|---------|------------|---------|---------|---------|-----------|
| **weapon.ballistic** | | | | | | |
| | BallisticLaunchComputer::Compute | WsfWeapon::Fire L45 | 发射时 1 次 | ~0.5ms | ❌ | 发射延迟不可接受 |
| | TrajectoryCalculator::Propagate | 每帧每导弹 | 每帧 50-200 次 | ~1ms/帧 | ❌ | 20导弹×200×5ms=20000ms |
| | AirToAirLaunchComputer::Compute | WsfLaunchComputer L78 | 发射时 1 次 | ~0.3ms | ❌ | 不可接受 |
| **weapon.guidance** | | | | | | |
| | GuidanceGainComputer::Compute | WsfGuidanceComputer | 每帧每导弹 | ~0.2ms | ❌ | 20导弹×5ms=100ms |
| | ProportionalNavigation::Compute | WsfGuidanceComputer | 每帧每导弹 10 次 | ~0.1ms | ❌ | 200×5ms=1000ms |
| **math.linear** | | | | | | |
| | UtVec3::Distance | engage决策 | 每交战 10 次 | ~0.001ms | ❌ | 交战频率 1Hz，暂可 |
| | UtVec3::AngleBetween | engage决策 | 每交战 5 次 | ~0.001ms | ❌ | 同上 |
| | UtMat3::Rotate | 弹道计算 | 每交战 5 次 | ~0.001ms | ❌ | 同上 |
| **mil.weapon** (通过 Service) | | | | | | |
| | mil_domain::get_max_range | WsfWeapon::SelectTarget | 目标选择时 | ~5ms | ✅ | 1Hz，容忍 100ms |
| | mil_domain::get_probability_of_hit | 交战评估 | 命中时 | ~5ms | ✅ | 低频，可接受 |

**结论**: weapon.* Tool 必须嵌入式，mil.weapon 可通过 Service 调用（低频）。

#### 9.2.3 mission Module → Tool/Repository 调用矩阵

| Tool/Repository | 具体方法 | 调用场景 | 调用频率 | 调用耗时 | 能否远端 | 远端化后果 |
|----------------|---------|---------|---------|---------|---------|-----------|
| **math.linear** (嵌入式) | | | | | | |
| | UtVec3::Distance | A* 路径搜索 | 每节点 1 次 | ~0.01ms | ❌ | A*搜索 10000 节点×5ms=50000ms |
| | UtVec3::Normalize | 向量归一化 | 每节点 1 次 | ~0.001ms | ❌ | 同上 |
| **geo.elevation** (Repository) | | | | | | |
| | GeoTileManager::QueryElevation | WsfRouteMover::PlanPath | 每路径 5-20 次 | ~1ms+IO | ✅ | 路径规划秒级，容忍 100ms |
| | GeoTileManager::QuerySlope | WsfRouteMover | 每路径 5 次 | ~1ms+IO | ✅ | 同上 |
| **space.orbital** (Repository/Service) | | | | | | |
| | WsfOrbitalPropagation::Propagate | 轨道任务 | 每积分步 | ~5ms/步 | ❌ | 必须嵌入式 via wsf_space |

**结论**: mission 的路径规划部分可通过 Repository 远端化，但 math.linear 必须嵌入式。

#### 9.2.4 wsf_space Module → Tool 调用矩阵

| Tool 域 | 具体方法 | 调用频率 | 调用耗时 | 能否远端 |
|---------|---------|---------|---------|---------|
| **space.orbital** | | | | |
| | WsfOrbitalPropagation::Propagate | 每积分步 (高轨 30s/步) | ~5ms/步 | ❌ 嵌入式 |
| | WsfTwoBodyPropagator::Step | 每积分步 | ~1ms/步 | ❌ 嵌入式 |
| **earth.gravity** | | | | |
| | UtEarth::GetGravityAcceleration | 每积分步 | ~0.01ms | ❌ 嵌入式 |
| **math.interpolation** | | | | |
| | UtMath::LinearInterpolate | 数值积分 | 每积分步 10 次 | ❌ 嵌入式 |

**结论**: wsf_space 的轨道计算工具必须嵌入式。

#### 9.2.5 mystic Module → Tool 调用矩阵

| Tool 域 | 具体方法 | 调用场景 | 调用频率 | 调用耗时 | 能否远端 |
|---------|---------|---------|---------|---------|---------|
| **math.statistics** | | | | | |
| | UtMath::Mean | 统计分析 | 分析时 | ~0.1ms | ✅ 离线分析 |
| | UtMath::StandardDeviation | 统计分析 | 分析时 | ~0.1ms | ✅ 离线分析 |
| | UtMath::Correlation | 相关性分析 | 分析时 | ~0.5ms | ✅ 离线分析 |
| **earth.coordinates** | | | | | |
| | UtEarth::ConvertGeodeticToGeocentric | 可视化 | 渲染时 | ~0.02ms | ✅ 可远端 |

**结论**: mystic 的 Tool 调用均为离线分析，无实时性要求。

---

### 9.3 不可分割模块组确定（基于实时性约束）

#### 9.3.1 不可分割组 1：afsim-engine Core + Embedded Tool

```
组成成员：
  ├── afsim-engine Module (Skill)
  ├── embedded Tool: earth.coordinates (UtEarth::GetGravityAcceleration 等 7 方法)
  ├── embedded Tool: earth.gravity (UtEarth::GetGravityModel)
  ├── embedded Tool: earth.atmosphere (UtEarth::GetDensity/Pressure/Temperature)
  ├── embedded Tool: math.linear (UtVec3::Magnitude/Distance/DotProduct 等 8 方法)
  ├── embedded Tool: math.rotation (UtDCM/UtQuat 系列 7 方法)
  ├── embedded Tool: math.interpolation (UtMath::LinearInterpolate)
  ├── embedded Tool: math.noise (UtNoise::GaussianRandom)
  ├── embedded Tool: units.conversion (UtUnits::Convert 等 3 方法)
  ├── embedded Tool: units.angle (UtAngle::Convert/Normalize)
  └── embedded Adapter: eventout_bridge

实时性约束分析：
  ├── 每帧总 Tool 调用次数: ~5,700 次
  ├── 单次远端调用延迟: 5-30ms (保守取 10ms)
  ├── 若远端化总延迟: 5,700 × 10ms = 57,000ms = 57 秒/帧
  ├── 60Hz 帧时间: 16.67ms
  ├── 帧超时率: 57s ÷ 16.67ms = 3418% 超时
  └── 结论: 完全不可接受，必须嵌入式

部署要求：
  ├── 必须与 compute_tools 节点共部署
  ├── 推荐使用共享内存 IPC (延迟 < 0.1ms)
  └── 即使分离部署，也必须通过本地 WASM/FFI 调用
```

#### 9.3.2 不可分割组 2：engage + weapon.* Tool

```
组成成员：
  ├── engage Module (Skill)
  ├── embedded Tool: weapon.ballistic (BallisticLaunchComputer 等)
  ├── embedded Tool: weapon.guidance (GuidanceGainComputer, ProportionalNavigation)
  ├── embedded Pipeline: weapon_engagement
  └── embedded Adapter: sensor_plot_adapter

实时性约束分析：
  ├── TrajectoryCalculator::Propagate 每帧每导弹 50-200 次迭代
  ├── 单次迭代涉及: UtVec3 运算 + UtMath 插值
  ├── 弹道积分必须在 < 1ms 内完成（否则影响制导实时性）
  ├── 发射计算 BallisticLaunchComputer::Compute 必须 < 0.5ms
  └── 若 weapon.* Tool 远端化：每次发射延迟 5-30ms → 不可接受

部署要求：
  ├── engage Module + weapon.* Tool 必须同节点
  ├── sensor_plot_adapter 必须与 engage 同节点（可视化紧耦合）
  └── 可通过共享内存与 afsim-engine Core 通信
```

#### 9.3.3 不可分割组 3：wsf_space + space.* Tool

```
组成成员：
  ├── wsf_space Module (Skill) ← 升级自 Service
  ├── embedded Tool: space.orbital (轨道积分器)
  ├── embedded Tool: earth.gravity (重力模型)
  └── embedded Tool: math.interpolation (数值积分)

实时性约束分析：
  ├── 轨道积分每步 5ms（高精度 RK78）
  ├── 高轨卫星积分步长 30s，低轨卫星积分步长 1-5s
  ├── 轨道计算不可中断，必须连续完成
  ├── 若远端化：积分步中断导致轨道精度丧失
  └── 每积分步内部 10-50 次 earth.*/math.* 调用 → 必须嵌入式

部署要求：
  ├── wsf_space Module + space.* Tool 必须嵌入式
  └── 可与 afsim-engine Core 合并部署（共享 earth.* Tool）
```

#### 9.3.4 不可分割组 4：sensor_detection Pipeline

```
组成成员：
  ├── Pipeline: sensor_detection (4 阶段)
  ├── embedded Tool: math.noise (UtNoise::GaussianRandom)
  ├── embedded Tool: math.statistics (概率计算)
  └── Service: mtt_tracker (航迹关联 - 异步模式)

实时性约束分析：
  ├── 探测判决必须在帧内完成（否则下一帧无法使用）
  ├── 噪声注入 WsfRadarSensor::Update() 每帧 3 次
  ├── 若 mtt_tracker 远端化：航迹关联延迟 5-30ms
  ├── 优化方案：WsfSensor::OnDetection() → 本地缓冲 → 批量异步发送
  └── N=10 帧批量：延迟增加 167ms (60Hz)，可接受

部署要求：
  ├── sensor_detection Pipeline 必须在 afsim-engine Core 内
  ├── mtt_tracker 可分离，但必须使用异步批量模式
  └── 降级策略：本地简化 KalmanFilter（若 mtt_tracker 不可用）
```

---

### 9.4 可分割模块组（可分布式部署）

#### 9.4.1 可分割组 1：DIS Gateway

```
组成成员：
  ├── dis_driver (独立进程，UDP 多播维护)
  ├── dis_gateway (MCP Service)
  ├── genio_serializer Adapter
  ├── genio_deserializer Adapter
  └── embedded Tool: dis.encoding, dis.entity, dis.radio, dis.warfare

实时性约束分析：
  ├── DIS EntityState PDU 发送频率: 5-20 Hz/实体
  ├── 每次 PDU 发送涉及: DisPdu::Serialize() → UDP 发送
  ├── DIS 协议本身延迟容忍: 50-100ms
  ├── PDU 发送与仿真帧解耦（异步发送）
  └── 外部 DIS 实体接收后通过 eventout_bridge 注入

分离可行性：
  ├── ✅ DIS 是松耦合协议（UDP，无连接）
  ├── ✅ PDU 发送/接收与仿真帧解耦
  ├── ✅ 即使 DIS 延迟 100ms，仿真引擎可缓冲处理
  └── ✅ 50ms 容忍度远大于网络延迟 5-30ms

网络拓扑：
  Sim-Engine Node ──(UDP)── DIS-Gateway Node ──(UDP 多播)── 外部 DIS 实体
       ↓
  eventout_bridge
       ↓
  skillEventBus → engage/mission

降级策略：
  └── 本地回环模式：PDU 不发送到网络，仅本地记录
```

#### 9.4.2 可分割组 2：Mil-Domain Service

```
组成成员：
  ├── mil_domain (MCP Service)
  ├── classification_service (MCP Service)
  └── embedded Tool: mil.weapon, mil.sensor, mil.platform

实时性约束分析：
  ├── WsfWeapon::SelectTarget() 调用 mil_domain::get_max_range
  ├── 调用频率: 交战决策时 ~1Hz（不是每帧！）
  ├── 延迟容忍: 100ms（远大于仿真帧时间 16.67ms）
  └── NOT 每帧调用 → 可分离部署

分离可行性：
  ├── ✅ mil_domain 维护的是"领域知识"（武器性能表、平台数据库）
  ├── ✅ 不是每帧更新的实时状态
  ├── ✅ 适合作为 Service 独立部署，通过 MCP 调用
  └── ✅ 1Hz × 5ms 调用延迟 = 0.005% CPU 开销

网络拓扑：
  Sim-Engine Node ──(MCP stdio)── Mil-Domain Node
       ↓
  WsfMilInterface
       ↓
  afsim-engine Core
```

#### 9.4.3 可分割组 3：Data Repository

```
组成成员：
  ├── geodata_elevation Repository
  ├── geodata_landuse Repository
  ├── geodata_bathymetry Repository
  ├── geodata_imagery Repository
  ├── result_db Repository
  └── project_workspace Repository

实时性约束分析：
  ├── mission::PlanPath() 调用 geodata 查询
  ├── 调用频率: 路径规划时（低频，秒级）
  ├── 每条路径查询 ~15-60 次（高程+坡度+坡向）
  ├── 路径规划本身秒级 → 查询延迟无影响
  └── mystic::PostProcess() 调用 result_db（离线）

分离可行性：
  ├── ✅ 数据仓库是只读查询服务
  ├── ✅ 数据量大（GB 级），不适合嵌入
  ├── ✅ 查询延迟对仿真实时性无影响
  └── ✅ 可通过 MCP Resource 协议访问

数据量与部署：
  ├── DTED 高程数据: 10-100 GB → 独立节点
  ├── 土地利用数据: 1-10 GB → 与高程合并
  ├── 仿真结果 DB: 1-100 GB → 独立节点
  └── 本地 GeoTileCache: ~100MB (LRU, 命中率 80%+)

网络拓扑：
  Sim-Engine Node ──(MCP stdio)── Data-Repository Node
       ↓                      ↓
  WsfRouteMover          GeoTileManager
       ↓                      ↓
  mission Module         geodata Repository
```

---

### 9.5 Tool-Module 调用频率汇总表（完整版）

| Tool 域 | 主调用方 | 调用场景 | 调用次数/帧 | 能否远端 | 部署形态 | 延迟容忍 |
|---------|---------|---------|-----------|---------|---------|---------|
| **earth.coordinates** | | | | | | |
| GetGravityAcceleration | afsim-engine | 每帧每平台 | 5×100平台=500 | ❌ | 嵌入式 | < 0.1ms |
| GetDensity/Pressure/Temp | afsim-engine | 每帧每平台 | 7×100平台=700 | ❌ | 嵌入式 | < 0.1ms |
| ConvertGeodeticToGeocentric | afsim-engine | 每帧每平台 | 2×100=200 | ❌ | 嵌入式 | < 0.1ms |
| GreatCircleDistance | afsim-engine | 路径规划 | 秒级 | ✅ | Repository | 100ms |
| **math.linear** | | | | | | |
| UtVec3::Magnitude | afsim-engine | 每帧每平台 | 20×100=2000 | ❌ | 嵌入式 | < 0.1ms |
| UtVec3::Distance | afsim-engine+engage | 每帧每武器 | 15×200=3000 | ❌ | 嵌入式 | < 0.1ms |
| UtVec3::DotProduct | afsim-engine | 每帧每平台 | 10×100=1000 | ❌ | 嵌入式 | < 0.1ms |
| UtMat3::Multiply | afsim-engine | 每帧每平台 | 8×100=800 | ❌ | 嵌入式 | < 0.1ms |
| **math.rotation** | | | | | | |
| UtDCM::Multiply | afsim-engine | 每帧每平台 | 8×100=800 | ❌ | 嵌入式 | < 0.1ms |
| UtDCM::Rotate | afsim-engine | 每帧每平台 | 5×100=500 | ❌ | 嵌入式 | < 0.1ms |
| UtQuat::Multiply | afsim-engine | 每帧每平台 | 5×100=500 | ❌ | 嵌入式 | < 0.1ms |
| **math.noise** | | | | | | |
| UtNoise::GaussianRandom | afsim-engine | 每帧每传感器 | 3×500=1500 | ❌ | 嵌入式 | < 0.1ms |
| **math.interpolation** | | | | | | |
| UtMath::LinearInterpolate | engage | 弹道积分 | 发射时 50-200 | ❌ | 嵌入式 | < 0.1ms |
| UtTable::Lookup | afsim-engine | 配置查询 | 初始化时 | ❌ | 嵌入式 | N/A |
| **weapon.ballistic** | | | | | | |
| BallisticLaunchComputer::Compute | engage | 发射时 | 1×发射=0.5ms | ❌ | 嵌入式 | < 0.5ms |
| TrajectoryCalculator::Propagate | engage | 每帧每导弹 | 50-200×20导弹 | ❌ | 嵌入式 | < 1ms |
| **weapon.guidance** | | | | | | |
| GuidanceGainComputer::Compute | engage | 每帧每导弹 | 10×20=200 | ❌ | 嵌入式 | < 0.2ms |
| ProportionalNavigation::Compute | engage | 每帧每导弹 | 10×20=200 | ❌ | 嵌入式 | < 0.1ms |
| **mil.weapon** | | | | | | |
| get_max_range | mil_domain | 目标选择 | 1Hz | ✅ | Service | 100ms |
| get_probability_of_hit | mil_domain | 交战评估 | 1Hz | ✅ | Service | 100ms |
| **dis.encoding** | | | | | | |
| DisPdu::Serialize | dis_gateway | 每帧每平台 | 1-5×100=500 | ✅ | Service | 100ms |
| **l16.encoding** | | | | | | |
| Interface::EncodeMessage | l16_gateway | 消息发生时 | 1-10/秒 | ✅ | Service | 100ms |
| **geo.elevation** | | | | | | |
| QueryElevation | mission | 路径规划 | 5-20×路径 | ✅ | Repository | 100ms |
| **space.orbital** | | | | | | |
| WsfOrbitalPropagation::Propagate | wsf_space | 每积分步 | 10-50×步 | ❌ | 嵌入式 | < 5ms |
| **math.statistics** | | | | | | |
| Mean/StdDev/Correlation | mystic | 离线分析 | 分析时 | ✅ | 嵌入式/远端 | 秒级 |

---

### 9.6 Plugin-Adapter-Pipeline-Module 完整嵌套关系

#### 9.6.1 afsim-engine Module 完整嵌套结构树

```
afsim-engine (Module / Skill)
│
├── 【生命周期钩子】
│     Initialize() → Run() [循环] → Pause() → Stop() → Destroy()
│     仿真帧循环: AdvanceFrame() at 10-60 Hz
│
├── 【内嵌 Plugin】← 通过 WsfPluginManager::LoadPlugins() 加载
│     │
│     ├── wsf_brawler
│     │     ├── 文件: wsf_brawler.dll / libwsf_brawler.so
│     │     ├── 入口: WsfPluginSetup(WsfApplication&) → 调用 WsfProcessor::AddType()
│     │     ├── 工厂: WsfBrawlerProcessor::Factory()
│     │     ├── 注册: WsfProcessor::AddType("WsfBrawlerProcessor", Factory)
│     │     ├── 继承: WsfQuantumTaskerProcessor (AI 决策基类)
│     │     ├── 成员: WsfThreatProcessor*, WsfPerceptionProcessor*, WsfWeapon*
│     │     ├── Update: 每帧调用 WsfBrawlerMover::Update() → UtEarth/UtVec3
│     │     └── Tool 依赖: earth.*, math.linear, math.rotation (极高频)
│     │
│     ├── wsf_air_combat (SA 态势感知)
│     │     ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfSA_Processor")
│     │     ├── 工厂: WsfSA_Processor::Factory()
│     │     ├── 继承: WsfProcessor
│     │     ├── 成员: WsfSensor*, WsfTrackManager*
│     │     ├── Update: WsfSensor::Update() → 探测判决 → WsfTrackManager
│     │     └── Tool 依赖: earth.*, math.linear (中频)
│     │
│     ├── wsf_iads_c2 (综合防空指挥控制)
│     │     ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfBattleManager")
│     │     ├── 工厂: WsfBattleManager::Factory() / UninitializedWsfBattleManager()
│     │     ├── 继承: WsfProcessor
│     │     ├── 成员: WsfWeapon*, WsfComm*, WsfSensor*
│     │     ├── Update: 威胁评估 → 武器分配 → 效果评估
│     │     └── Tool 依赖: earth.* (中频)
│     │
│     ├── wsf_6dof (六自由度运动模型)
│     │     ├── 入口: WsfPluginSetup() → WsfMover::AddType("WsfGuidedMover")
│     │     ├── 工厂: WsfSixDOF_Mover::Factory()
│     │     ├── 继承: WsfGuidedMoverBase
│     │     ├── 成员: SixDOF_Vehicle*, WsfLaunchComputer*
│     │     ├── Update: 六自由度运动方程 → UtVec3/UtMat3 高频
│     │     └── Tool 依赖: UtVec3, UtMat3, UtDCM (极高频)
│     │
│     ├── wsf_radar_clutter (雷达杂波模型)
│     │     ├── 入口: WsfPluginSetup() → WsfSensor::AddType("WsfRadarClutterProcessor")
│     │     ├── 继承: WsfProcessor
│     │     ├── Update: MTI/MTD 滤波 → 杂波图更新
│     │     └── Tool 依赖: UtNoise, UtMath (高频)
│     │
│     ├── wsf_optical (光学传感器)
│     │     ├── 入口: WsfPluginSetup() → WsfSensor::AddType("WsfOpticalSensor")
│     │     ├── 继承: WsfSensor
│     │     ├── Update: 红外/光电探测 → 信噪比计算
│     │     └── Tool 依赖: UtMath, UtVec3 (高频)
│     │
│     ├── wsf_ew (电子战)
│     │     ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfEW_Processor")
│     │     ├── 继承: WsfProcessor
│     │     ├── 成员: WsfComm* (干扰), WsfSensor* (ESM)
│     │     └── 调用 Service: cyber_service
│     │
│     ├── wsf_battle_damage (战损评估)
│     │     ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfBattleDamageProcessor")
│     │     ├── 继承: WsfProcessor
│     │     ├── 成员: WsfWeapon*, WsfPlatform*
│     │     └── 功能: 命中判定 → 损伤模型 → 任务能力评估
│     │
│     ├── wsf_multiresolution (多分辨率建模)
│     │     ├── 入口: WsfPluginSetup() → WsfPlatform::AddType("WsfLOD_Platform")
│     │     ├── 继承: WsfPlatform
│     │     └── 功能: LOD 切换 → 聚合/解聚
│     │
│     ├── wsf_simple_platform (简单平台)
│     │     ├── 入口: WsfPluginSetup() → WsfPlatform::AddType("WsfSimplePlatform")
│     │     └── 继承: WsfPlatform
│     │
│     ├── guidance_computer (制导计算机)
│     │     ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfGuidanceComputer")
│     │     ├── 继承: WsfProcessor
│     │     ├── 成员: WsfLaunchComputer*
│     │     ├── Update: 制导律计算 → 指令输出
│     │     └── Tool 依赖: weapon.guidance (嵌入式)
│     │
│     └── classification_proc (目标分类)
│           ├── 入口: WsfPluginSetup() → WsfProcessor::AddType("WsfClassificationProcessor")
│           ├── 继承: WsfProcessor
│           └── 功能: 目标类型识别 → 敌我识别
│
├── 【内嵌 Adapter】
│     │
│     ├── eventout_bridge
│     │     ├── 类型: WsfEvent → skillEventBus 事件桥接
│     │     ├── 宿主注入: WsfEventManager::AddEvent() → skillEventBus.emit()
│     │     ├── 转换规则:
│     │     │   WsfEvent::WeaponFired → "afsim:weapon_fired"
│     │     │   WsfEvent::Detection → "afsim:sensor_detection"
│     │     │   WsfEvent::TrackUpdate → "afsim:track_update"
│     │     │   WsfEvent::EngagementStart → "afsim:engagement_start"
│     │     │   WsfEvent::PlatformKilled → "afsim:platform_killed"
│     │     │   WsfEvent::MissileImpact → "afsim:missile_impact"
│     │     └── 方向: afsim-engine → skillEventBus (单向)
│     │
│     ├── wsfcgi_adapter
│     │     ├── 类型: HTTP CGI → WsfCommand 桥接
│     │     ├── 宿主注入: WsfScenario::Load() 时注册 CGI 处理器
│     │     ├── 方向: HTTP 请求 → afsim-engine (双向)
│     │     ├── 用途: Wizard 前端远程控制仿真 (start/pause/stop)
│     │     └── 端点: /wsfcgi/simulation/control, /wsfcgi/entity/query
│     │
│     ├── wizard_config_adapter
│     │     ├── 类型: .scenario XML → WsfScenario 对象
│     │     ├── 宿主注入: Wizard::ImportScenario()
│     │     ├── 方向: Wizard UI → afsim-engine
│     │     └── 转换: XML 解析 → WsfScenarioBuilder → WsfScenario
│     │
│     ├── genius_serializer
│     │     ├── 类型: 结构化数据 → 二进制流
│     │     ├── 宿主注入: genio::GenO::Write()
│     │     └── 用于: DIS PDU 序列化
│     │
│     └── genius_deserializer
│           ├── 类型: 二进制流 → 结构化数据
│           ├── 宿主注入: genio::GenI::Read()
│           └── 用于: DIS PDU 反序列化
│
├── 【内嵌 Pipeline】
│     │
│     ├── sensor_detection
│     │     ├── 宿主注入: WsfSensor::Update() 内各阶段
│     │     ├── 阶段 1: signal_generation
│     │     │     WsfRadarSensor::GenerateSignal()
│     │     │     → 调用 Tool: UtNoise::GaussianRandom (嵌入式)
│     │     ├── 阶段 2: mti_filter
│     │     │     WsfRadarMTI_SignalProcessor::Process()
│     │     │     → 调用 Tool: UtMath::FFT (嵌入式)
│     │     ├── 阶段 3: detection_decision
│     │     │     WsfSensor::DecideDetection()
│     │     │     → 调用 Tool: UtMath::Probability (嵌入式)
│     │     ├── 阶段 4: track_correlate
│     │     │     WsfTrackManager::CorrelateDetection()
│     │     │     → 调用 Service: mtt_tracker.track.correlate (异步)
│     │     └── 输出: 探测事件 → eventout_bridge → skillEventBus
│     │
│     └── ripr_job
│           ├── 宿主注入: WsfRIPRProcessor::SubmitJob()
│           ├── 阶段 1: job_submit → RIPR 作业提交
│           ├── 阶段 2: job_dispatch → xio_job_driver 分发
│           ├── 阶段 3: job_execute → RIPR 计算节点执行
│           ├── 阶段 4: result_collect → 结果收集
│           ├── 阶段 5: result_merge → 结果合并
│           └── 调用 Driver: xio_job_driver
│
├── 【调用 Service】
│     │
│     ├── dis_gateway
│     │     ├── 调用点: WsfDisInterface::SendEntityState()
│     │     ├── 调用频率: 每帧每平台 1-5 次
│     │     ├── 调用方式: DisPdu::Serialize() → dis_gateway.send_pdu()
│     │     ├── 延迟容忍: 100ms
│     │     └── 部署: DIS-Gateway Node (独立进程)
│     │
│     ├── mil_domain
│     │     ├── 调用点: WsfWeapon::SelectTarget(), WsfWeapon::AssessEffect()
│     │     ├── 调用频率: 交战决策时 ~1Hz
│     │     ├── 调用方式: MCP call_tool("mil.weapon.get_max_range", ...)
│     │     ├── 延迟容忍: 100ms
│     │     └── 部署: Mil-Domain Node (独立进程)
│     │
│     └── l16_gateway
│           ├── 调用点: WsfComm::SendMessage()
│           ├── 调用频率: 通信消息发生时
│           ├── 延迟容忍: 100ms
│           └── 部署: L16-Gateway Node (独立进程)
│
├── 【调用 Repository】
│     │
│     └── geodata_elevation
│           ├── 调用点: WsfRouteMover::PlanPath()
│           ├── 调用频率: 路径规划时 (秒级)
│           ├── 调用方式: MCP read_resource("geo://elevation/tile/...")
│           ├── 延迟容忍: 100ms
│           ├── 本地缓存: GeoTileCache (~100MB LRU)
│           └── 部署: Data-Repository Node (独立进程)
│
└── 【Embedded Tool】← 必须与 afsim-engine 同节点
      ├── earth.coordinates (UtEarth 7 方法) ← 每帧 500+ 次
      ├── earth.gravity (UtEarth 2 方法) ← 每积分步 1 次
      ├── earth.atmosphere (UtEarth 3 方法) ← 每帧 700+ 次
      ├── math.linear (UtVec3/UtMat3 8 方法) ← 每帧 7000+ 次
      ├── math.rotation (UtDCM/UtQuat 7 方法) ← 每帧 1800+ 次
      ├── math.interpolation (UtMath 2 方法) ← 发射时
      ├── math.noise (UtNoise 1 方法) ← 每帧 1500+ 次
      ├── units.conversion (UtUnits 3 方法) ← 每帧 2600+ 次
      └── units.angle (UtAngle 2 方法) ← 每帧 900+ 次
```

#### 9.6.2 engage Module 完整嵌套结构树

```
engage (Module / Skill)
│
├── 【生命周期钩子】
│     Initialize() → Run() [交战循环] → Stop() → Destroy()
│     交战循环: 目标选择 → 武器发射 → 弹道跟踪 → 命中判定
│
├── 【无内嵌 Plugin】← engage 是顶层交战管理，不含 Plugin
│
├── 【内嵌 Adapter】
│     │
│     └── sensor_plot_adapter
│           ├── 类型: 交战探测结果 → Warlock 可视化数据
│           ├── 宿主注入: engage 交战决策循环内
│           ├── 方向: engage → Warlock 可视化
│           ├── 转换: EngagementResult → PlotData → Qt Signal
│           └── 输出: engagement_plot_update 事件
│
├── 【内嵌 Pipeline】
│     │
│     └── weapon_engagement
│           ├── 宿主注入: WsfWeapon::Fire() 触发
│           ├── 阶段 1: target_selection
│           │     WsfWeapon::SelectTarget()
│           │     → 调用 Service: mil_domain.get_max_range (5ms)
│           │     → 调用 Tool: UtVec3::Distance (嵌入式)
│           │     → 输出: SelectedTarget
│           ├── 阶段 2: launch_compute
│           │     BallisticLaunchComputer::Compute()
│           │     → 调用 Tool: weapon.ballistic.ballistic_launch_compute
│           │     → 调用 Tool: UtMath::LinearInterpolate (嵌入式)
│           │     → 调用 Tool: UtVec3::Distance (嵌入式)
│           │     → 耗时: ~0.5ms (本地)
│           │     → 输出: LaunchSolution
│           ├── 阶段 3: trajectory_propagate
│           │     TrajectoryCalculator::Propagate()
│           │     → 调用 Tool: weapon.ballistic.trajectory_calculate
│           │     → 每帧迭代 50-200 次
│           │     → 耗时: ~1ms/帧/导弹
│           │     → 输出: MissileTrajectory
│           ├── 阶段 4: impact_decision
│           │     WsfWeapon::AssessImpact()
│           │     → 调用 Tool: weapon.guidance.proportional_nav
│           │     → 调用 Tool: UtVec3::Distance (嵌入式)
│           │     → 输出: ImpactPoint / Miss
│           ├── 阶段 5: effect_assessment
│           │     WsfWeapon::AssessEffect()
│           │     → 调用 Service: mil_domain.get_lethality (5ms)
│           │     → 调用 Tool: math.interpolation (嵌入式)
│           │     → 输出: DamageLevel
│           └── 事件输出: "engage:weapon_fired", "engage:impact", "engage:effect"
│
├── 【调用 Tool (嵌入式 via afsim-engine)】
│     │
│     ├── weapon.ballistic (嵌入式)
│     │     ├── 方法: BallisticLaunchComputer::Compute, TrajectoryCalculator::Propagate
│     │     ├── 调用频率: 每次发射 1 次 Compute + 每帧 Propagate
│     │     └── 嵌入式原因: 弹道积分 50-200 次迭代/帧，远端延迟不可接受
│     │
│     ├── weapon.guidance (嵌入式)
│     │     ├── 方法: GuidanceGainComputer::Compute, ProportionalNavigation::Compute
│     │     ├── 调用频率: 每帧每导弹 10 次
│     │     └── 嵌入式原因: 制导计算实时性要求 < 0.1ms
│     │
│     └── math.linear (嵌入式 via afsim-engine)
│           ├── 方法: UtVec3::Distance, AngleBetween, DotProduct, Rotate
│           ├── 调用频率: 每交战 10-25 次
│           └── 嵌入式原因: 交战几何计算每帧多次
│
└── 【调用 Service】
      │
      ├── mil_domain
      │     ├── 方法: get_max_range, get_probability_of_hit, get_lethality
      │     ├── 调用频率: 目标选择 ~1Hz, 效果评估 ~0.1Hz
      │     ├── 延迟: 5-30ms (MCP stdio)
      │     └── 容忍: 100ms
      │
      └── mtt_tracker (异步优化)
            ├── 方法: track.initiate, track.update, track.correlate
            ├── 调用频率: 探测发生时 → 本地缓冲 → 批量发送
            ├── 优化: N=10 帧批量，延迟增加 167ms (60Hz)
            └── 降级: 本地简化 KalmanFilter
```

#### 9.6.3 mission Module 完整嵌套结构树

```
mission (Module / Skill)
│
├── 【生命周期钩子】
│     Initialize() → Plan() [规划循环] → Execute() → Monitor() → Complete()
│     规划循环: 任务分解 → 资源分配 → 路径规划 → 执行调度
│
├── 【无内嵌 Plugin】
│
├── 【内嵌 Adapter】
│     │
│     └── mission_result_adapter
│           ├── 类型: 任务执行结果 → 态势更新事件
│           ├── 宿主注入: mission 执行状态变化时
│           ├── 方向: mission → afsim-engine
│           └── 输出: mission_task_update 事件
│
├── 【内嵌 Pipeline】
│     │
│     └── mission_planning
│           ├── 宿主注入: 用户触发或自动规划时
│           ├── 阶段 1: task_decomposition
│           │     MissionTasker::Decompose()
│           │     → 任务分解为子任务
│           │     → 输出: TaskTree
│           ├── 阶段 2: resource_allocation
│           │     WsfAssetManager::Allocate()
│           │     → 资源匹配子任务
│           │     → 调用 Service: mil_domain.get_available_assets (5ms)
│           │     → 输出: ResourceAssignment
│           ├── 阶段 3: path_planning
│           │     WsfRouteMover::PlanPath()
│           │     → A* 搜索算法
│           │     → 调用 Tool: UtVec3::Distance (嵌入式)
│           │     → 调用 Repository: geodata_elevation.query_elevation (1ms+IO)
│           │     → 调用 Repository: geodata_landuse.query_land_use (0.5ms+IO)
│           │     → 输出: PlannedRoute
│           ├── 阶段 4: execution_schedule
│           │     WsfEventManager::Schedule()
│           │     → 任务时序安排
│           │     → 输出: ExecutionTimeline
│           ├── 阶段 5: progress_monitor
│           │     WsfEventManager::Track()
│           │     → 任务进度监控
│           │     → 事件: mission:task_started, mission:task_completed
│           └── 输出: MissionPlan
│
├── 【调用 Tool (嵌入式 via afsim-engine)】
│     │
│     ├── math.linear
│     │     ├── 方法: UtVec3::Distance, Normalize (A* 搜索)
│     │     ├── 调用频率: 每搜索节点 1-2 次
│     │     └── 嵌入式原因: A* 搜索 10000+ 节点，远端延迟不可接受
│     │
│     └── earth.coordinates
│           ├── 方法: UtEarth::GreatCircleDistance (路径距离)
│           ├── 调用频率: 每搜索节点 1 次
│           └── 嵌入式原因: 同上
│
├── 【调用 Repository】
│     │
│     ├── geodata_elevation
│     │     ├── 方法: query_elevation, query_slope, query_aspect
│     │     ├── 调用频率: 每路径 5-20 次 (航路点 × 3)
│     │     ├── 延迟: ~1ms + IO (含网络)
│     │     ├── 容忍: 100ms (路径规划秒级)
│     │     ├── 本地缓存: GeoTileCache (~100MB)
│     │     └── 缓存命中率: 80%+
│     │
│     └── geodata_landuse
│           ├── 方法: query_land_use
│           ├── 调用频率: 每路径 5 次
│           └── 延迟: ~0.5ms + IO
│
└── 【调用 Service】
      │
      └── mil_domain
            ├── 方法: get_available_assets, get_asset_capability
            ├── 调用频率: 资源分配时 ~1Hz
            ├── 延迟: 5-30ms
            └── 容忍: 100ms
```

#### 9.6.4 wizard Module 完整嵌套结构树

```
wizard (Module / Skill)
│
├── 【生命周期钩子】
│     Initialize() → Load() → Edit() [编辑循环] → Save() → Destroy()
│     编辑循环: 平台编辑 → 传感器配置 → 武器配置 → 场景验证
│
├── 【内嵌 Plugin】← 通过 Wizard Plugin 接口加载
│     │
│     ├── ScenarioAnalyzer
│     │     ├── 类型: wkf::Plugin (Wizard Plugin)
│     │     ├── 入口: Wizard::LoadPlugin("ScenarioAnalyzerPlugin")
│     │     ├── 生命周期: Initialize() → Analyze() → Destroy()
│     │     ├── 功能: 场景静态分析、冲突检测
│     │     └── 输出: analysis_report 事件
│     │
│     ├── TaskList
│     │     ├── 类型: wkf::Plugin
│     │     ├── 入口: Wizard::LoadPlugin("TaskListPlugin")
│     │     ├── 功能: 任务列表管理
│     │     └── 输出: task_list_update 事件
│     │
│     └── MysticLauncher
│           ├── 类型: wkf::Plugin
│           ├── 入口: Wizard::LoadPlugin("MysticLauncherPlugin")
│           ├── 功能: 启动 Mystic 后处理器
│           └── 调用: mystic::start_post_processing
│
├── 【内嵌 Adapter】
│     │
│     ├── wizard_config_adapter
│     │     ├── 类型: .scenario XML → WsfScenario 对象
│     │     ├── 宿主注入: Wizard::ImportScenario()
│     │     ├── 方向: Wizard UI → afsim-engine
│     │     ├── 转换: XML 解析 → WsfScenarioBuilder → WsfScenario
│     │     └── 用途: 导入场景文件
│     │
│     └── wizard_export_adapter
│           ├── 类型: WsfScenario → .scenario XML
│           ├── 宿主注入: Wizard::ExportScenario()
│           ├── 方向: afsim-engine → Wizard UI
│           ├── 转换: WsfScenario → XML 序列化
│           └── 用途: 导出场景文件
│
├── 【调用 Service】
│     │
│     └── afsim-engine (仿真控制)
│           ├── 调用: simulation.load_scenario, simulation.start, simulation.pause
│           ├── 协议: wsfcgi (HTTP CGI)
│           ├── 延迟: 10-50ms (HTTP 请求)
│           └── 部署: Sim-Engine Node
│
└── 【调用 Repository】
      │
      └── project_workspace
            ├── 方法: list_files, search_project, read_file, write_file
            ├── 调用频率: UI 交互时
            ├── 延迟: 10-100ms (文件系统)
            └── 部署: Data-Repository Node
```

#### 9.6.5 mystic Module 完整嵌套结构树

```
mystic (Module / Skill)
│
├── 【生命周期钩子】
│     Initialize() → Load() → Analyze() [分析循环] → Report() → Destroy()
│     分析循环: 结果加载 → 过滤 → 聚合 → 统计 → 报告
│
├── 【内嵌 Plugin】← 通过 Mystic Plugin 接口加载
│     │
│     ├── ResultTimeController
│     │     ├── 类型: mystic::Plugin
│     │     ├── 入口: Mystic::LoadPlugin("ResultTimeControllerPlugin")
│     │     ├── 功能: 仿真结果时间轴控制 (播放/暂停/快进)
│     │     └── 输出: time_slider_update 事件
│     │
│     ├── TaskAssigner
│     │     ├── 类型: mystic::Plugin
│     │     ├── 入口: Mystic::LoadPlugin("TaskAssignerPlugin")
│     │     ├── 功能: 分析任务分配给操作员
│     │     └── 输出: task_assigned 事件
│     │
│     ├── TaskStatus
│     │     ├── 类型: mystic::Plugin
│     │     ├── 功能: 任务状态跟踪
│     │     └── 输出: task_status_update 事件
│     │
│     ├── Log
│     │     ├── 类型: mystic::Plugin
│     │     ├── 功能: 日志查看
│     │     └── 输出: log_entry 事件
│     │
│     └── WsfPrompt
│           ├── 类型: warlock::Plugin
│           ├── 功能: 仿真提示/建议生成
│           └── 调用: AI/LLM 生成建议
│
├── 【内嵌 Adapter】
│     │
│     ├── mystic_result_adapter
│     │     ├── 类型: RvResultDb → JSON 流
│     │     ├── 宿主注入: RvWsfPipe::Stream()
│     │     ├── 方向: 仿真结果 DB → Mystic UI
│     │     ├── 转换: 二进制 DB 记录 → JSON 事件
│     │     └── 事件: result:event, result:track, result:engagement
│     │
│     └── warlock_event_adapter
│           ├── 类型: Warlock Qt Signal → skillEventBus 事件
│           ├── 宿主注入: Warlock 信号连接时
│           ├── 方向: Warlock 可视化 → mystic
│           ├── 转换: Qt Signal → skillEventBus.emit()
│           └── 事件: warlock:viewport_change, warlock:selection_change
│
├── 【内嵌 Pipeline】
│     │
│     └── post_processing
│           ├── 宿主注入: 仿真结束后手动触发
│           ├── 阶段 1: event_read
│           │     RvResultDb::QueryEvents()
│           │     → 调用 Repository: result_db.query_events
│           │     → 输出: EventStream
│           ├── 阶段 2: filter
│           │     EventFilter::Apply()
│           │     → 时间范围/平台类型/事件类型过滤
│           │     → 输出: FilteredEvents
│           ├── 阶段 3: aggregation
│           │     Aggregator::Aggregate()
│           │     → 事件聚合 (击杀数、弹药消耗等)
│           │     → 输出: AggregatedStats
│           ├── 阶段 4: statistics
│           │     Statistician::Compute()
│           │     → 调用 Tool: math.statistics (嵌入式)
│           │     → Mean, StdDev, Correlation 计算
│           │     → 输出: StatisticalReport
│           └── 阶段 5: report_generation
│                 ReportGenerator::Generate()
│                 → 输出: PDF/HTML 报告
│
└── 【调用 Repository】
      │
      └── result_db
            ├── 方法: query_events, query_tracks, query_engagements
            ├── 调用频率: 分析时大量查询
            ├── 延迟: 10-100ms (DB 查询)
            ├── 数据量: 1-100 GB
            └── 部署: Data-Repository Node
```

---

### 9.7 Service-Module 依赖关系详解

#### 9.7.1 dis_gateway Service ↔ afsim-engine Module

```
依赖方向: dis_gateway → afsim-engine (DIS 协议接入)

【dis_gateway Service 暴露的 Tool】
  ├── dis.entity.register(platform_id, position, velocity, force_id)
  ├── dis.entity.update(platform_id, position, velocity)
  ├── dis.entity.remove(platform_id)
  ├── dis.entity.list() → Vec<DisEntityId>
  ├── dis.pdu.send_fire(weapon_id, target_id, firing_location)
  ├── dis.pdu.send_state(entity_id, location, orientation, velocity)
  ├── dis.pdu.send_detonation(weapon_id, target_id, impact_point)
  ├── dis.radio.set_transmitter_frequency(radio_id, frequency_mhz)
  ├── dis.radio.set_modulation(radio_id, modulation_type)
  └── dis.warfare.set_fuse_type(weapon_id, fuse_type)

【afsim-engine 调用 dis_gateway 的入口】
  └── WsfDisInterface (afsim-engine 内置)
        ├── SendEntityState() → dis.entity.register/update
        ├── SendWeaponFire() → dis.pdu.send_fire
        ├── SendDetonation() → dis.pdu.send_detonation
        └── ReceiveEntityState() → 通过 eventout_bridge 注入

【调用频率分析】
  - SendEntityState: 每帧每平台 1-5 次 (DIS 通常 5-20Hz)
  - SendWeaponFire: 每次发射时 ~1 次
  - ReceiveEntityState: 外部 DIS 实体更新频率 (5-30Hz)

【实时性约束】
  - 延迟容忍: 100ms (DIS 协议本身松耦合)
  - 每帧可缓冲: 16.67ms (60Hz) × 6 ≈ 100ms

【网络拓扑】
  Sim-Engine Node
       │
       ├── WsfDisInterface
       │     └── DisPdu::Serialize() → GenIO 二进制
       │
       ├── eventout_bridge
       │     └── "afsim:dis_entity_update" → skillEventBus
       │
       ↓ (UDP, 本地端口转发或跨节点)
  DIS-Gateway Node
       │
       ├── dis_driver (UDP 多播维护)
       │     └── genO::Write() → UDP socket
       │
       └── dis_gateway (MCP Service)
             └── call_tool("dis.pdu.send_*")

【降级策略】
  └── 本地回环模式: PDU 不发送到网络，仅本地记录
```

#### 9.7.2 mil_domain Service ↔ afsim-engine Module

```
依赖方向: 双向，但解耦 (mil_domain 不直接参与仿真帧循环)

【mil_domain Service 暴露的 Tool】
  ├── mil.platform.get_type(platform_id) → PlatformType
  ├── mil.platform.get_side(platform_id) → Side
  ├── mil.platform.get_damage_factor(platform_id) → Float
  ├── mil.platform.get_speed(platform_id) → Speed
  ├── mil.sensor.get_detection_range(sensor_id) → Distance
  ├── mil.sensor.get_signature(target_id) → Signature
  ├── mil.sensor.get_pdd(sensor_id, target_id) → Float (探测概率)
  ├── mil.weapon.get_max_range(weapon_id) → Distance
  ├── mil.weapon.get_min_range(weapon_id) → Distance
  ├── mil.weapon.get_probability_of_hit(weapon_id, target_id, range) → Float
  ├── mil.weapon.get_lethality(weapon_id, target_id) → Float
  ├── mil.weapon.get_reload_time(weapon_id) → Time
  ├── mil.engagement.start(weapon_id, target_id) → EngagementId
  ├── mil.engagement.end(engagement_id, result)
  └── mil.engagement.get_status(engagement_id) → EngagementStatus

【afsim-engine 调用 mil_domain 的入口】
  └── WsfMilInterface (afsim-engine 内置, MCP stdio 调用)
        ├── WsfWeapon::SelectTarget() → mil.weapon.get_max_range
        ├── WsfWeapon::AssessEffect() → mil.weapon.get_lethality
        └── WsfSensor::GetDetectionProbability() → mil.sensor.get_pdd

【调用频率分析】
  - 目标选择: 每次交战开始时 ~1 次
  - 效果评估: 每次命中时 ~1 次
  - NOT 每帧调用 ← 这是 mil_domain 与 afsim-engine 可分离的关键

【实时性约束】
  - 延迟容忍: 100ms (远大于仿真帧时间 16.67ms)
  - 1Hz × 5ms = 0.5% CPU 开销

【网络拓扑】
  Sim-Engine Node ──(MCP stdio)── Mil-Domain Node
       │                            │
       WsfMilInterface            mil_domain Service
       │                            │
       │                            └── 武器性能数据库
       │                                平台数据库
       ↓
  afsim-engine Core
```

#### 9.7.3 mtt_tracker Service ↔ afsim-engine Module

```
依赖方向: 双向，紧密耦合但时序可分离

【mtt_tracker Service 暴露的 Tool】
  ├── mtt.track.initiate(detection_info) → TrackId
  ├── mtt.track.update(track_id, detection_info)
  ├── mtt.track.delete(track_id)
  ├── mtt.track.predict(track_id, future_time) → PredictedState
  ├── mtt.track.get_history(track_id) → Vec<TrackState>
  ├── mtt.track.correlate(detection_batch) → Vec<CorrelationResult>
  └── mtt.track.get_tracks() → Vec<TrackSummary>

【afsim-engine 调用 mtt_tracker 的入口】
  └── WsfTrackManager (afsim-engine 内置)
        ├── WsfSensor::OnDetection() → mtt.track.initiate
        ├── WsfTrackManager::Update() → mtt.track.update
        └── WsfTrackManager::Predict() → mtt.track.predict

【关键问题: mtt_tracker 能分离部署吗？】

原始分析:
  - WsfSensor::OnDetection() 每帧每传感器触发
  - mtt.track.initiate/update 调用频率: 10-100 Hz (总航迹数 × 更新率)
  - 若 mtt_tracker 为远端 Service，每次调用增加 5-30ms 延迟

【优化方案对比】

方案 A: 直接远端化 (不推荐)
  - 延迟: 5-30ms × 100 Hz = 500-3000ms/s
  - 问题: 航迹更新滞后，仿真精度下降
  └── 结论: 不可接受

方案 B: 异步批量模式 (推荐) ⭐
  - WsfSensor::OnDetection() → 本地缓冲队列 (DetectionBuffer)
  - 每 N 帧批量发送到 mtt_tracker (N=10-60)
  - 延迟增加: N × 帧时间 = 10 × 16.67ms = 167ms (60Hz)
  - 吞吐量提升: N 倍
  - 航迹更新从 100Hz 降到 6Hz (60Hz ÷ 10)
  └── 结论: 可接受，航迹平滑，实时性损失可容忍

方案 C: 共享内存模式 (最优但复杂)
  - mtt_tracker 与 afsim-engine 共享内存区域 (SharedMemoryQueue)
  - 延迟 < 0.1ms
  - 需要进程间同步
  └── 结论: 长期目标

方案 D: 本地降级模式 (容错)
  - 若 mtt_tracker 不可用，本地使用简化 KalmanFilter
  - 不调用远端 Service
  └── 结论: 容错机制

【网络拓扑 (推荐异步模式)】
  Sim-Engine Node
       │
       ├── DetectionBuffer (本地缓冲, N=10 帧)
       │     └── 每 10 帧批量发送
       │
       ↓ (MCP stdio, 每 167ms 一次)
  Cyber-MTT Node
       │
       └── mtt_tracker Service
             └── KalmanFilter / JPDA 航迹管理
                   │
                   └── 航迹更新事件
                        │
                        ↓
                   skillEventBus → engage/mission

【降级策略】
  └── 本地 KalmanFilter: 若 mtt_tracker 不可用，自动切换
```

---

### 9.8 Repository-Module 依赖关系详解

#### 9.8.1 geodata_elevation Repository → mission Module

```
【geodata_elevation Repository 暴露的 Tool 和 Resource】

Tool:
  ├── query_elevation(lat, lon) → Float (高程 米)
  ├── query_slope(lat, lon) → Float (坡度 度)
  ├── query_aspect(lat, lon) → Float (坡向 度)
  └── query_batch(queries: Vec<(lat, lon)>) → Vec<Float>

Resource:
  ├── geo://elevation/tile/{z}/{x}/{y} (瓦片数据, GeoTIFF)
  └── geo://elevation/point?lat={}&lon={} (单点查询)

【mission Module 调用 geodata 的入口】
  └── WsfRouteMover::PlanPath()
        ├── WsfTerrainAvoidance::CheckTerrainClearance()
        │     └── query_elevation (每航路点 1 次)
        └── WsfRouteGenerator::EvaluateWaypoint()
              └── query_elevation, query_slope, query_aspect (每航路点 3 次)

【调用频率分析】
  - 路径规划时: 每条路径 5-20 个航路点
  - 每个航路点查询: 1-3 次 (高程 + 坡度 + 坡向)
  - 总体: 规划一次路径 ~15-60 次查询
  - 规划频率: 秒级 (用户手动触发或自动重规划)
  - 延迟容忍: 100ms (路径规划本身秒级，无影响)

【数据量与部署】
  - DTED 高程数据: 10-100 GB
  - 瓦片格式: GeoTIFF / SRTM HGT
  - 部署: Data-Repository Node (独立服务器)
  - 本地缓存: GeoTileCache (~100MB LRU)
  - 缓存命中率目标: 80%+

【网络拓扑】
  Sim-Engine Node
       │
       ├── WsfRouteMover
       │     └── PlanPath()
       │
       ├── GeoTileCache (本地 LRU)
       │     └── 命中 → 直接返回 (< 0.1ms)
       │     └── 未命中 ↓
       │
       ↓ (MCP stdio)
  Data-Repository Node
       │
       └── geodata_elevation Service
             └── GeoTileManager
                   └── DTED 文件系统

【缓存策略】
  ├── L1 缓存: GeoTileCache (内存, ~100MB)
  ├── L2 缓存: 磁盘文件系统 (热点区域预加载)
  └── 预取策略: 沿路径方向提前加载相邻瓦片
```

---

## 10. 8 类组件部署汇总

### 10.1 Tool（工具）

| 项目 | 内容 |
|------|------|
| 数量 | 23 域，~1,105 方法 |
| 部署形态 | WASM/FFI 嵌入式，每个节点本地部署 |
| 注册方式 | `DistributedToolRegistry.register_embedded(domain, tools[])` |
| 调用方式 | `actionRouter.invoke("earth.coordinates.great_circle_distance", args)` |
| 健康检查 | 不需要 |
| 故障恢复 | 重试 |
| 网络需求 | 无 |
| 延迟 | < 1ms |

### 10.2 Adapter（适配器）

| 项目 | 内容 |
|------|------|
| 数量 | 10 个 |
| 部署形态 | 嵌入在 Module 或 Service 内部 |
| 注册方式 | Skill manifest lifecycle 钩子 |
| 调用方式 | 自动触发，无需显式调用 |
| 健康检查 | 不需要（随宿主） |
| 故障恢复 | 跳过或降级 |
| 网络需求 | 无 |
| 延迟 | < 0.1ms |

| 适配器名 | 宿主 | 输入方向 | 输出方向 |
|---------|------|---------|---------|
| eventout_bridge | afsim-engine | WsfEvent | skillEventBus 事件 |
| genio_serializer | dis_gateway | 结构化数据 | 二进制流 |
| genio_deserializer | dis_gateway | 二进制流 | 结构化数据 |
| wsfcgi_adapter | afsim-engine | HTTP 请求 | WsfCommand |
| ripr_converter | post_processor | RIPR 数据 | Quantum 格式 |
| xio_bridge | afsim-engine | XIO 消息 | AgentMessage |
| warlock_event_adapter | mystic | Warlock 事件 | skillEventBus 事件 |
| wizard_config_adapter | afsim-engine | Wizard 项目文件 | WsfScenario |
| sensor_plot_adapter | engage | 探测结果 | 可视化数据 |
| mystic_result_adapter | mystic | 仿真结果 DB | JSON 流 |

### 10.3 Driver（驱动）

| 项目 | 内容 |
|------|------|
| 数量 | 5 个，~25 Tool + 9 Resource |
| 部署形态 | 独立进程，通过 TransportHandler 接入 |
| 注册方式 | `MessageGateway.register_transport()` + `register_endpoint()` |
| 调用方式 | `MessageGateway.send_message()` + 事件流自动接收 |
| 健康检查 | 必须（LivenessCheck + GatewayHealthCheck） |
| 故障恢复 | 重连 + 实体表恢复 |
| 网络需求 | 必须 |
| 延迟 | 5-30ms |

| 驱动名 | 连接类型 | 主动行为 | 暴露 Tool | 暴露 Resource |
|--------|---------|---------|----------|-------------|
| dis_driver | UDP 多播 | 心跳、PDU 收发、实体表维护 | send_pdu, get_entity, list_entities | dis://entities, dis://pdu_log |
| l16_driver | JTIDS/TADIL-J | 时隙管理、消息收发 | send_message, receive_message | l16://messages, l16://timeslots |
| packetio_tcp_driver | TCP | 连接维护、自动重连 | connect, send_packet | packetio://connections |
| packetio_udp_driver | UDP | 端口监听、数据报收发 | send_datagram, bind_port | packetio://ports |
| xio_job_driver | XIO 协议 | RIPR 作业分发、结果收集 | submit_job, get_job_status | xio://jobs, xio://results |

### 10.4 Service（服务）

| 项目 | 内容 |
|------|------|
| 数量 | 6 个，~790 Tool + 15 Resource |
| 部署形态 | MCP stdio 子进程 |
| 注册方式 | `McpToolRegistry.register_server(name, status, tools[], resources[])` |
| 调用方式 | `McpToolRegistry.call_tool(server, tool, args)` |
| 健康检查 | 必须（LivenessCheck + ReadinessCheck） |
| 故障恢复 | 子进程重启 + 消息队列缓存重放 |
| 网络需求 | 可选 |
| 延迟 | 5-30ms |

| 服务名 | 状态类型 | Tool 数 | Resource | 依赖 | 降级策略 |
|--------|---------|--------|----------|------|---------|
| mil_domain | 平台注册表、作战状态 | ~300 | mil://platforms, mil://weapons | dis_gateway | 缓存快照 |
| mtt_tracker | 航迹数据库 | ~50 | mtt://tracks | dis_gateway | 异步模式 |
| space_domain | 轨道参数、覆盖数据 | ~200 | space://orbits | 无 | 二体近似 |
| cyber_service | 漏洞库、攻击效果 | ~100 | cyber://vulnerabilities | 无 | 跳过 |
| fusion_center | 融合航迹 | ~80 | fusion://tracks | mtt, mil | 单源航迹 |
| classification_service | 分类模型 | ~60 | classification://models | mil_domain | 默认规则 |

### 10.5 Module（模块/Skill）

| 项目 | 内容 |
|------|------|
| 数量 | 8 个，~32 Capability |
| 部署形态 | Skill (iframe sandbox) |
| 注册方式 | `skillStore.createSkill(manifest)` + `skillExtensionStore.registerCapability()` |
| 调用方式 | Capability 系统 |
| 健康检查 | 必须 |
| 故障恢复 | 场景回滚 + 重新初始化 |
| 网络需求 | 可选 |
| 延迟 | < 1ms |

| 模块名 | Capabilities | 内嵌 Adapter | 内嵌 Plugin | 依赖 Tool | 依赖 Service |
|--------|-------------|-------------|------------|----------|-------------|
| afsim-engine | page: sim-control, panel: entity-status, toolbar: sim-controls, statusBar: sim-time, chatCommand: /run | eventout_bridge, wsfcgi_adapter, wizard_config_adapter | wsf_brawler, wsf_air_combat, wsf_iads_c2, wsf_multiresolution, wsf_6dof, wsf_radar_clutter, wsf_optical, wsf_ew, wsf_battle_damage, guidance_computer | earth.*, math.*, units.* | dis_gateway, mil_domain, l16_gateway |
| engage | page: engagement-dashboard, panel: weapon-status, toolbar: engage-controls | sensor_plot_adapter | — | weapon.*, math.* | mil_domain, mtt_tracker |
| mission | page: mission-planner, panel: task-list, toolbar: mission-controls | — | — | earth.*, math.* | geodata_elevation, geodata_landuse |
| wizard | page: scenario-editor, panel: project-tree, toolbar: wizard-tools | wizard_config_adapter | ScenarioAnalyzer, TaskList, MysticLauncher | earth.* | afsim-engine (via wsfcgi) |
| mystic | page: analysis-dashboard, panel: result-browser, toolbar: time-controls | mystic_result_adapter, warlock_event_adapter | ResultTimeController, TaskAssigner, TaskStatus, Log, WsfPrompt | math.* | result_db |
| wsf_space | page: orbital-view, panel: satellite-list, toolbar: orbit-controls | — | — | earth.*, math.* | space_domain |
| sensor_manager | page: sensor-coverage, panel: detection-log, toolbar: sensor-controls | — | wsf_radar_clutter, wsf_optical | earth.*, math.* | mil_domain |
| weapon_manager | page: weapon-inventory, panel: engagement-log, toolbar: weapon-controls | — | guidance_computer, wsf_battle_damage | weapon.*, math.* | mil_domain |

### 10.6 Plugin（插件）

| 项目 | 内容 |
|------|------|
| 数量 | 20 个，~1,490 方法 |
| 部署形态 | 宿主 Skill 子目录 |
| 注册方式 | Skill manifest plugins 字段 |
| 调用方式 | 宿主钩子回调 |
| 健康检查 | 不需要（随宿主） |
| 故障恢复 | 降级（跳过插件功能） |
| 网络需求 | 无 |
| 延迟 | < 0.1ms |

| 插件名 | 宿主 | 注册钩子 | AddType 调用 | 扩展能力 | 嵌入式 Tool |
|--------|------|---------|------------|---------|-----------|
| wsf_brawler | afsim-engine | processor:create | WsfProcessor::AddType("WsfBrawlerProcessor") | 空战机动模型 | earth.*, math.linear, math.rotation |
| wsf_air_combat | afsim-engine | processor:create | WsfProcessor::AddType("WsfSA_Processor") | SA处理器/态势感知 | earth.*, math.linear |
| wsf_iads_c2 | afsim-engine | processor:create | WsfProcessor::AddType("WsfBattleManager") | IADS C2 指挥控制 | earth.* |
| wsf_multiresolution | afsim-engine | platform:create | WsfPlatform::AddType("WsfLOD_Platform") | 多分辨率建模 | — |
| wsf_6dof | afsim-engine | mover:create | WsfMover::AddType("WsfGuidedMover") | 六自由度运动模型 | UtVec3, UtMat3, UtDCM (极高频) |
| wsf_simple_platform | afsim-engine | platform:create | WsfPlatform::AddType("WsfSimplePlatform") | 简单平台模型 | — |
| wsf_radar_clutter | afsim-engine | sensor:create | WsfSensor::AddType("WsfRadarClutterProcessor") | 雷达杂波模型 | UtNoise, UtMath |
| wsf_optical | afsim-engine | sensor:create | WsfSensor::AddType("WsfOpticalSensor") | 光学传感器模型 | UtMath, UtVec3 |
| wsf_ew | afsim-engine | processor:create | WsfProcessor::AddType("WsfEW_Processor") | 电子战模型 | — |
| wsf_battle_damage | afsim-engine | processor:create | WsfProcessor::AddType("WsfBattleDamageProcessor") | 战损评估 | — |
| guidance_computer | afsim-engine | processor:create | WsfProcessor::AddType("WsfGuidanceComputer") | 制导计算机 | weapon.guidance |
| classification_proc | afsim-engine | processor:create | WsfProcessor::AddType("WsfClassificationProcessor") | 目标分类处理器 | — |
| scenario_analyzer | wizard | plugin:load | Wizard::LoadPlugin("ScenarioAnalyzerPlugin") | 场景分析 | — |
| task_list | wizard | plugin:load | Wizard::LoadPlugin("TaskListPlugin") | 任务列表 | — |
| mystic_launcher | wizard | plugin:load | Wizard::LoadPlugin("MysticLauncherPlugin") | Mystic 启动器 | — |
| wsf_prompt | mystic | plugin:load | Mystic::LoadPlugin("WsfPromptPlugin") | 仿真提示 | — |
| result_time_controller | mystic | plugin:load | Mystic::LoadPlugin("ResultTimeControllerPlugin") | 结果时间控制 | — |
| task_assigner | mystic | plugin:load | Mystic::LoadPlugin("TaskAssignerPlugin") | 任务分配 | — |
| task_status | mystic | plugin:load | Mystic::LoadPlugin("TaskStatusPlugin") | 任务状态 | — |
| log_viewer | mystic | plugin:load | Mystic::LoadPlugin("LogPlugin") | 日志查看 | — |

### 10.7 Pipeline（管道）

| 项目 | 内容 |
|------|------|
| 数量 | 10 个，49 阶段 |
| 部署形态 | 声明式 Action Chain |
| 注册方式 | Skill manifest pipelines 字段 |
| 调用方式 | `actionRouter.chain(actions[])` |
| 健康检查 | 不需要（随宿主） |
| 故障恢复 | 重放（从失败阶段重新执行） |
| 网络需求 | 无（各阶段可能跨节点调用 Tool） |
| 延迟 | 各阶段延迟之和 |

| 管道名 | 宿主 | 阶段 | 阶段数 | 内嵌 Tool/Service |
|--------|------|------|--------|----------------|
| sensor_detection | afsim-engine | 信号生成 → MTI/MTD处理 → 检测判决 → 航迹关联 | 4 | earth.*, math.*, mtt_tracker |
| weapon_engagement | engage | 目标选择 → 发射计算 → 弹道仿真 → 命中判定 → 效果评估 | 5 | weapon.*, mil_domain |
| mission_planning | mission | 任务分解 → 资源分配 → 路径规划 → 执行调度 → 进度监控 | 5 | earth.*, geodata_elevation |
| post_processing | mystic | 事件读取 → 过滤 → 聚合 → 统计分析 → 报告生成 | 5 | math.statistics, result_db |
| ripr_job | afsim-engine | 作业提交 → 分发 → 执行 → 结果收集 → 合并 | 5 | xio_job_driver |
| orbit_propagation | wsf_space | 初始轨道 → 摄动计算 → 机动规划 → 覆盖计算 → 结果输出 | 5 | earth.gravity, math.* |
| cyber_attack | cyber_service | 漏洞扫描 → 攻击路径 → 效果注入 → 影响评估 → 恢复模拟 | 5 | — |
| track_fusion | mtt_tracker | 多源接收 → 时间对齐 → 关联匹配 → 状态融合 → 航迹输出 | 5 | math.linear |
| dis_pdu_processing | dis_driver | PDU接收 → 解码 → 实体匹配 → 状态更新 → 事件发射 | 5 | dis.encoding |
| l16_message_processing | l16_driver | 消息接收 → 解码 → 验证 → 路由 → 事件发射 | 5 | l16.encoding |

### 10.8 Repository（仓库）

| 项目 | 内容 |
|------|------|
| 数量 | 6 个，~15 Tool + 10 Resource |
| 部署形态 | MCP Resource 服务 |
| 注册方式 | `McpToolRegistry.register_server()` + resources |
| 调用方式 | `McpToolRegistry.read_resource(server, uri)` 或 `call_tool(server, query_tool, args)` |
| 健康检查 | 可选 |
| 故障恢复 | 重新加载 / 使用缓存 |
| 网络需求 | 可选 |
| 延迟 | 1-50ms |

| 仓库名 | 数据类型 | 数据量 | Resource URI | Tool | 缓存策略 | 消费者 |
|--------|---------|--------|-------------|------|---------|--------|
| geodata_elevation | DTED 高程 | 10-100 GB | geo://elevation?lat={}&lon={} | query_elevation, query_slope | Tile 缓存 | mission |
| geodata_landuse | 土地利用 | 1-10 GB | geo://landuse?lat={}&lon={} | query_land_use | Tile 缓存 | mission |
| geodata_bathymetry | 水深数据 | 1-10 GB | geo://bathymetry?lat={}&lon={} | query_depth | Tile 缓存 | mission |
| geodata_imagery | GeoTIFF 影像 | 10-100 GB | geo://imagery?bounds={}&zoom={} | query_imagery | LRU 缓存 | wizard, mystic |
| result_db | 仿真结果 | 1-100 GB | result://runs/{id}/events | query_events, query_tracks | 内存映射 | mystic, engage |
| project_workspace | 项目文件 | 10-500 MB | project://files/{path} | list_files, search_project | 文件类型缓存 | wizard |

### 10.9 全景统计

| 组件类型 | 数量 | 接口数 | 部署形态 | 独立进程 | 健康检查 |
|---------|------|--------|---------|---------|---------|
| Tool | 23 域 | ~1,105 | WASM/FFI | ❌ | ❌ |
| Adapter | 10 | ~10 | 嵌入宿主 | ❌ | ❌ |
| Driver | 5 | ~34 | 独立进程 | ✅ | ✅ |
| Service | 6 | ~805 | MCP 子进程 | ✅ | ✅ |
| Module | 8 | ~32 Cap | Skill iframe | ❌ | ✅ |
| Plugin | 20 | ~1,490 | Skill 子目录 | ❌ | ❌ |
| Pipeline | 10 | 49 阶段 | Action Chain | ❌ | ❌ |
| Repository | 6 | ~25 | MCP Resource | ✅ | 可选 |
| **总计** | **88** | **~3,550** | | | |

---

## 11. 实现优先级与工作量评估

### 11.1 实现优先级

| 优先级 | 任务 | 基于现有代码 | 工作量 | 理由 |
|--------|------|------------|--------|------|
| **P0** | afsim-engine Module + embedded Tool (earth/math/units) | Skill 架构 | 大 | 核心仿真引擎，必须先完成 |
| **P0** | DistributedToolRegistry (三级路由) | McpToolRegistry | 中 | 统一调用入口，支持三级路由 |
| **P0** | DistributedBridge (Skill 间通信) | MessageGateway + skillEventBus | 中 | 分布式 Skill 通信核心 |
| **P0** | eventout_bridge Adapter | skillEventBus | 小 | 仿真事件→Skill 事件桥接 |
| **P0** | dis_gateway + dis_driver 部署 | MTT Runtime | 中 | DIS 是核心外部接口 |
| **P1** | engage Module + weapon.* Tool | afsim-engine | 中 | 武器交战核心功能 |
| **P1** | mil_domain Service 部署 | McpToolRegistry | 中 | 军事领域知识服务 |
| **P1** | ServiceOrchestrator (服务编排) | HealthCheckRegistry | 中 | 服务生命周期管理 |
| **P1** | mission Module 部署 | afsim-engine | 中 | 任务规划核心功能 |
| **P1** | mtt_tracker Service (异步优化) | McpToolRegistry | 中 | 航迹管理，异步模式 |
| **P1** | geodata Repository 部署 | McpToolRegistry | 中 | 地形数据服务 |
| **P2** | wsf_space Module 部署 | afsim-engine | 中 | 太空领域 |
| **P2** | cyber_service + fusion_center 部署 | McpToolRegistry | 中 | 网络战+融合 |
| **P2** | wizard Module + Plugin 部署 | Skill 架构 | 中 | 场景编辑 |
| **P2** | mystic Module + Pipeline 部署 | Skill 架构 | 中 | 后处理分析 |
| **P2** | l16_gateway + l16_driver 部署 | dis_gateway 模式 | 中 | Link16 数据链 |
| **P2** | Store CRDT 跨节点同步 | CrdtEngine | 中 | 状态一致性 |

### 11.2 开发里程碑

```
里程碑 1: 核心仿真引擎 (M0)
  ├── afsim-engine Module (Skill)
  ├── earth.*/math.*/units.* Tool (WASM 嵌入式)
  ├── eventout_bridge Adapter
  ├── dis_gateway + dis_driver
  └── 基本 Skill 通信 (同节点)

里程碑 2: 武器交战系统 (M1)
  ├── engage Module
  ├── weapon.ballistic/guidance Tool
  ├── weapon_engagement Pipeline
  ├── sensor_plot_adapter
  └── mil_domain Service (基础)

里程碑 3: 任务规划系统 (M2)
  ├── mission Module
  ├── mission_planning Pipeline
  ├── geodata Repository
  └── mtt_tracker Service (异步模式)

里程碑 4: 完整领域覆盖 (M3)
  ├── wsf_space Module + space_domain Service
  ├── cyber_service + fusion_center
  ├── wizard Module + 3 Plugin
  └── mystic Module + post_processing Pipeline

里程碑 5: 分布式部署 (M4)
  ├── 跨节点 DistributedBridge
  ├── 跨节点 Tool 调用
  ├── Store CRDT 同步
  └── Service 降级策略
```

### 11.3 关键技术风险

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| WASM Tool 性能 | 嵌入式 Tool 必须达到 < 0.1ms/调用 | 使用 Rust WASM 编译 UtVec3 等高频工具，预分配内存 |
| DIS 同步延迟 | 若同步延迟 > 100ms，仿真精度下降 | 异步缓冲 + 插值外推 |
| mtt_tracker 异步模式 | 航迹更新滞后导致交战决策延迟 | N=10 帧批量，延迟 167ms (60Hz)，可接受 |
| Plugin 热加载 | 动态加载 Plugin 增加复杂度 | 初期静态编译，成熟后支持热加载 |
| 多节点 CRDT 一致性 | 并发写入可能冲突 | 使用 HLC 替代递增 ID |

---

## 附录 A: Tool 完整清单（23 域，~1,105 方法）

### A.1 earth.* 域 (~73 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| earth.coordinates | ~40 | geodetic_to_geocentric, geocentric_to_geodetic, great_circle_distance, geodesic_distance, local_geodetic_frame |
| earth.gravity | ~10 | get_gravity_acceleration, get_gravity_model, get_egm96_coefficients |
| earth.magnetic | ~8 | get_magnetic_field, get_declination, get_inclination |
| earth.atmosphere | ~15 | get_density, get_pressure, get_temperature, get_speed_of_sound, get_wind_velocity |

### A.2 math.* 域 (~265 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| math.linear | ~120 | dot_product, cross_product, normalize, magnitude, distance, angle_between, project, reject |
| math.rotation | ~60 | multiply, transpose, rotate, extract_euler_angles, to_quaternion, from_quaternion |
| math.interpolation | ~30 | linear_interpolate, cubic_interpolate, spline_interpolate, lookup, bilinear_interpolate |
| math.statistics | ~15 | mean, variance, standard_deviation, correlation, covariance, regression |
| math.noise | ~20 | gaussian_random, uniform_random, perlin_noise, pink_noise, markov_process |
| math.complex | ~20 | add, subtract, multiply, divide, magnitude, phase, conjugate |

### A.3 units.* 域 (~120 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| units.conversion | ~60 | convert, convert_to_standard, convert_from_standard, parse, format |
| units.angle | ~25 | convert, normalize, difference, to_radians, to_degrees, wrap |
| units.speed | ~15 | convert, mach_to_speed, speed_to_mach, knots_to_mps |
| units.time | ~20 | convert, format, parse, add, subtract, difference |

### A.4 weapon.* 域 (~112 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| weapon.ballistic | ~40 | ballistic_launch_compute, trajectory_calculate, impact_point_predict, time_of_flight |
| weapon.guidance | ~72 | guidance_gain_compute, proportional_nav, pure_pursuit, line_of_sight, bang_bang |

### A.5 dis.* 域 (~430 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| dis.encoding | ~200 | serialize, deserialize, get_pdu_length, validate, get_pdu_type |
| dis.entity | ~50 | get_entity, set_entity, compare, create_id, parse_id |
| dis.radio | ~80 | set_transmitter_frequency, set_radio_type, set_modulation, transmit |
| dis.warfare | ~100 | set_fuse_type, set_warhead_type, set_detonation_result |

### A.6 l16.* 域 (~30 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| l16.encoding | ~30 | encode_j_series, decode_j_series, validate_message, get_message_type |

### A.7 geo.* 域 (~65 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| geo.elevation | ~30 | query_elevation, query_slope, query_aspect, query_batch |
| geo.landuse | ~20 | query_land_use, query_land_cover, query_terrain_type |
| geo.bathymetry | ~15 | query_depth, query_sea_floor, query_tidal_range |

### A.8 script.* 域 (~200 方法)

| 子域 | 方法数 | 示例方法 |
|------|--------|---------|
| script.engine | ~200 | execute, compile, get_variable, set_variable, call_function |

---

## 附录 B: Plugin 加载机制详解

### B.1 WsfPluginManager 源码分析

根据 [WsfPluginManager.cpp:72-149](file:///d:/Programe/Afsim 2.9/afsim-2.9.0-win64/swdev/src/core/wsf/source/WsfPluginManager.cpp#L72)，Plugin 加载流程如下：

```
WsfPluginManager::LoadPlugins()
│
├── 1. 遍历 mPluginSearchPaths 列表
│     │
│     ├── 若路径是文件: LoadPlugin(path)
│     └── 若路径是目录: LoadAll(directory, recursive)
│
└── 2. 对每个 Plugin 调用 LoadPluginInitialize()

LoadPluginInitialize(library, filename)
│
├── 1. 获取版本函数: WsfPluginVersion* versionFunc = GetSymbol("WsfPluginVersion")
│
├── 2. 调用版本函数: (*versionFunc)(version)
│     └── 获取 Plugin 声明的 API 版本号
│
├── 3. 验证版本: ValidateVersion(version)
│     └── 检查主版本号、次版本号、编译器兼容性
│
├── 4. 获取 Setup 函数: SetupFuncPtr setupFunc = GetSymbol("WsfPluginSetup")
│
├── 5. 调用 Setup 函数: (*setupFunc)(WsfApplication&)
│     └── Plugin 在此注册 AddType
│
└── 6. 析构时调用 Complete 函数: WSF_PluginComplete
```

### B.2 Plugin SetupFuncPtr 典型实现

```
extern "C" void WsfPluginSetup(WsfApplication& app)
{
    // 注册 Processor 类型
    WsfProcessor::AddType("WsfBrawlerProcessor", WsfBrawlerProcessor::Factory);
    WsfProcessor::AddType("WsfSA_Processor", WsfSA_Processor::Factory);
    WsfProcessor::AddType("WsfBattleManager", WsfBattleManager::Factory);

    // 注册 Platform 类型
    WsfPlatform::AddType("WsfLOD_Platform", WsfLOD_Platform::Factory);

    // 注册 Mover 类型
    WsfMover::AddType("WsfGuidedMover", WsfGuidedMover::Factory);

    // 注册 Sensor 类型
    WsfSensor::AddType("WsfOpticalSensor", WsfOpticalSensor::Factory);
}

extern "C" void WsfPluginComplete(WsfApplication& app)
{
    // 清理资源
}
```

### B.3 AddType 注册体系

```
WsfProcessor::AddType(name, factory)
  └── WsfProcessorTypeFactory::Register(name, factory)
        └── 存储在 static std::map<std::string, FactoryFunc>

WsfPlatform::AddType(name, factory)
  └── 同上

WsfMover::AddType(name, factory)
  └── 同上

WsfSensor::AddType(name, factory)
  └── 同上

场景加载时:
  WsfScenario::Load() → 解析 .scenario XML
    → 遇到 <Processor Type="WsfBrawlerProcessor">
    → 调用 WsfProcessor::Create("WsfBrawlerProcessor")
    → 通过 AddType 注册的 Factory 创建实例
```

---

## 12. Rust 高性能计算引擎架构

### 12.1 重构动机与目标

#### 12.1.1 AFSIM C++ 性能瓶颈

| 瓶颈 | C++ 现状 | Rust 优势 |
|------|---------|----------|
| 内存安全 | 手动 new/delete，悬垂指针风险 | 所有权系统编译期保证，零成本抽象 |
| 并行计算 | OpenMP pragma，数据竞争难检测 | Send/Sync trait 编译期禁止数据竞争 |
| SIMD 向量化 | 编译器 intrinsics，平台耦合 | `std::simd` / `packed_simd2` 跨平台抽象 |
| WASM 编译 | Emscripten 体积大、性能损失 | `wasm-pack` 原生支持，WASM SIMD 自动启用 |
| 跨平台构建 | CMake 工具链复杂 | Cargo 统一构建，交叉编译零配置 |
| FFI 边界 | extern "C" 手动绑定 | `cbindgen` 自动生成 C 头文件，`ffi` crate 零开销 |

#### 12.1.2 重构目标

```
性能目标:
  ├── earth.coordinates: < 10ns/调用 (C++ ~20ns, 目标 2x)
  ├── math.linear:       < 5ns/调用  (C++ ~10ns, 目标 2x)
  ├── math.rotation:     < 15ns/调用 (C++ ~30ns, 目标 2x)
  ├── weapon.ballistic:  < 200μs/发射 (C++ ~500μs, 目标 2.5x)
  ├── weapon.guidance:   < 50μs/帧/导弹 (C++ ~100μs, 目标 2x)
  ├── dis.encoding:      < 1μs/PDU序列化 (C++ ~3μs, 目标 3x)
  └── space.orbital:     < 2ms/积分步 (C++ ~5ms, 目标 2.5x)

架构目标:
  ├── 模型与引擎完全解耦 ← 第 13 章
  ├── 算法与数据完全分离 ← 第 14 章
  ├── 编译期类型安全
  ├── 零运行时开销
  └── WASM + Native 双目标编译
```

### 12.2 Rust Compute Engine Crate 架构

#### 12.2.1 Crate 分层设计

```
afsim-compute/                          ← Rust workspace root
│
├── crates/
│   ├── afsim-core/                     ← 核心类型定义（纯数据，零算法）
│   │     src/
│   │     ├── types.rs                  ← Vec3, Mat3, DCM, Quat, GeoCoord
│   │     ├── units.rs                  ← 物理量类型系统 (Length, Speed, Angle, Time)
│   │     ├── earth_model.rs            ← 地球模型枚举 (WGS84, Spherical)
│   │     ├── atmosphere_model.rs       ← 大气模型枚举 (Standard, UserDefined)
│   │     ├── gravity_model.rs          ← 重力模型枚举
│   │     ├── sensor_params.rs          ← 传感器参数 (纯数据)
│   │     ├── weapon_params.rs          ← 武器参数 (纯数据)
│   │     ├── platform_state.rs         ← 平台状态 (纯数据)
│   │     └── lib.rs
│   │
│   ├── afsim-earth/                    ← 地球计算算法（纯算法，零数据）
│   │     src/
│   │     ├── coordinates.rs            ← 坐标转换算法
│   │     ├── gravity.rs                ← 重力计算算法
│   │     ├── atmosphere.rs             ← 大气模型算法
│   │     ├── magnetic.rs               ← 地磁计算算法
│   │     ├── geodesic.rs               ← 大地测量算法
│   │     ├── simd_helpers.rs           ← SIMD 批量坐标转换
│   │     └── lib.rs
│   │
│   ├── afsim-math/                     ← 数学计算算法（纯算法，零数据）
│   │     src/
│   │     ├── linear.rs                 ← 线性代数 (Vec3, Mat3 操作)
│   │     ├── rotation.rs               ← 旋转 (DCM, Quat 操作)
│   │     ├── interpolation.rs          ← 插值算法
│   │     ├── statistics.rs             ← 统计算法
│   │     ├── noise.rs                  ← 噪声生成算法
│   │     ├── simd_vec3.rs              ← SIMD 批量 Vec3 运算
│   │     ├── simd_mat3.rs              ← SIMD 批量 Mat3 运算
│   │     └── lib.rs
│   │
│   ├── afsim-units/                    ← 单位转换算法（纯算法，零数据）
│   │     src/
│   │     ├── conversion.rs             ← 通用转换算法
│   │     ├── angle.rs                  ← 角度转换算法
│   │     ├── speed.rs                  ← 速度转换算法
│   │     ├── time.rs                   ← 时间转换算法
│   │     └── lib.rs
│   │
│   ├── afsim-weapon/                   ← 武器计算算法（纯算法，零数据）
│   │     src/
│   │     ├── ballistic.rs              ← 弹道计算算法
│   │     ├── guidance.rs               ← 制导算法
│   │     ├── launch_computer.rs        ← 发射计算算法
│   │     ├── proportional_nav.rs       ← 比例导引算法
│   │     ├── simd_trajectory.rs        ← SIMD 批量弹道积分
│   │     └── lib.rs
│   │
│   ├── afsim-dis/                      ← DIS 协议算法（纯算法，零数据）
│   │     src/
│   │     ├── encoding.rs               ← PDU 编解码算法
│   │     ├── entity.rs                 ← 实体管理算法
│   │     ├── radio.rs                  ← 无线电 PDU 算法
│   │     ├── warfare.rs                ← 交战 PDU 算法
│   │     ├── simd_serialize.rs         ← SIMD 批量序列化
│   │     └── lib.rs
│   │
│   ├── afsim-orbital/                  ← 轨道计算算法（纯算法，零数据）
│   │     src/
│   │     ├── propagation.rs            ← 轨道传播算法
│   │     ├── two_body.rs               ← 二体问题算法
│   │     ├── perturbation.rs           ← 摄动算法
│   │     ├── coverage.rs               ← 覆盖计算算法
│   │     └── lib.rs
│   │
│   ├── afsim-data/                     ← 数据层（纯数据，零算法）
│   │     src/
│   │     ├── gravity_tables.rs         ← 重力系数表 (EGM96)
│   │     ├── atmosphere_tables.rs      ← 标准大气表
│   │     ├── magnetic_tables.rs        ← 地磁系数表 (IGRF)
│   │     ├── weapon_db.rs              ← 武器性能数据库
│   │     ├── sensor_db.rs              ← 传感器性能数据库
│   │     ├── platform_db.rs            ← 平台数据库
│   │     ├── geodata_cache.rs          ← 地理数据缓存
│   │     └── lib.rs
│   │
│   ├── afsim-engine/                   ← 仿真引擎（编排层，零计算）
│   │     src/
│   │     ├── frame_step.rs             ← 帧步进编排
│   │     ├── platform_manager.rs       ← 平台生命周期管理
│   │     ├── event_scheduler.rs        ← 事件调度器
│   │     ├── observer.rs               ← 观察者模式
│   │     ├── scenario_loader.rs        ← 场景加载（调用 data 层）
│   │     ├── plugin_host.rs            ← Plugin 宿主
│   │     └── lib.rs
│   │
│   ├── afsim-wasm/                     ← WASM 绑定层
│   │     src/
│   │     ├── earth_wasm.rs             ← earth.* Tool WASM 导出
│   │     ├── math_wasm.rs              ← math.* Tool WASM 导出
│   │     ├── weapon_wasm.rs            ← weapon.* Tool WASM 导出
│   │     └── lib.rs
│   │
│   └── afsim-ffi/                      ← C FFI 绑定层
│         src/
│         ├── earth_ffi.rs              ← earth.* Tool C 导出
│         ├── math_ffi.rs               ← math.* Tool C 导出
│         ├── weapon_ffi.rs             ← weapon.* Tool C 导出
│         └── lib.rs
│
├── Cargo.toml                          ← workspace 定义
└── Makefile                            ← 构建目标 (native/wasm/ffi)
```

#### 12.2.2 依赖关系图（严格无环）

```
afsim-wasm ──→ afsim-earth ──→ afsim-core
afsim-ffi  ──→ afsim-math  ──→ afsim-core
               afsim-weapon ──→ afsim-core
               afsim-dis   ──→ afsim-core
               afsim-orbital──→ afsim-core
               afsim-units ──→ afsim-core

afsim-engine ──→ afsim-earth  (调用算法)
             ──→ afsim-math   (调用算法)
             ──→ afsim-weapon (调用算法)
             ──→ afsim-dis    (调用算法)
             ──→ afsim-orbital(调用算法)
             ──→ afsim-units  (调用算法)
             ──→ afsim-data   (加载数据)
             ──→ afsim-core   (使用类型)

afsim-data ──→ afsim-core (使用类型，不依赖任何算法 crate)

关键约束:
  ├── afsim-core: 零依赖（纯类型定义）
  ├── afsim-data: 仅依赖 afsim-core（纯数据）
  ├── afsim-earth/math/weapon/dis/orbital/units: 仅依赖 afsim-core（纯算法）
  ├── afsim-engine: 依赖所有算法 crate + data crate（编排层）
  ├── 算法 crate 之间: 零互依赖
  └── 数据 crate 与算法 crate: 零互依赖
```

### 12.3 SIMD 加速策略

#### 12.3.1 批量 Vec3 运算（最热点）

AFSIM 每帧约 5,700 次 Tool 调用中，Vec3 运算占 ~60%。通过 SIMD 批量化可将每帧调用次数从 5,700 降至 ~700 次（8x 批量）。

```rust
// afsim-math/src/simd_vec3.rs

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[derive(Clone, Copy, Debug)]
#[repr(C, align(32))]
pub struct SimdVec3 {
    x: __m256,
    y: __m256,
    z: __m256,
}

impl SimdVec3 {
    pub fn magnitude(&self) -> [f64; 4] {
        unsafe {
            let xx = _mm256_mul_pd(self.x, self.x);
            let yy = _mm256_mul_pd(self.y, self.y);
            let zz = _mm256_mul_pd(self.z, self.z);
            let sum = _mm256_add_pd(_mm256_add_pd(xx, yy), zz);
            let result = _mm256_sqrt_pd(sum);
            let mut out = [0.0f64; 4];
            _mm256_storeu_pd(out.as_mut_ptr(), result);
            out
        }
    }

    pub fn dot(&self, other: &SimdVec3) -> [f64; 4] {
        unsafe {
            let xx = _mm256_mul_pd(self.x, other.x);
            let yy = _mm256_mul_pd(self.y, other.y);
            let zz = _mm256_mul_pd(self.z, other.z);
            let result = _mm256_add_pd(_mm256_add_pd(xx, yy), zz);
            let mut out = [0.0f64; 4];
            _mm256_storeu_pd(out.as_mut_ptr(), result);
            out
        }
    }

    pub fn distance(&self, other: &SimdVec3) -> [f64; 4] {
        unsafe {
            let dx = _mm256_sub_pd(self.x, other.x);
            let dy = _mm256_sub_pd(self.y, other.y);
            let dz = _mm256_sub_pd(self.z, other.z);
            let xx = _mm256_mul_pd(dx, dx);
            let yy = _mm256_mul_pd(dy, dy);
            let zz = _mm256_mul_pd(dz, dz);
            let sum = _mm256_add_pd(_mm256_add_pd(xx, yy), zz);
            let result = _mm256_sqrt_pd(sum);
            let mut out = [0.0f64; 4];
            _mm256_storeu_pd(out.as_mut_ptr(), result);
            out
        }
    }
}
```

#### 12.3.2 批量坐标转换

```rust
// afsim-earth/src/simd_helpers.rs

pub fn batch_geodetic_to_geocentric(
    lats: &[f64],
    lons: &[f64],
    alts: &[f64],
    model: &WGS84,
) -> Vec<Vec3> {
    assert_eq!(lats.len(), lons.len());
    assert_eq!(lats.len(), alts.len());
    let n = lats.len();
    let mut results = Vec::with_capacity(n);

    let chunk_size = 4;
    let chunks = n / chunk_size;

    for i in 0..chunks {
        let base = i * chunk_size;
        unsafe {
            let lat_v = _mm256_loadu_pd(lats[base..].as_ptr());
            let lon_v = _mm256_loadu_pd(lons[base..].as_ptr());
            let alt_v = _mm256_loadu_pd(alts[base..].as_ptr());
            let sin_lat = _mm256_sin_pd(lat_v);
            let cos_lat = _mm256_cos_pd(lat_v);
            let sin_lon = _mm256_sin_pd(lon_v);
            let cos_lon = _mm256_cos_pd(lon_v);
            let n_rad = _mm256_add_pd(
                _mm256_set1_pd(model.a),
                _mm256_mul_pd(_mm256_set1_pd(model.f), sin_lat),
            );
            let x = _mm256_mul_pd(_mm256_add_pd(n_rad, alt_v), _mm256_mul_pd(cos_lat, cos_lon));
            let y = _mm256_mul_pd(_mm256_add_pd(n_rad, alt_v), _mm256_mul_pd(cos_lat, sin_lon));
            let z = _mm256_mul_pd(
                _mm256_add_pd(
                    _mm256_mul_pd(_mm256_set1_pd(1.0 - model.f), n_rad),
                    alt_v,
                ),
                sin_lat,
            );
            let mut ox = [0.0; 4]; let mut oy = [0.0; 4]; let mut oz = [0.0; 4];
            _mm256_storeu_pd(ox.as_mut_ptr(), x);
            _mm256_storeu_pd(oy.as_mut_ptr(), y);
            _mm256_storeu_pd(oz.as_mut_ptr(), z);
            for j in 0..chunk_size {
                results.push(Vec3::new(ox[j], oy[j], oz[j]));
            }
        }
    }
    for i in (chunks * chunk_size)..n {
        results.push(geodetic_to_geocentric(lats[i], lons[i], alts[i], model));
    }
    results
}
```

#### 12.3.3 SIMD 性能预估

| 操作 | 标量 (C++) | 标量 (Rust) | SIMD×4 (Rust) | 加速比 |
|------|-----------|------------|--------------|--------|
| Vec3::Magnitude | ~10 ns | ~8 ns | ~3 ns | 3.3x |
| Vec3::Distance | ~12 ns | ~10 ns | ~4 ns | 3.0x |
| Vec3::DotProduct | ~8 ns | ~6 ns | ~2 ns | 4.0x |
| Mat3::Multiply | ~30 ns | ~25 ns | ~10 ns | 3.0x |
| DCM::Rotate | ~20 ns | ~16 ns | ~6 ns | 3.3x |
| Geodetic→Geocentric | ~50 ns | ~40 ns | ~15 ns | 3.3x |
| PDU Serialize | ~3000 ns | ~1000 ns | ~400 ns | 3.0x |
| Trajectory Propagate (100步) | ~500 μs | ~200 μs | ~80 μs | 6.3x |

### 12.4 并行计算策略

#### 12.4.1 帧内并行（Rayon）

```rust
// afsim-engine/src/frame_step.rs

use rayon::prelude::*;

pub fn advance_frame(&mut self, frame_time: f64) {
    self.observer.notify_frame_starting(frame_time);

    // 阶段 1: Platform 更新 (可并行)
    self.platforms
        .par_iter_mut()
        .for_each(|platform| platform.update(frame_time));

    self.observer.notify_platforms_updated(frame_time);

    // 阶段 2: Comm 更新 (可并行)
    self.comms
        .par_iter_mut()
        .for_each(|comm| comm.update(frame_time));

    // 阶段 3: Processor 更新 (可并行)
    self.processors
        .par_iter_mut()
        .for_each(|proc| proc.update(frame_time));

    // 阶段 4: Sensor 更新 (可并行)
    self.sensors
        .par_iter_mut()
        .for_each(|sensor| sensor.update(frame_time));

    // 阶段 5: 事件调度 (必须串行)
    self.event_scheduler.dispatch(frame_time);
}
```

#### 12.4.2 并行安全保证

```rust
// Send + Sync 编译期保证

// ✅ Vec3 是 Send + Sync (f64 字段)
assert_impl_all!(Vec3: Send, Sync);

// ✅ Mat3 是 Send + Sync
assert_impl_all!(Mat3: Send, Sync);

// ✅ PlatformState 是 Send + Sync (仅包含 f64/Vec3/枚举)
assert_impl_all!(PlatformState: Send, Sync);

// ❌ 编译期拒绝: 若 Platform 包含 Rc<RefCell<...>>
// error[E0277]: `Rc<RefCell<SensorManager>>` cannot be sent between threads safely

// 解决方案: 使用 Arc<RwLock<>> 或 Arc<Mutex<>>
pub struct Platform {
    state: PlatformState,                    // Send + Sync
    sensors: Vec<Sensor>,                    // Send + Sync (若 Sensor 是 Send+Sync)
    weapons: Arc<RwLock<Vec<Weapon>>>,       // 显式线程安全
    comm: Arc<Mutex<CommManager>>,           // 显式线程安全
}
```

#### 12.4.3 并行帧性能预估

| 场景 | 串行帧时间 | Rayon 4核 | Rayon 8核 | 加速比(8核) |
|------|-----------|----------|----------|------------|
| 100 平台 + 500 传感器 | 8.5 ms | 2.8 ms | 1.6 ms | 5.3x |
| 200 平台 + 1000 传感器 | 17 ms | 5.2 ms | 3.0 ms | 5.7x |
| 500 平台 + 2000 传感器 | 42 ms | 12 ms | 7.0 ms | 6.0x |
| 1000 平台 + 5000 传感器 | 85 ms | 23 ms | 13 ms | 6.5x |

### 12.5 WASM 双目标编译

#### 12.5.1 编译配置

```toml
# Cargo.toml (workspace)

[workspace]
members = [
    "crates/afsim-core",
    "crates/afsim-earth",
    "crates/afsim-math",
    "crates/afsim-units",
    "crates/afsim-weapon",
    "crates/afsim-dis",
    "crates/afsim-orbital",
    "crates/afsim-data",
    "crates/afsim-engine",
    "crates/afsim-wasm",
    "crates/afsim-ffi",
]

[profile.release]
opt-level = 3
lto = true
codegen-units = 1

[profile.release-wasm]
inherits = "release"
opt-level = 3
lto = true
```

#### 12.5.2 WASM 导出示例

```rust
// afsim-wasm/src/earth_wasm.rs

use wasm_bindgen::prelude::*;
use afsim_earth::coordinates;
use afsim_core::types::GeoCoord;

#[wasm_bindgen]
pub fn geodetic_to_geocentric(lat: f64, lon: f64, alt: f64) -> Vec<f64> {
    let result = coordinates::geodetic_to_geocentric(
        GeoCoord::new(lat, lon, alt),
        &WGS84::default(),
    );
    vec![result.x, result.y, result.z]
}

#[wasm_bindgen]
pub fn batch_geodetic_to_geocentric(lats: &[f64], lons: &[f64], alts: &[f64]) -> Vec<f64> {
    let results = coordinates::batch_geodetic_to_geocentric(lats, lons, alts, &WGS84::default());
    let mut flat = Vec::with_capacity(results.len() * 3);
    for r in results {
        flat.push(r.x);
        flat.push(r.y);
        flat.push(r.z);
    }
    flat
}

#[wasm_bindgen]
pub fn great_circle_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    coordinates::great_circle_distance(
        GeoCoord::new(lat1, lon1, 0.0),
        GeoCoord::new(lat2, lon2, 0.0),
        &WGS84::default(),
    )
}
```

#### 12.5.3 C FFI 导出示例

```rust
// afsim-ffi/src/earth_ffi.rs

use afsim_earth::coordinates;
use afsim_core::types::{Vec3, GeoCoord};

#[no_mangle]
pub extern "C" fn afsim_geodetic_to_geocentric(
    lat: f64, lon: f64, alt: f64,
    out_x: *mut f64, out_y: *mut f64, out_z: *mut f64,
) {
    let result = coordinates::geodetic_to_geocentric(
        GeoCoord::new(lat, lon, alt),
        &WGS84::default(),
    );
    unsafe {
        *out_x = result.x;
        *out_y = result.y;
        *out_z = result.z;
    }
}

#[no_mangle]
pub extern "C" fn afsim_great_circle_distance(
    lat1: f64, lon1: f64, lat2: f64, lon2: f64,
) -> f64 {
    coordinates::great_circle_distance(
        GeoCoord::new(lat1, lon1, 0.0),
        GeoCoord::new(lat2, lon2, 0.0),
        &WGS84::default(),
    )
}
```

### 12.6 Rust Compute Engine 与 AxAgent 集成

#### 12.6.1 嵌入式 Tool 注册

```rust
// 在 AxAgent Rust Runtime 中注册嵌入式 Tool

use afsim_wasm;

pub fn register_compute_tools(registry: &mut DistributedToolRegistry) {
    registry.register_embedded_batch("earth.coordinates", vec![
        EmbeddedTool {
            name: "geodetic_to_geocentric",
            handler: |args| {
                let lat: f64 = args["lat"].as_f64().unwrap();
                let lon: f64 = args["lon"].as_f64().unwrap();
                let alt: f64 = args["alt"].as_f64().unwrap();
                let result = afsim_earth::coordinates::geodetic_to_geocentric(
                    GeoCoord::new(lat, lon, alt), &WGS84::default(),
                );
                Ok(json!({"x": result.x, "y": result.y, "z": result.z}))
            },
        },
        EmbeddedTool {
            name: "great_circle_distance",
            handler: |args| { /* ... */ },
        },
    ]);

    registry.register_embedded_batch("math.linear", vec![
        EmbeddedTool {
            name: "vec3_magnitude",
            handler: |args| {
                let v: Vec3 = serde_json::from_value(args["vec"].clone())?;
                Ok(json!({"result": v.magnitude()}))
            },
        },
        EmbeddedTool {
            name: "vec3_distance",
            handler: |args| { /* ... */ },
        },
        EmbeddedTool {
            name: "vec3_dot_product",
            handler: |args| { /* ... */ },
        },
    ]);
}
```

#### 12.6.2 WASM Tool 在 Skill Sandbox 中加载

```typescript
// Skill Sandbox 中加载 WASM Tool

const wasmModule = await WebAssembly.instantiateStreaming(
  fetch('/tools/afsim-earth.wasm'),
  { env: { memory: new WebAssembly.Memory({ initial: 256 }) } }
);

const { geodetic_to_geocentric, batch_geodetic_to_geocentric } = wasmModule.instance.exports;

actionRouter.registerEmbeddedTool('earth.coordinates.geodetic_to_geocentric', (args) => {
  const result = geodetic_to_geocentric(args.lat, args.lon, args.alt);
  return { x: result[0], y: result[1], z: result[2] };
});
```

---

## 13. 模型与引擎完全解耦设计

### 13.1 解耦原则定义

#### 13.1.1 三层解耦架构

```
┌─────────────────────────────────────────────────────────┐
│                    模型层 (Model Layer)                    │
│  定义"是什么": 实体类型、物理参数、行为规范                    │
│  对应 crate: afsim-core (类型) + afsim-data (数据)        │
│  特征: 纯数据、零算法、零副作用、可序列化                      │
├─────────────────────────────────────────────────────────┤
│                    算法层 (Algorithm Layer)                │
│  定义"怎么算": 坐标转换、弹道积分、制导律、检测概率             │
│  对应 crate: afsim-earth/math/weapon/dis/orbital/units   │
│  特征: 纯函数、零状态、零 IO、可测试、可替换                    │
├─────────────────────────────────────────────────────────┤
│                    引擎层 (Engine Layer)                   │
│  定义"何时算": 帧调度、事件编排、生命周期管理                    │
│  对应 crate: afsim-engine                                │
│  特征: 编排逻辑、状态管理、IO 调度、不含任何计算                 │
└─────────────────────────────────────────────────────────┘
```

#### 13.1.2 解耦约束规则

| 规则 | 约束 | 违反示例 | 修正方式 |
|------|------|---------|---------|
| 模型不依赖算法 | afsim-core/data 不 import 任何算法 crate | core 中调用 `geodetic_to_geocentric()` | 将调用移到 engine 层 |
| 算法不依赖引擎 | afsim-earth/math 不 import afsim-engine | earth 中调用 `frame_time` | 将 frame_time 作为参数传入 |
| 算法不依赖数据 | afsim-earth/math 不 import afsim-data | math 中读取 `weapon_db` | 将数据作为参数传入算法 |
| 引擎不包含计算 | afsim-engine 不含任何数学公式 | engine 中内联弹道积分 | 调用 afsim-weapon |
| 模型可独立编译 | afsim-core 零外部依赖 | core 依赖 rayon | 移除依赖 |
| 算法可独立测试 | 纯函数输入→输出 | 算法读取全局状态 | 改为参数传入 |

### 13.2 模型层设计（afsim-core + afsim-data）

#### 13.2.1 核心类型系统

```rust
// afsim-core/src/types.rs

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Mat3 {
    pub data: [[f64; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct DCM {
    pub data: [[f64; 3]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoCoord {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EarthModel {
    WGS84,
    Spherical { radius: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Blue,
    Red,
    Neutral,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformId(pub String);

#[derive(Clone, Debug)]
pub struct PlatformState {
    pub id: PlatformId,
    pub side: Side,
    pub position: Vec3,
    pub velocity: Vec3,
    pub orientation: DCM,
    pub damage_factor: f64,
}
```

#### 13.2.2 物理量类型系统（零算法）

```rust
// afsim-core/src/units.rs

macro_rules! define_unit {
    ($name:ident, $base_unit:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
        pub struct $name(pub f64);

        impl $name {
            pub fn raw(&self) -> f64 { self.0 }
            pub fn from_raw(v: f64) -> Self { Self(v) }
        }

        impl std::ops::Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
        }

        impl std::ops::Sub for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
        }
    };
}

define_unit!(Length, "meters");
define_unit!(Speed, "meters_per_second");
define_unit!(Angle, "radians");
define_unit!(Time, "seconds");
define_unit!(Mass, "kilograms");
define_unit!(Force, "newtons");
define_unit!(Frequency, "hertz");
define_unit!(Power, "watts");
```

#### 13.2.3 数据层（afsim-data）

```rust
// afsim-data/src/weapon_db.rs

use afsim_core::types::PlatformId;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WeaponRecord {
    pub weapon_type: String,
    pub max_range: f64,
    pub min_range: f64,
    pub max_speed: f64,
    pub warhead_weight: f64,
    pub fuse_type: String,
    pub guidance_type: String,
    pub probability_of_hit: Vec<(f64, f64)>,
    pub lethality_radius: f64,
    pub reload_time: f64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WeaponDatabase {
    pub records: Vec<WeaponRecord>,
}

impl WeaponDatabase {
    pub fn load_from_json(path: &str) -> Result<Self, std::io::Error> {
        let data = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn find_by_type(&self, weapon_type: &str) -> Option<&WeaponRecord> {
        self.records.iter().find(|r| r.weapon_type == weapon_type)
    }
}
```

```rust
// afsim-data/src/gravity_tables.rs

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EGM96Coefficients {
    pub degree: usize,
    pub order: usize,
    pub c: Vec<Vec<f64>>,
    pub s: Vec<Vec<f64>>,
}

impl EGM96Coefficients {
    pub fn load_embedded() -> Self {
        const EGM96_C: &str = include_str!("../data/egm96_c.json");
        const EGM96_S: &str = include_str!("../data/egm96_s.json");
        Self {
            degree: 360,
            order: 360,
            c: serde_json::from_str(EGM96_C).unwrap(),
            s: serde_json::from_str(EGM96_S).unwrap(),
        }
    }
}
```

### 13.3 算法层设计（纯函数，零状态）

#### 13.3.1 算法 trait 约定

```rust
// afsim-earth/src/coordinates.rs

use afsim_core::types::{Vec3, GeoCoord, EarthModel};

pub fn geodetic_to_geocentric(
    coord: GeoCoord,
    model: &EarthModel,
) -> Vec3 {
    match model {
        EarthModel::WGS84 => geodetic_to_geocentric_wgs84(coord),
        EarthModel::Spherical { radius } => geodetic_to_geocentric_spherical(coord, *radius),
    }
}

fn geodetic_to_geocentric_wgs84(coord: GeoCoord) -> Vec3 {
    let a: f64 = 6_378_137.0;
    let f: f64 = 1.0 / 298.257223563;
    let e2 = 2.0 * f - f * f;
    let sin_lat = coord.lat.sin();
    let cos_lat = coord.lat.cos();
    let n = a / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    Vec3 {
        x: (n + coord.alt) * cos_lat * coord.lon.cos(),
        y: (n + coord.alt) * cos_lat * coord.lon.sin(),
        z: (n * (1.0 - e2) + coord.alt) * sin_lat,
    }
}

fn geodetic_to_geocentric_spherical(coord: GeoCoord, radius: f64) -> Vec3 {
    Vec3 {
        x: (radius + coord.alt) * coord.lat.cos() * coord.lon.cos(),
        y: (radius + coord.alt) * coord.lat.cos() * coord.lon.sin(),
        z: (radius + coord.alt) * coord.lat.sin(),
    }
}
```

#### 13.3.2 武器算法（纯函数，数据参数化）

```rust
// afsim-weapon/src/ballistic.rs

use afsim_core::types::{Vec3, DCM};

pub struct LaunchSolution {
    pub launch_azimuth: f64,
    pub launch_elevation: f64,
    pub time_of_flight: f64,
    pub impact_point: Vec3,
}

pub fn compute_ballistic_launch(
    launch_pos: Vec3,
    target_pos: Vec3,
    muzzle_velocity: f64,        // ← 来自 afsim-data::WeaponRecord
    gravity: f64,                // ← 来自 afsim-earth::gravity
    drag_coefficient: f64,       // ← 来自 afsim-data::WeaponRecord
    initial_mass: f64,           // ← 来自 afsim-data::WeaponRecord
    cross_section_area: f64,     // ← 来自 afsim-data::WeaponRecord
    air_density_at_launch: f64,  // ← 来自 afsim-earth::atmosphere
) -> LaunchSolution {
    let distance = (target_pos - launch_pos).magnitude();
    let flight_time = estimate_flight_time(distance, muzzle_velocity, gravity);
    let elevation = compute_launch_elevation(distance, muzzle_velocity, gravity, flight_time);
    let azimuth = compute_azimuth(launch_pos, target_pos);

    LaunchSolution {
        launch_azimuth: azimuth,
        launch_elevation: elevation,
        time_of_flight: flight_time,
        impact_point: target_pos,
    }
}

pub fn propagate_trajectory(
    position: Vec3,
    velocity: Vec3,
    dt: f64,
    gravity: f64,
    drag_coefficient: f64,
    mass: f64,
    cross_section_area: f64,
    air_density: f64,
) -> (Vec3, Vec3) {
    let speed = velocity.magnitude();
    let drag_force = 0.5 * air_density * speed * speed * drag_coefficient * cross_section_area;
    let drag_accel = drag_force / mass;
    let drag_decel = velocity.normalize() * (-drag_accel);
    let gravity_accel = Vec3::new(0.0, 0.0, -gravity);
    let accel = gravity_accel + drag_decel;
    let new_velocity = velocity + accel * dt;
    let new_position = position + velocity * dt + accel * 0.5 * dt * dt;
    (new_position, new_velocity)
}
```

#### 13.3.3 算法可替换性（Strategy 模式）

```rust
// afsim-weapon/src/guidance.rs

pub trait GuidanceLaw: Send + Sync {
    fn compute_acceleration(
        &self,
        missile_pos: Vec3,
        missile_vel: Vec3,
        target_pos: Vec3,
        target_vel: Vec3,
        time_to_go: f64,
    ) -> Vec3;
}

pub struct ProportionalNavigation {
    pub nav_constant: f64,
}

impl GuidanceLaw for ProportionalNavigation {
    fn compute_acceleration(
        &self,
        missile_pos: Vec3,
        missile_vel: Vec3,
        target_pos: Vec3,
        target_vel: Vec3,
        time_to_go: f64,
    ) -> Vec3 {
        let relative_pos = target_pos - missile_pos;
        let relative_vel = target_vel - missile_vel;
        let los_rate = relative_pos.cross(&relative_vel) / relative_pos.magnitude_squared();
        let closing_velocity = -relative_vel.dot(&relative_pos.normalize());
        self.nav_constant * closing_velocity * los_rate
    }
}

pub struct PurePursuit;

impl GuidanceLaw for PurePursuit {
    fn compute_acceleration(
        &self,
        missile_pos: Vec3,
        missile_vel: Vec3,
        target_pos: Vec3,
        _target_vel: Vec3,
        _time_to_go: f64,
    ) -> Vec3 {
        let to_target = (target_pos - missile_pos).normalize();
        let desired_vel = to_target * missile_vel.magnitude();
        (desired_vel - missile_vel).normalize()
    }
}

pub struct AugmentedProportionalNavigation {
    pub nav_constant: f64,
    pub augmentation_gain: f64,
}

impl GuidanceLaw for AugmentedProportionalNavigation {
    fn compute_acceleration(
        &self,
        missile_pos: Vec3,
        missile_vel: Vec3,
        target_pos: Vec3,
        target_vel: Vec3,
        time_to_go: f64,
    ) -> Vec3 {
        let pn = ProportionalNavigation { nav_constant: self.nav_constant };
        let pn_accel = pn.compute_acceleration(missile_pos, missile_vel, target_pos, target_vel, time_to_go);
        let target_accel = Vec3::ZERO;
        pn_accel + target_accel * self.augmentation_gain
    }
}
```

### 13.4 引擎层设计（编排，零计算）

#### 13.4.1 帧步进编排

```rust
// afsim-engine/src/frame_step.rs

use afsim_earth as earth;
use afsim_math as math;
use afsim_weapon as weapon;
use afsim_data as data;

pub struct FrameStepEngine {
    platforms: Vec<Platform>,
    gravity: f64,
    weapon_db: data::WeaponDatabase,
    atm_model: earth::AtmosphereModel,
}

impl FrameStepEngine {
    pub fn advance_frame(&mut self, frame_time: f64) {
        for platform in &mut self.platforms {
            // 引擎编排: 调用算法，传入数据
            let gravity_accel = earth::gravity::get_gravity_acceleration(
                platform.state.position,
                &earth::WGS84,
            );

            let air_density = earth::atmosphere::get_density(
                platform.state.position,
                &self.atm_model,
            );

            platform.state.velocity.z -= gravity_accel * frame_time;
            platform.state.position = platform.state.position
                + platform.state.velocity * frame_time;

            for weapon in &mut platform.weapons {
                if weapon.is_in_flight() {
                    let record = self.weapon_db.find_by_type(&weapon.weapon_type)
                        .expect("weapon type not found");

                    let (new_pos, new_vel) = weapon::ballistic::propagate_trajectory(
                        weapon.position,
                        weapon.velocity,
                        frame_time,
                        gravity_accel,
                        record.drag_coefficient,
                        record.initial_mass,
                        record.cross_section_area,
                        air_density,
                    );

                    weapon.position = new_pos;
                    weapon.velocity = new_vel;
                }
            }
        }
    }
}
```

#### 13.4.2 引擎与模型/算法的接口边界

```
引擎层 (afsim-engine) 的职责边界:

✅ 引擎应该做的:
  ├── 调度: 决定何时调用哪个算法
  ├── 编排: 将算法结果传递给下一个算法
  ├── 状态管理: 维护平台/武器/传感器的状态
  ├── 数据加载: 从 afsim-data 加载参数
  ├── 事件分发: 将计算结果转为事件通知
  └── 生命周期: 管理 Platform/Weapon/Sensor 的创建和销毁

❌ 引擎不应该做的:
  ├── 内联数学公式 (应调用 afsim-math)
  ├── 直接读取数据库 (应通过 afsim-data)
  ├── 硬编码物理常数 (应通过 afsim-core 类型)
  ├── 实现制导律 (应调用 afsim-weapon)
  └── 实现坐标转换 (应调用 afsim-earth)
```

### 13.5 模型-引擎解耦验证矩阵

| 验证项 | 检查方法 | 预期结果 |
|--------|---------|---------|
| afsim-core 零外部依赖 | `cargo tree -p afsim-core` | 仅显示自身 |
| afsim-data 不依赖算法 crate | `cargo tree -p afsim-data` | 不含 afsim-earth/math/weapon |
| 算法 crate 不依赖引擎 | `cargo tree -p afsim-earth` | 不含 afsim-engine |
| 算法 crate 不依赖数据 | `cargo tree -p afsim-math` | 不含 afsim-data |
| 算法函数无全局状态 | `grep "static mut" afsim-*/src/*.rs` | 零匹配 |
| 算法函数无 IO 操作 | `grep "std::fs\|std::net\|std::io" afsim-earth/src/*.rs` | 零匹配 |
| 引擎不含数学公式 | `grep "sin\|cos\|sqrt\|pow" afsim-engine/src/*.rs` | 零匹配 |
| 所有算法可独立测试 | `cargo test -p afsim-earth` | 100% 通过 |

---

## 14. 算法与数据分离架构

### 14.1 分离原则定义

#### 14.1.1 核心原则

```
算法（计算）与数据分离 = 函数与参数分离

算法 (Algorithm):
  ├── 纯函数: f(data) → result
  ├── 无副作用: 不修改输入，不写全局状态
  ├── 无 IO: 不读文件，不访问网络
  ├── 可缓存: 相同输入 → 相同输出
  ├── 可并行: 无共享状态 → 无锁
  └── 可替换: 同接口不同实现

数据 (Data):
  ├── 不可变: 算法读取数据但不修改
  ├── 可序列化: JSON/MessagePack/Binary
  ├── 可热加载: 运行时替换数据文件
  ├── 可版本化: 数据格式版本号
  └── 可校验: Schema 验证
```

#### 14.1.2 分离模式对比

| 模式 | 描述 | 适用场景 | 示例 |
|------|------|---------|------|
| 参数注入 | 算法函数签名包含数据参数 | 高频调用，数据量小 | `get_gravity(pos, &egm96_coeffs)` |
| 数据引用 | 算法接收 `&DataStore` 引用 | 中频调用，数据量中 | `get_max_range(weapon_type, &weapon_db)` |
| 查询回调 | 算法接收闭包按需查询 | 低频调用，数据量大 | `query_elevation(lat, lon, &geo_cache)` |
| 预计算表 | 数据预计算后作为查找表 | 固定模式，高频查询 | `sin_table[angle]` 代替 `angle.sin()` |

### 14.2 AFSIM 各域的算法-数据分离设计

#### 14.2.1 earth 域：参数注入模式

```
算法层 (afsim-earth):
  ├── geodetic_to_geocentric(coord, &EarthModel) → Vec3
  ├── great_circle_distance(coord1, coord2, &EarthModel) → f64
  ├── get_gravity_acceleration(pos, &GravityModel) → Vec3
  ├── get_density(alt, &AtmosphereModel) → f64
  └── get_magnetic_field(pos, &MagneticModel) → Vec3

数据层 (afsim-data):
  ├── WGS84_EARTH_MODEL: EarthModel = EarthModel::WGS84
  ├── EGM96_COEFFICIENTS: EGM96Coefficients (360×360 球谐系数)
  ├── STANDARD_ATMOSPHERE: AtmosphereTable (高度-密度-温度-压力)
  └── IGRF13_COEFFICIENTS: IGRFCoefficients (地磁模型)

引擎层 (afsim-engine):
  ├── 加载: let gravity_model = data::GravityModel::from_egm96(data::EGM96_COEFFICIENTS.clone());
  ├── 注入: let g = earth::get_gravity_acceleration(pos, &gravity_model);
  └── 缓存: 若 pos 不变，g 不变 → 缓存结果
```

#### 14.2.2 weapon 域：数据引用模式

```
算法层 (afsim-weapon):
  ├── compute_ballistic_launch(launch_pos, target_pos, muzzle_velocity, gravity, drag, mass, area, rho) → LaunchSolution
  ├── propagate_trajectory(pos, vel, dt, gravity, drag, mass, area, rho) → (Vec3, Vec3)
  ├── compute_guidance(missile_pos, missile_vel, target_pos, target_vel, ttg, &dyn GuidanceLaw) → Vec3
  └── assess_lethality(weapon_type, target_type, range, &WeaponDB) → f64

数据层 (afsim-data):
  ├── WeaponDatabase { records: Vec<WeaponRecord> }
  │     ├── weapon_type: "AIM-120C"
  │     ├── max_range: 75000.0
  │     ├── drag_coefficient: 0.35
  │     ├── initial_mass: 150.0
  │     ├── cross_section_area: 0.04
  │     ├── probability_of_hit: [(0.0, 0.0), (10000.0, 0.9), (75000.0, 0.3)]
  │     └── lethality_radius: 15.0
  └── LethalityTable { entries: Vec<(String, String, f64)> }

引擎层 (afsim-engine):
  ├── 加载: let weapon_db = data::WeaponDatabase::load_from_json("weapons.json")?;
  ├── 查询: let record = weapon_db.find_by_type("AIM-120C");
  ├── 注入: let solution = weapon::compute_ballistic_launch(
  │       launch_pos, target_pos,
  │       record.muzzle_velocity, gravity,
  │       record.drag_coefficient, record.initial_mass,
  │       record.cross_section_area, air_density,
  │   );
  └── 热加载: weapon_db.reload_from_json("weapons_v2.json")?;
```

#### 14.2.3 sensor 域：查询回调模式

```
算法层 (afsim-earth/math 用于 sensor):
  ├── compute_detection_probability(range, rcs, sensitivity, &NoiseModel) → f64
  ├── compute_radar_cross_section(aspect_angle, &RCSTable) → f64
  └── apply_clutter_filter(signal, &ClutterModel) → f64

数据层 (afsim-data):
  ├── RCS_TABLE: HashMap<(PlatformType, AspectAngle), f64>
  ├── NOISE_MODEL: NoiseParameters
  └── CLUTTER_MODEL: ClutterParameters

引擎层 (afsim-engine):
  ├── 传感器更新时:
  │   let rcs = data::RCS_TABLE.get(&(target_type, aspect_angle))
  │       .unwrap_or(&default_rcs);
  │   let p_d = math::statistics::detection_probability(
  │       range, *rcs, sensor_sensitivity, &data::NOISE_MODEL,
  │   );
  │   if random::<f64>() < p_d {
  │       self.on_detection(target, range);
  │   }
  └── 数据可运行时替换:
      data::RCS_TABLE.update_from_json("rcs_updated.json")?;
```

#### 14.2.4 DIS 域：预计算表模式

```
算法层 (afsim-dis):
  ├── serialize_entity_state(entity_state) -> Vec<u8>
  ├── deserialize_entity_state(data: &[u8]) -> EntityState
  └── compute_pdu_length(pdu_type) -> usize

数据层 (afsim-data):
  ├── PDU_LENGTH_TABLE: [usize; 64]  ← 预计算 PDU 长度
  ├── ENTITY_TYPE_DATABASE: HashMap<u32, EntityTypeRecord>
  └── RADIO_TYPE_DATABASE: HashMap<u32, RadioTypeRecord>

引擎层 (afsim-engine):
  ├── 序列化: let pdu_bytes = dis::serialize_entity_state(&entity.state);
  ├── 长度查表: let len = data::PDU_LENGTH_TABLE[pdu_type as usize];
  └── 批量: let batch = dis::batch_serialize(&entity_states);
```

### 14.3 数据热加载机制

#### 14.3.1 数据版本管理

```rust
// afsim-data/src/lib.rs

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DataVersion {
    pub schema_version: u32,
    pub data_version: u32,
    pub checksum: String,
    pub loaded_at: chrono::DateTime<chrono::Utc>,
}

pub trait ReloadableData: Send + Sync {
    fn data_version(&self) -> &DataVersion;
    fn reload(&mut self, path: &str) -> Result<DataVersion, std::io::Error>;
    fn validate(&self) -> Result<(), Vec<String>>;
}
```

#### 14.3.2 数据热加载流程

```
运行时数据更新:

  ① 文件监控线程检测到 weapons.json 变更
  ② 加载新数据到临时 WeaponDatabase
  ③ 验证新数据 (schema + 逻辑校验)
  ④ 原子替换:
     let mut db = weapon_db.write().unwrap();
     *db = new_db;
  ⑤ 广播 data:weapon_db_updated 事件
  ⑥ 引擎层收到事件，标记缓存失效
```

### 14.4 算法-数据分离验证矩阵

| 验证项 | 检查方法 | 预期结果 |
|--------|---------|---------|
| 算法函数无硬编码常数 | `grep -r "6378137\|298.257" afsim-earth/src/` | 零匹配 |
| 算法函数无文件读取 | `grep -r "fs::read\|include_str" afsim-earth/src/` | 零匹配 |
| 数据文件可独立修改 | 修改 weapons.json → 重新加载 → 结果变化 | ✅ |
| 算法可独立替换 | 替换 ProportionalNavigation 为 PurePursuit | ✅ 行为变化 |
| 数据格式有版本号 | `grep "schema_version" afsim-data/src/` | 非零匹配 |
| 所有数据可序列化 | `serde_json::to_string(&data)` | ✅ |
| 算法纯函数可缓存 | 相同输入 → 相同输出 | ✅ |

### 14.5 算法-数据分离与 8 类组件映射

| 组件类型 | 算法部分 | 数据部分 | 分离模式 |
|---------|---------|---------|---------|
| **Tool** | afsim-earth/math/weapon 算法函数 | afsim-data 参数表 | 参数注入 |
| **Adapter** | 转换逻辑 (纯函数) | 映射规则 (JSON) | 参数注入 |
| **Driver** | 协议编解码 (纯函数) | 连接参数 (配置) | 数据引用 |
| **Service** | 业务逻辑 (纯函数) | 领域数据库 | 查询回调 |
| **Module** | 编排逻辑 (引擎) | 场景配置 | 数据引用 |
| **Plugin** | 行为算法 (纯函数) | 行为参数 (配置) | 参数注入 |
| **Pipeline** | 各阶段算法 | 阶段参数 | 参数注入 |
| **Repository** | 查询优化算法 | 数据文件 | 查询回调 |

---

## 15. Rust 重构实施路线图

### 15.1 重构阶段划分

```
阶段 R0: 基础设施 (2 周)
  ├── 创建 afsim-compute workspace
  ├── 实现 afsim-core (类型定义)
  ├── 实现 afsim-data (数据加载)
  ├── 配置 CI/CD (cargo test + clippy + wasm-pack)
  └── 交付物: 可编译的 workspace + 类型系统

阶段 R1: 高频热点重构 (4 周)
  ├── 实现 afsim-math (Vec3/Mat3/DCM/Quat + SIMD)
  ├── 实现 afsim-earth (坐标转换 + 重力 + 大气)
  ├── 实现 afsim-units (单位转换)
  ├── 性能基准测试 vs C++ 原版
  ├── 实现 afsim-wasm (WASM 导出)
  └── 交付物: earth.*/math.*/units.* Tool 可用

阶段 R2: 武器与轨道重构 (4 周)
  ├── 实现 afsim-weapon (弹道 + 制导 + SIMD 批量)
  ├── 实现 afsim-orbital (轨道传播)
  ├── 实现 afsim-dis (PDU 编解码 + SIMD)
  ├── 性能基准测试 vs C++ 原版
  └── 交付物: weapon.*/dis.*/space.* Tool 可用

阶段 R3: 引擎编排层 (4 周)
  ├── 实现 afsim-engine (帧步进 + 并行)
  ├── 实现 Plugin 宿主 (动态加载)
  ├── 实现 Adapter 桥接
  ├── 实现 Pipeline 编排
  └── 交付物: 完整仿真引擎可运行

阶段 R4: AxAgent 集成 (4 周)
  ├── DistributedToolRegistry 注册嵌入式 Tool
  ├── WASM Tool 在 Skill Sandbox 中加载
  ├── C FFI 与遗留 AFSIM C++ 互操作
  ├── 数据热加载机制
  └── 交付物: AFSIM-AxAgent 完整集成

阶段 R5: 优化与验证 (2 周)
  ├── SIMD 性能调优
  ├── Rayon 并行调优
  ├── 内存分配优化 (arena/pool)
  ├── 完整场景回归测试
  └── 交付物: 生产就绪
```

### 15.2 遗留 C++ 互操作策略

#### 15.2.1 三阶段渐进替换

```
阶段 A: Rust Sidecar (R0-R1)
  ├── Rust 编译为 .dll/.so
  ├── C++ 通过 FFI 调用 Rust 函数
  ├── C++ 仍为主进程，Rust 为计算加速器
  └── 示例: C++ 调用 Rust 的 geodetic_to_geocentric

阶段 B: Rust Engine (R2-R3)
  ├── Rust 实现完整引擎
  ├── C++ Plugin 通过 FFI 回调注册
  ├── Rust 为主进程，C++ Plugin 为动态库
  └── 示例: Rust 引擎加载 wsf_brawler C++ Plugin

阶段 C: 全 Rust (R4-R5)
  ├── 所有核心模块用 Rust 重写
  ├── C++ 仅保留不常更新的 Plugin
  ├── WASM 编译支持
  └── 示例: 完整 Rust 仿真引擎 + WASM 前端
```

#### 15.2.2 C++ → Rust FFI 桥接

```rust
// afsim-ffi/src/lib.rs

use std::ffi::CStr;
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn afsim_engine_create() -> *mut afsim_engine::FrameStepEngine {
    let engine = Box::new(afsim_engine::FrameStepEngine::new());
    Box::into_raw(engine)
}

#[no_mangle]
pub extern "C" fn afsim_engine_advance_frame(
    engine: *mut afsim_engine::FrameStepEngine,
    frame_time: f64,
) {
    unsafe {
        (*engine).advance_frame(frame_time);
    }
}

#[no_mangle]
pub extern "C" fn afsim_engine_destroy(engine: *mut afsim_engine::FrameStepEngine) {
    unsafe {
        drop(Box::from_raw(engine));
    }
}

#[no_mangle]
pub extern "C" fn afsim_load_scenario(
    engine: *mut afsim_engine::FrameStepEngine,
    path: *const c_char,
) -> i32 {
    let path_str = unsafe { CStr::from_ptr(path) }.to_str().unwrap();
    match (*engine).load_scenario(path_str) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
```

### 15.3 性能验证方案

#### 15.3.1 基准测试框架

```rust
// benches/earth_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use afsim_earth::coordinates;
use afsim_core::types::{GeoCoord, EarthModel};

fn bench_geodetic_to_geocentric(c: &mut Criterion) {
    let coord = GeoCoord::new(39.9_f64.to_radians(), 116.4_f64.to_radians(), 100.0);
    let model = EarthModel::WGS84;

    c.bench_function("geodetic_to_geocentric_single", |b| {
        b.iter(|| coordinates::geodetic_to_geocentric(black_box(coord), black_box(&model)))
    });

    let coords: Vec<GeoCoord> = (0..1000).map(|i| {
        GeoCoord::new(
            (i as f64 * 0.01).to_radians(),
            (i as f64 * 0.02).to_radians(),
            100.0,
        )
    }).collect();

    c.bench_function("geodetic_to_geocentric_batch_1000", |b| {
        b.iter(|| coordinates::batch_geodetic_to_geocentric_simd(black_box(&coords), black_box(&model)))
    });
}

criterion_group!(benches, bench_geodetic_to_geocentric);
criterion_main!(benches);
```

#### 15.3.2 性能目标与验收标准

| 操作 | C++ 基线 | Rust 目标 | SIMD 目标 | 验收标准 |
|------|---------|----------|----------|---------|
| Vec3::Magnitude | 10 ns | 8 ns | 3 ns | < 5 ns |
| Vec3::Distance | 12 ns | 10 ns | 4 ns | < 6 ns |
| Geodetic→Geocentric | 50 ns | 40 ns | 15 ns | < 20 ns |
| Gravity Acceleration | 80 ns | 60 ns | 20 ns | < 30 ns |
| Ballistic Launch Compute | 500 μs | 200 μs | 80 μs | < 250 μs |
| Trajectory Propagate (100步) | 500 μs | 200 μs | 80 μs | < 250 μs |
| PDU Serialize | 3 μs | 1 μs | 400 ns | < 1.5 μs |
| 100 平台帧步进 | 8.5 ms | 5 ms | 2 ms (4核) | < 6 ms |
| 1000 平台帧步进 | 85 ms | 50 ms | 15 ms (8核) | < 60 ms |

### 15.4 风险与缓解

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| SIMD 性能不及预期 | 中 | 高 | 保留标量回退路径，criterion 基准验证 |
| WASM SIMD 浏览器兼容性 | 低 | 中 | 检测 `WebAssembly.simd()` 支持，不支持时回退 |
| C++ FFI 边界开销 | 中 | 中 | 批量 FFI 调用，减少跨边界次数 |
| 算法精度差异 | 低 | 高 | 逐位对比 C++/Rust 输出，容忍 < 1e-12 |
| 数据格式迁移 | 中 | 低 | 提供自动迁移工具，版本号校验 |
| Rayon 线程池竞争 | 低 | 中 | 使用 work-stealing 调度，避免全局锁 |

### 15.5 重构后的完整架构总览

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         AxAgent 分布式集群                                    │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │  Sim-Engine Node                                                     │ │
│  │                                                                       │ │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │ │
│  │  │  afsim-engine (Rust) ← 编排层，零计算                           │ │ │
│  │  │    FrameStepEngine::advance_frame()                              │ │ │
│  │  │    ├── rayon::par_iter_mut() → Platform::update()               │ │ │
│  │  │    ├── rayon::par_iter_mut() → Sensor::update()                │ │ │
│  │  │    └── event_scheduler.dispatch()                                │ │ │
│  │  └─────────────────────────────────────────────────────────────────┘ │ │
│  │         │ 调用算法                    │ 加载数据                       │ │
│  │         ▼                            ▼                               │ │
│  │  ┌──────────────────┐    ┌──────────────────────────────────────┐   │ │
│  │  │  算法层 (纯函数)   │    │  数据层 (纯数据)                      │   │ │
│  │  │  afsim-earth      │    │  afsim-data                          │   │ │
│  │  │  afsim-math+SIMD  │    │    WeaponDatabase (JSON)             │   │ │
│  │  │  afsim-weapon     │    │    SensorDatabase (JSON)             │   │ │
│  │  │  afsim-dis        │    │    EGM96Coefficients (embedded)      │   │ │
│  │  │  afsim-orbital    │    │    StandardAtmosphere (embedded)     │   │ │
│  │  │  afsim-units      │    │    IGRFCoefficients (embedded)       │   │ │
│  │  └──────────────────┘    │    GeoTileCache (LRU)                 │   │ │
│  │         │                └──────────────────────────────────────┘   │ │
│  │         │ 依赖类型                                                   │ │
│  │         ▼                                                           │ │
│  │  ┌──────────────────┐                                               │ │
│  │  │  类型层 (纯类型)   │ ← afsim-core: Vec3, Mat3, DCM, Quat,       │ │
│  │  │                    │   GeoCoord, EarthModel, Side, PlatformId    │ │
│  │  └──────────────────┘                                               │ │
│  │                                                                      │ │
│  │  ┌──────────────────┐    ┌──────────────────┐                       │ │
│  │  │  WASM 导出         │    │  C FFI 导出        │                       │ │
│  │  │  afsim-wasm       │    │  afsim-ffi        │                       │ │
│  │  │  → Skill Sandbox  │    │  → 遗留 C++ 互操作 │                       │ │
│  │  └──────────────────┘    └──────────────────┘                       │ │
│  └───────────────────────────────────────────────────────────────────────┘ │
│                                                                             │
│  ┌────────────────────┐  ┌────────────────────┐  ┌────────────────────┐  │
│  │  DIS-Gateway Node  │  │  Mil-Domain Node   │  │  Data-Repo Node    │  │
│  │  dis_driver (Rust) │  │  mil_domain (Rust) │  │  geodata (Rust)    │  │
│  │  dis_gateway       │  │  weapon_db         │  │  result_db         │  │
│  └────────────────────┘  └────────────────────┘  └────────────────────┘  │
│                                                                             │
│  通信: MessageGateway (WS/HTTP/IPC) + DistributedToolRegistry              │
│  状态: CrdtEngine (HLC) + DistributedBridge                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

*文档版本: 4.0*
*最后更新: 2026-05-09*
*扩展: Rust 高性能计算重构 + 模型-引擎解耦 + 算法-数据分离*
