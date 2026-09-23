// SPDX-License-Identifier: AGPL-3.0-only

// i18n-exempt: Mock data for browser preview mode. Not user-facing UI.
import i18n from "@/i18n";
/**
 * Browser-mode mock backend using localStorage.
 * Activated when the app runs outside Tauri (e.g. `pnpm dev` in browser).
 * Provides CRUD operations for providers, conversations, apps, settings, and gateway.
 */

import { CAPABILITY_DOMAIN_META, CAPABILITY_DOMAIN_PROTOCOL_ORDER, domainLabelKey } from "@/lib/domainMeta";
import type { CreateNarrativeRequest, NarrativeStructureRecord } from "@/lib/narrativeStructure";
import type { CapabilityDomain } from "@/types";
import type {
  Conversation,
  ConversationBranch,
  ConversationCategory,
  CreateSearchProviderInput,
  DeliveryInvoice,
  DemandLead,
  DemandSubscription,
  GatewayKey,
  KnowledgeBase,
  KnowledgeDocument,
  MemoryItem,
  MemoryNamespace,
  Message,
  Note,
  NoteSearchResult,
  PlatformConfig,
  PlatformSession,
  ProgramPolicy,
  SaveProgramPolicyInput,
  SearchProvider,
  Wiki,
  WikiTemplate,
} from "@/types";
import type { Artifact } from "@/types";
import type { BackupManifest } from "@/types";
import type { CreateKnowledgeBaseInput } from "@/types";
import type { CreateMemoryItemInput, CreateMemoryNamespaceInput } from "@/types";
import type {
  CreateDynamicUISchemaParams,
  DynamicUIFormDataRecord,
  DynamicUIPinRecord,
  DynamicUISchemaRecord,
  DynamicUISchemaVersion,
  ListVersionsResponse,
  SaveDynamicUIFormDataParams,
  UpdateDynamicUISchemaParams,
} from "@/types";
import type {
  CapabilityDiscoveryResult,
  CapabilityIndexStats,
  CapabilityPassportDto,
  CapabilityStats,
  IndexResult,
  RankedCapability,
} from "@/types";
import { emitBrowserEvent } from "./browserEvents";

interface Fleet {
  id: string;
  name: string;
  sceneTemplateSlug?: string;
  status: string;
  createdAt: number;
  updatedAt: number;
  metadata: Record<string, unknown>;
}

interface FleetMember {
  id: string;
  fleetId: string;
  agentId: string;
  agentSlug: string;
  displayName: string;
  role: string;
  agentProfileId?: string;
  roomId: string;
  status: string;
  joinedAt: number;
  todayTokens: number;
  totalTokens: number;
}

/**
 * 浏览器模式持久化的舰队消息（对齐 `@/types` 的 `FleetMessage` 与后端
 * `fleet_messages` 表）。
 *
 * ⚠ `conversationId` 是**会话作用域**（`"group"` / `"dm:<slug>"`），
 * 不是成员站位的物理房间（`FleetMember.roomId`）。二者曾同名，导致
 * DM 与群聊混进同一条时间线。
 */
interface FleetMessage {
  id: string;
  fleetId: string;
  conversationId: string;
  seq: number;
  authorKind: "human" | "agent";
  authorId: string;
  authorSlug?: string;
  authorDisplayName?: string;
  content: string;
  createdAt: number;
}

/** 浏览器模式持久化的 AgentSession（与后端 agent_sessions 表 upsert 语义对齐） */
interface MockAgentSession {
  conversationId: string;
  name: string | null;
  metadata: Record<string, unknown> | null;
  cwd: string | null;
  permissionMode: string;
  createdAt: number;
  updatedAt: number;
}

/** 浏览器模式 Channel 形状（与 Tauri v2 Channel 的 onmessage 对齐） */
interface MockChannel {
  onmessage?: (evt: unknown) => void;
}

interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  icon: string;
  tags: string[];
  version: number;
  isPreset: boolean;
  isEditable: boolean;
  isPublic: boolean;
  /** 是否为系统模板（认知编排器等），include_system=true 时才能读到 */
  isSystem?: boolean;
  triggerConfig: Record<string, unknown>;
  nodes: unknown[];
  edges: unknown[];
  createdAt: number;
  updatedAt: number;
}

interface CreateWorkflowTemplateInput {
  name?: string;
  description?: string;
  tags?: string[];
  nodes?: unknown[];
  edges?: unknown[];
}

interface UpdateWorkflowTemplateInput {
  name?: string;
  description?: string;
  tags?: string[];
  nodes?: unknown[];
  edges?: unknown[];
}

interface ProviderKey {
  id: string;
  provider_id: string;
  key_encrypted: string;
  key_prefix: string;
  enabled: boolean;
  last_validated_at: number | null;
  last_error: string | null;
  rotation_index: number;
  created_at: number;
}

interface Provider {
  id: string;
  name: string;
  provider_type: string;
  api_host: string;
  api_path?: string;
  sort_order?: number;
  enabled: boolean;
  models: Array<{
    model_id: string;
    name: string;
    mode?: string;
    enabled?: boolean;
  }>;
  keys: ProviderKey[];
  proxy_config: unknown;
  created_at: number;
  updated_at: number;
}

interface Settings {
  [key: string]: unknown;
}

// ── OPC 需求发现预置平台（浏览器模式 Mock 数据） ────────────────────
// 与后端 DemandPlatform DTO（serde camelCase）对齐；id 与内置扫描器
// platform() 返回值一致（下划线风格），保证 builtin_scanner_for 可路由。
interface MarketPlatform {
  id: string;
  name: string;
  platformType: string;
  enabled: boolean;
  baseUrl: string | null;
  config: Record<string, unknown> | null;
  lastSyncAt: number | null;
  status: string;
  lastError: string | null;
  createdAt: number;
  updatedAt: number;
}

const MOCK_PLATFORM_DEFS: Array<[string, string, string | null]> = [
  ["reddit", "Reddit", "https://www.reddit.com"],
  ["hackernews", "HackerNews", "https://news.ycombinator.com"],
  ["github_issue", "GitHub Issues", "https://github.com"],
  ["github_discussion", "GitHub Discussions", "https://github.com"],
  ["stackoverflow", "StackOverflow", "https://stackoverflow.com"],
  ["producthunt", "Product Hunt", "https://www.producthunt.com"],
  ["huggingface", "HuggingFace", "https://huggingface.co"],
  ["package_ecosystem", "Package Ecosystem", null],
  ["arxiv", "arXiv", "https://arxiv.org"],
  ["twitter", "Twitter/X", "https://twitter.com"],
  ["zhubajie", "猪八戒", "https://www.zbj.com"],
  ["xianyu", "闲鱼", "https://www.goofish.com"],
  ["linkedin", "LinkedIn", "https://www.linkedin.com"],
  ["zhihu", "知乎", "https://www.zhihu.com"],
  ["csdn", "CSDN", "https://www.csdn.net"],
  ["juejin", "掘金", "https://juejin.cn"],
  ["dribbble", "Dribbble", "https://dribbble.com"],
  ["upwork", "Upwork", "https://www.upwork.com"],
];

function buildMockPlatform(id: string, name: string, baseUrl: string | null): MarketPlatform {
  const now = 1700000000;
  return {
    id,
    name,
    platformType: "scanner",
    enabled: true,
    baseUrl,
    config: { description: `${name} 扫描器`, auto_sync: true },
    lastSyncAt: null,
    status: "idle",
    lastError: null,
    createdAt: now,
    updatedAt: now,
  };
}

let PRESET_MOCK_PLATFORMS: MarketPlatform[] = MOCK_PLATFORM_DEFS.map(([id, name, baseUrl]) =>
  buildMockPlatform(id, name, baseUrl)
);

// ── OPC 需求线索（浏览器模式 Mock 数据） ────────────────────────────
let MOCK_DEMAND_LEADS: DemandLead[] = [
  {
    id: "lead-hn-001",
    platform: "hackernews",
    title: "Show HN: 我需要一个能自动汇总 arXiv 论文的周报工具",
    description: "每周要读 50+ 篇论文，人工筛选太慢。希望有工具按我的兴趣关键词自动聚类并生成摘要周报，愿意付费。",
    budgetMin: 500,
    budgetMax: 2000,
    budgetCurrency: "USD",
    contactName: null,
    contactEmail: null,
    contactPhone: null,
    sourceUrl: "https://news.ycombinator.com/item?id=41000001",
    status: "new",
    confidence: 0.82,
    painScore: 88,
    marketGapScore: 64,
    commercialValueScore: 76,
    demandType: "content_creation",
    linkedWorkflowId: null,
    implementedAt: null,
    createdAt: 1700000000,
    updatedAt: 1700000000,
  },
  {
    id: "lead-zbj-002",
    platform: "zhubajie",
    title: "求开发：跨境电商多店铺库存同步系统",
    description: "同时经营 6 个平台店铺，库存经常超卖。需要一套能对接各平台 API、实时同步库存并预警的后台系统。",
    budgetMin: 30000,
    budgetMax: 80000,
    budgetCurrency: "CNY",
    contactName: "王先生",
    contactEmail: null,
    contactPhone: null,
    sourceUrl: "https://www.zbj.com/requirement/123456",
    status: "new",
    confidence: 0.9,
    painScore: 92,
    marketGapScore: 55,
    commercialValueScore: 81,
    demandType: "development",
    linkedWorkflowId: null,
    implementedAt: null,
    createdAt: 1700000100,
    updatedAt: 1700000100,
  },
  {
    id: "lead-reddit-003",
    platform: "reddit",
    title: "Anyone know a self-hosted alternative to Notion AI for team wiki?",
    description:
      "We can't send data to third-party SaaS. Looking for a self-hosted wiki with semantic search over our internal docs.",
    budgetMin: null,
    budgetMax: null,
    budgetCurrency: "USD",
    contactName: null,
    contactEmail: null,
    contactPhone: null,
    sourceUrl: "https://www.reddit.com/r/selfhosted/comments/abc123",
    status: "new",
    confidence: 0.55,
    painScore: 70,
    marketGapScore: 72,
    commercialValueScore: 61,
    demandType: "development",
    linkedWorkflowId: null,
    implementedAt: null,
    createdAt: 1700000200,
    updatedAt: 1700000200,
  },
];

/** 模拟需求订阅词表（浏览器模式，v133） */
let MOCK_DEMAND_SUBSCRIPTIONS: DemandSubscription[] = [
  {
    id: "sub-selfhosted",
    keyword: "self-hosted wiki",
    enabled: true,
    intervalHours: 6,
    minScore: 60,
    platforms: [],
    lastScannedAt: null,
    lastHitCount: 0,
    createdAt: 1700000300,
    updatedAt: 1700000300,
  },
  {
    id: "sub-invoice",
    keyword: "invoice automation",
    enabled: true,
    intervalHours: 12,
    minScore: 70,
    platforms: ["upwork"],
    lastScannedAt: 1699996400,
    lastHitCount: 2,
    createdAt: 1700000400,
    updatedAt: 1700000400,
  },
];

/** 交付发票账本 mock（P4）：预置一张已开出未回款的发票 */
let MOCK_INVOICES: DeliveryInvoice[] = [
  {
    id: "inv-zbj-002",
    leadId: "lead-zbj-002",
    linkedWorkflowId: null,
    title: "跨境电商多店铺库存同步系统",
    amount: 80000,
    currency: "CNY",
    status: "sent",
    issuedAt: 1700000500,
    paidAt: null,
    notes: null,
    createdAt: 1700000450,
    updatedAt: 1700000500,
  },
];

// ── 能力域覆盖层（P2，PLAN-domain-single-source.md §9.3） ────────────────
//
// ⚠ 行集合**派生**自 `domainMeta.ts` 的协议顺序（+ `system`），**不在这里手抄
//   一遍 9 个 id**：手抄的失败方式是静默的 —— 后端新增一个域后浏览器模式少一行，
//   而门禁 `check-domain-single-source.mjs` 会把「又一份 id 副本」记进报告。
//   `nav_path` / `nav_order` 同样取自 `CAPABILITY_DOMAIN_META`（前端**唯一**持有
//   域 id 集合的地方），故本文件不新增任何域表副本。
//
// ⚠ `builtin_aliases` 刻意留空：27 条内置别名是**后端声明**（`DOMAIN_NODES`），
//   前端没有它、也不该为了 mock 造一份 —— 造出来的那份必然与后端各自腐烂。
//   （真实别名在桌面端由后端给出；浏览器模式只用于看渲染与交互。）
//
// 覆盖层语义照搬后端：**没有覆盖行 = 用内置默认**。故只预置 1 条演示覆盖，
// 其余保持「未改过」（`has_override: false`、全部启用）。

/** 演示用覆盖层状态：域 id → 该域的覆盖值 */
const MOCK_DOMAIN_OVERRIDES: Record<string, { enabled: boolean; extra_aliases: string[] }> = {
  finance: { enabled: true, extra_aliases: ["投资", "证券"] },
};

/** 与后端 `CapabilityDomainEntryDto` 的 snake_case 源同形（命令出口会转 camelCase） */
interface MockCapabilityDomainRow {
  id: string;
  label_key: string;
  nav_path: string | null;
  nav_order: number | null;
  is_system: boolean;
  toggleable: boolean;
  toggle_block_reason: string | null;
  enabled: boolean;
  has_override: boolean;
  builtin_aliases: string[];
  extra_aliases: string[];
  effective_aliases: string[];
}

/** 由「内置声明（前端侧派生）+ 覆盖层」拼出合并视图，与后端 `build_domain_entry` 同义。 */
function mockCapabilityDomainRegistry(): MockCapabilityDomainRow[] {
  const ids: CapabilityDomain[] = [...CAPABILITY_DOMAIN_PROTOCOL_ORDER, "system"];
  return ids.map((id) => {
    const isSystem = id === "system";
    const override = MOCK_DOMAIN_OVERRIDES[id];
    const extraAliases = override?.extra_aliases ?? [];
    const meta = CAPABILITY_DOMAIN_META.find((m) => m.id === id);
    // 例外域与后端 `is_toggleable` 同判据：General 是唯一兜底域、System 是内部域
    const toggleable = id !== "general" && !isSystem;
    return {
      id,
      label_key: domainLabelKey(id),
      nav_path: meta?.path ?? null,
      nav_order: meta ? CAPABILITY_DOMAIN_META.indexOf(meta) : null,
      is_system: isSystem,
      toggleable,
      toggle_block_reason: toggleable
        ? null
        : (isSystem ? "system_is_internal" : "general_is_fallback"),
      enabled: override?.enabled ?? true,
      has_override: override !== undefined,
      builtin_aliases: [],
      extra_aliases: extraAliases,
      effective_aliases: extraAliases,
    };
  });
}

/**
 * 模拟认知编排器路由匹配（L1/L2/L3）
 * 根据用户输入推断业务域、能力簇和工作流
 */
function mockCognitiveRoute(input: string): {
  domain: string;
  cluster: string;
  capabilityId: string;
  routePath: string;
  executionMode: string;
  candidates: Array<{ id: string; name: string; description: string; score: number }>;
} {
  const text = input.toLowerCase();

  // L1 域路由规则
  const domainRules: Array<{ keywords: string[]; domain: string; cluster: string }> = [
    {
      keywords: ["小说", "诗歌", "散文", "文学", "写作", "创作", "novel", "poetry", "prose", "literary", "writing"],
      domain: "content_creation",
      cluster: "literary",
    },
    {
      keywords: ["股票", "基金", "投资", "行情", "交易", "stock", "trading", "finance", "investment"],
      domain: "finance",
      cluster: "stock_analysis",
    },
    {
      keywords: ["订单", "退款", "发货", "物流", "order", "refund", "shipping"],
      domain: "automation",
      cluster: "order_management",
    },
    {
      keywords: ["部署", "监控", "ci/cd", "docker", "devops", "deployment", "monitoring"],
      domain: "devops",
      cluster: "deployment",
    },
    {
      keywords: ["数据", "分析", "报表", "data", "analysis", "sql"],
      domain: "data_analysis",
      cluster: "analysis",
    },
    {
      keywords: ["邮件", "通知", "email", "notification", "message"],
      domain: "communication",
      cluster: "messaging",
    },
  ];

  // 查找匹配的域
  for (const rule of domainRules) {
    if (rule.keywords.some((kw) => text.includes(kw.toLowerCase()))) {
      // L3 能力路由：根据域返回对应的工作流
      const workflows: Record<string, Array<{ id: string; name: string; description: string }>> = {
        content_creation: [
          { id: "workflow-cm-literary-creation", name: "文字创作", description: "小说/诗歌/散文创作工作流" },
          { id: "workflow-cm-viral-content", name: "爆款内容生成", description: "选题策划 → 内容创作 → 优化打磨" },
          { id: "workflow-cm-multi-platform", name: "多平台适配", description: "内容创作 → 平台适配 → 分发策略" },
        ],
        finance: [
          { id: "stock-analysis", name: "股票分析", description: "技术面/基本面/新闻分析" },
          { id: "stock-trading", name: "股票交易", description: "交易执行与风控" },
        ],
        automation: [
          { id: "order-fulfillment", name: "订单履约", description: "订单处理与物流" },
        ],
        devops: [
          { id: "ci-cd-pipeline", name: "CI/CD 流水线", description: "持续集成与部署" },
          { id: "monitoring", name: "监控告警", description: "系统监控与告警" },
        ],
        data_analysis: [
          { id: "data-analysis", name: "数据分析", description: "数据查询与可视化" },
        ],
        communication: [
          { id: "notification", name: "通知推送", description: "消息通知服务" },
        ],
      };

      const candidates = workflows[rule.domain] || [];
      const primaryCandidate = candidates[0];

      return {
        domain: rule.domain,
        cluster: rule.cluster,
        capabilityId: primaryCandidate?.id ?? "",
        routePath: `${rule.domain}/${rule.cluster}`,
        executionMode: primaryCandidate ? "workflow" : "ask",
        candidates: candidates.map((c, i) => ({
          id: c.id,
          name: c.name,
          description: c.description,
          score: 0.95 - i * 0.1,
        })),
      };
    }
  }

  // 默认：通用域
  return {
    domain: "general",
    cluster: "chat",
    capabilityId: "",
    routePath: "general/chat",
    executionMode: "ask",
    candidates: [],
  };
}

function genId(): string {
  return crypto.randomUUID();
}

function nowTs(): number {
  return Date.now();
}

function getStore<T>(key: string, defaultValue: T): T {
  try {
    const data = localStorage.getItem(`axagent_${key}`);
    return data ? JSON.parse(data) : defaultValue;
  } catch {
    return defaultValue;
  }
}

function setStore<T>(key: string, value: T): void {
  try {
    localStorage.setItem(`axagent_${key}`, JSON.stringify(value));
  } catch (e) {
    console.warn(`Failed to write localStorage key: axagent_${key}`, e);
  }
}

// ── Wiki 笔记 mock（浏览器模式最小数据集，localStorage 持久化）──────

function getWikiNotes(): Note[] {
  return getStore<Note[]>("mock.wikiNotes", []);
}

function setWikiNotes(notes: Note[]): void {
  setStore("mock.wikiNotes", notes);
}

// ── Wiki 图谱 fixture（**可选**，浏览器模式下渲染真实规模图谱）──────────
//
// 背景：浏览器模式的 wiki 图谱此前完全没有数据通路 —— `llm_wiki_list` 恒返回 `[]`
// ⇒ 图页面判定「无可用 wiki」后连图数据都不请求。想在图谱页做任何渲染层调试
// （性能、分组、边可见性）都无从下手，只能靠 Tauri 真机。
//
// 本 fixture 是**纯 opt-in** 的：URL 取 `window.__WIKI_GRAPH_FIXTURE_URL__`，
// 未设置时退回 `/__mock-wiki-graph.json`；**拉不到就返回 null，三个 case 全部退回
// 改动前的行为**（e2e 不依赖 fixture，因此不受影响）。
//
// 文件形态与两个后端命令的返回**逐字段对齐**（由 output/ 下的导出脚本从 PG 生成）：
//   { wiki, graph, communitiesResult }  ⇒  llm_wiki_list / get_wiki_graph_cached
//                                          / wiki_graph_communities_cached
type WikiGraphFixture = {
  wiki: Wiki;
  graph: { nodes: unknown[]; edges: unknown[] };
  communitiesResult: {
    communities: Record<string, number>;
    /**
     * 实体侧社区（`entity:<id>` → cid），**可选** —— 生成 fixture 的导出脚本会带上它；
     * 老 fixture 没有该字段时前端退回「锚点继承同名笔记的桶」。
     * 有了它，浏览器模式才能复现「实体侧单独算社区」这条路径，
     * 否则图页面在浏览器里永远只跑兜底分支（判据与真机不同源）。
     */
    entityCommunities?: Record<string, number>;
  };
};

let wikiGraphFixturePromise: Promise<WikiGraphFixture | null> | undefined;

function loadWikiGraphFixture(): Promise<WikiGraphFixture | null> {
  if (!wikiGraphFixturePromise) {
    wikiGraphFixturePromise = (async () => {
      const override = (globalThis as { __WIKI_GRAPH_FIXTURE_URL__?: string })
        .__WIKI_GRAPH_FIXTURE_URL__;
      const url = override ?? "/__mock-wiki-graph.json";
      try {
        const res = await fetch(url);
        if (!res.ok) { return null; }
        // ⚠ 必须校验 content-type：Vite dev server 对未知路径会走 SPA 兜底，
        // 返回 `index.html` 且状态码是 **200**（不是 404）。只判 `res.ok`
        // 会拿到一段 HTML 再去 JSON.parse，然后被 catch 吞掉 ——
        // 结果是「fixture 配错了」和「fixture 不存在」表现完全一样，
        // 排查时会被误导。
        const ctype = res.headers.get("content-type") ?? "";
        if (!ctype.includes("json")) { return null; }
        return (await res.json()) as WikiGraphFixture;
      } catch {
        // 文件不存在 / 不是合法 JSON 都属正常情形（默认不启用 fixture），
        // 此处**刻意不打印 warning**，否则每次浏览器模式启动都会刷一条噪音。
        return null;
      }
    })();
  }
  return wikiGraphFixturePromise;
}

function mockWikiNote(vaultId: string, title: string, content: string, tags: string[]): Note {
  const ts = nowTs();
  return {
    id: genId(),
    vaultId,
    title,
    filePath: `${title}.md`,
    content,
    contentHash: "",
    author: "mock",
    tags,
    userEdited: false,
    createdAt: ts,
    updatedAt: ts,
    isDeleted: false,
  };
}

// 按 vault 播种演示笔记（仅首次），返回该 vault 的笔记列表。
function seedWikiNotes(vaultId: string): Note[] {
  const existing = getWikiNotes().filter((n) => n.vaultId === vaultId);
  if (existing.length > 0) { return existing; }
  const seeded = [
    mockWikiNote(vaultId, "概览", "# 概览\n\n这是一个浏览器模式演示 Wiki。\n\n数据保存在 localStorage。", [
      "overview",
    ]),
    mockWikiNote(vaultId, "快速上手", "# 快速上手\n\n从左侧面板创建笔记开始，编辑内容会自动保存到浏览器本地。", [
      "guide",
    ]),
    mockWikiNote(
      vaultId,
      "常见问题",
      "# 常见问题\n\nQ: 浏览器模式下数据可靠吗？\nA: 数据仅存于当前浏览器的 localStorage，清除站点数据会丢失。",
      [
        "faq",
      ],
    ),
  ];
  setWikiNotes([...getWikiNotes(), ...seeded]);
  return seeded;
}

// ── Capability System (能力发现系统) ──────────────────────────────

const CAPABILITY_STORAGE_KEY = "mock.capabilities";

function capabilityStats(): CapabilityStats {
  return {
    totalCalls: 0,
    successCount: 0,
    avgDurationSeconds: 0,
    recentSuccessRate: 0,
    circuitState: "closed",
  };
}

function mockPassport(
  capabilityId: string,
  name: string,
  kind: CapabilityPassportDto["kind"],
  domain: CapabilityPassportDto["domain"],
  description: string,
  tags: string[],
  subCategory?: string,
  source: CapabilityPassportDto["source"] = "builtin",
  evolvable: CapabilityPassportDto["evolvable"] = "local",
): CapabilityPassportDto {
  return {
    capabilityId: capabilityId,
    name,
    description,
    version: null,
    owner: null,
    createdAt: null,
    updatedAt: null,
    kind,
    domain,
    subCategory: subCategory,
    inputSchema: null,
    outputSchema: null,
    implementation: null,
    tags,
    negativeScenarios: [],
    securityLevel: "public",
    modalitySupport: {
      supportsText: true,
      supportsImage: false,
      supportsAudio: false,
      supportsVideo: false,
      supportsFile: false,
    },
    outputCapabilities: {
      supportsText: true,
      supportsTable: false,
      supportsChart: false,
      supportsImage: false,
      supportsInteractive: false,
    },
    estimatedCostUsd: 0,
    avgDurationSeconds: 0,
    executionMode: "sync",
    timeoutMs: null,
    planningComplexity: "simple",
    modelIqRequirement: 60,
    experimentGroup: null,
    stats: capabilityStats(),
    level: "l3",
    enabled: true,
    source,
    evolvable,
    exposure: "auto",
    toolRef: null,
    aliases: [],
    steps: [],
    skillSteps: [],
    placeholders: [],
    templateBody: null,
    instantiatesTo: null,
    exampleInstance: null,
    upstream: [],
    downstream: [],
    preconditions: [],
    attachedSnippets: [],
  };
}

function defaultMockPassports(): CapabilityPassportDto[] {
  return [
    mockPassport(
      "cap.workflow.stock_analysis",
      "股票走势分析工作流",
      "workflow",
      "finance",
      "综合分析股票走势、均线与成交量，输出趋势判断。",
      ["股票", "走势", "K线", "分析"],
    ),
    mockPassport(
      "cap.workflow.image_generation",
      "图像生成工作流",
      "workflow",
      "ai_media",
      "根据文字描述生成图像。",
      ["图像", "生成", "AI绘画"],
    ),
    mockPassport(
      "cap.tool.web_search",
      "网络搜索工具",
      "tool",
      "general",
      "执行网络搜索并返回结果摘要。",
      ["搜索", "网络", "查询"],
    ),
    mockPassport(
      "cap.tool.code_execution",
      "代码执行工具",
      "tool",
      "devops",
      "在沙箱中执行代码并返回输出。",
      ["代码", "执行", "Python", "JS"],
    ),
    mockPassport(
      "cap.agent.data_analyst",
      "数据分析智能体",
      "agent",
      "data_analysis",
      "执行数据分析任务并生成报告。",
      ["数据分析", "统计", "报告"],
      "agent_profile",
    ),
    mockPassport(
      "cap.agent_role.analyst",
      "分析师协作角色",
      "agent",
      "data_analysis",
      "分析师协作角色，负责数据洞察与决策建议。",
      ["分析师", "协作", "决策"],
      "agent_role",
    ),
    mockPassport(
      "cap.kb.product_docs",
      "产品文档知识库",
      "knowledge_base",
      "general",
      "检索产品使用文档。",
      ["文档", "知识库", "产品"],
    ),
    mockPassport(
      "cap.skill.web_automation",
      "网页自动化技能",
      "skill",
      "automation",
      "由浏览器插件提供的网页自动化能力，可执行点击、填表与截图。",
      ["网页", "自动化", "插件"],
      undefined,
      "plugin",
      "derived",
    ),
  ];
}

function readCapabilityPassports(): CapabilityPassportDto[] {
  return getStore<CapabilityPassportDto[]>(CAPABILITY_STORAGE_KEY, []).length
    ? getStore<CapabilityPassportDto[]>(CAPABILITY_STORAGE_KEY, [])
    : defaultMockPassports();
}

function writeCapabilityPassports(passports: CapabilityPassportDto[]): void {
  setStore(CAPABILITY_STORAGE_KEY, passports);
}

function capabilityStatsFrom(
  passports: CapabilityPassportDto[],
): CapabilityIndexStats {
  const totalVectors = passports.reduce(
    (sum, p) => sum + p.tags.length + 2,
    0,
  );
  return {
    totalCapabilities: passports.length,
    totalVectors: totalVectors,
    positiveVectors: passports.length * 2,
    negativeVectors: totalVectors - passports.length * 2,
    lastIndexedAt: nowTs(),
  };
}

function indexResultFor(
  passport: CapabilityPassportDto,
  success: boolean,
  error?: string | null,
): IndexResult {
  return {
    capabilityId: passport.capabilityId,
    success,
    vectorDimensions: 768,
    indexedAtMs: nowTs(),
    error: error ?? null,
  };
}

function rankCapabilityFor(
  passport: CapabilityPassportDto,
  userInput: string,
  baseScore: number,
): RankedCapability {
  const input = userInput.toLowerCase();
  const tagHit = passport.tags.some((tag) => input.includes(tag.toLowerCase()));
  const score = Math.min(0.99, baseScore + (tagHit ? 0.15 : 0));
  return {
    passport,
    semanticScore: baseScore,
    historyScore: 0.5,
    speedScore: 0.8,
    costScore: 0.8,
    personalizationBoost: 0,
    explorationBoost: 0,
    finalScore: score,
    reasons: tagHit ? ["关键词命中"] : ["语义相似"],
  };
}

function mockDiscover(userInput: string): CapabilityDiscoveryResult {
  const passports = readCapabilityPassports();
  const candidates: RankedCapability[] = passports
    .map((p, idx) => rankCapabilityFor(p, userInput, 0.7 - idx * 0.08))
    .sort((a, b) => b.finalScore - a.finalScore);

  const primary = candidates[0] ?? null;
  const next = candidates[1] ?? null;
  const ambiguous = !!primary && !!next && next.finalScore >= 0.85;

  return {
    primaryMatch: primary,
    alternatives: candidates.slice(1, 3),
    ambiguous,
    clarificationPrompt: ambiguous
      ? i18n.t("browserMock.capabilityAmbiguous")
      : null,
    suggestions: [],
    circuitInfo: null,
    totalElapsedMs: 12,
    phaseTimings: [
      { phase: "retrieval", elapsedMs: 5 },
      { phase: "filter", elapsedMs: 3 },
      { phase: "rank", elapsedMs: 4 },
    ],
    extractedEntities: [],
  };
}

function generateBrowserResponse(userContent: string): string {
  const greeting = /^(hi|hello|hey)/i.test(userContent.trim());
  if (greeting) {
    return i18n.t("browserMock.greeting");
  }
  const truncatedContent = userContent.length > 50 ? userContent.slice(0, 50) + "..." : userContent;
  return i18n.t("browserMock.receivedMessage", { userContent: truncatedContent });
}

// ── 数据格式转换工具 ────────────────────────────────────────────────────

/**
 * 将 snake_case 字符串转换为 camelCase
 * 例如：provider_type → providerType, model_id → modelId
 */
function snakeToCamel(str: string): string {
  return str.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
}

/**
 * 递归地将对象或数组中的所有 snake_case 键转换为 camelCase
 * 用于将后端 mock 数据转换为前端期望的格式
 */
function convertToCamelCase<T>(obj: T): T {
  if (obj === null || obj === undefined) {
    return obj;
  }
  if (Array.isArray(obj)) {
    return obj.map((item) => convertToCamelCase(item)) as T;
  }
  if (typeof obj === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      const camelKey = snakeToCamel(key);
      result[camelKey] = convertToCamelCase(value);
    }
    return result as T;
  }
  return obj;
}

/**
 * 将 camelCase 字符串转换为 snake_case
 * 例如：providerType → provider_type, modelId → model_id
 */
function camelToSnake(str: string): string {
  return str.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

/**
 * 递归地将对象或数组中的所有 camelCase 键转换为 snake_case
 * 用于将前端参数转换为后端期望的格式
 */
function convertToSnakeCase<T>(obj: T): T {
  if (obj === null || obj === undefined) {
    return obj;
  }
  if (Array.isArray(obj)) {
    return obj.map((item) => convertToSnakeCase(item)) as T;
  }
  if (typeof obj === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      const snakeKey = camelToSnake(key);
      result[snakeKey] = convertToSnakeCase(value);
    }
    return result as T;
  }
  return obj;
}

// ── Built-in Providers ──────────────────────────────────────────────────

const BUILT_IN_PROVIDERS = [
  {
    id: "builtin-openai",
    name: "OpenAI",
    provider_type: "openai",
    api_host: "https://api.openai.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-openai",
        model_id: "gpt-5.5",
        name: "gpt-5.5",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai",
        model_id: "gpt-5.4",
        name: "gpt-5.4",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai",
        model_id: "gpt-5.4-mini",
        name: "gpt-5.4-mini",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai",
        model_id: "o4-mini",
        name: "o4-mini",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 200000,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 0,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-openai-responses",
    name: "OpenAI Responses",
    provider_type: "openai_responses",
    api_host: "https://api.openai.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-openai-responses",
        model_id: "gpt-5.5",
        name: "gpt-5.5",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai-responses",
        model_id: "gpt-5.4",
        name: "gpt-5.4",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai-responses",
        model_id: "gpt-5.4-mini",
        name: "gpt-5.4-mini",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-openai-responses",
        model_id: "o4-mini",
        name: "o4-mini",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 200000,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 1,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-gemini",
    name: "Gemini",
    provider_type: "gemini",
    api_host: "https://generativelanguage.googleapis.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-gemini",
        model_id: "gemini-3.5-flash",
        name: "gemini-3.5-flash",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-gemini",
        model_id: "gemini-2.5-flash",
        name: "gemini-2.5-flash",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-gemini",
        model_id: "gemini-2.5-pro",
        name: "gemini-2.5-pro",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 2,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-anthropic",
    name: "Claude",
    provider_type: "anthropic",
    api_host: "https://api.anthropic.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-anthropic",
        model_id: "claude-sonnet-4-6",
        name: "claude-sonnet-4-6",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 200000,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-anthropic",
        model_id: "claude-haiku-4-5",
        name: "claude-haiku-4-5",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 200000,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-anthropic",
        model_id: "claude-opus-4-8",
        name: "claude-opus-4-8",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 200000,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 3,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-deepseek",
    name: "DeepSeek",
    provider_type: "openai",
    api_host: "https://api.deepseek.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-deepseek",
        model_id: "deepseek-v4-flash",
        name: "deepseek-v4-flash",
        capabilities: ["TextGeneration", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-deepseek",
        model_id: "deepseek-v4-pro",
        name: "deepseek-v4-pro",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 4,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-qwen",
    name: i18n.t("browserMock.tongyi"),
    provider_type: "openai",
    api_host: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-qwen",
        model_id: "qwen3.7-max",
        name: "qwen3.7-max",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-qwen",
        model_id: "qwen3.6-plus",
        name: "qwen3.6-plus",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-qwen",
        model_id: "qwen3.6-flash",
        name: "qwen3.6-flash",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 1048576,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 5,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-kimi",
    name: "Kimi",
    provider_type: "openai",
    api_host: "https://api.moonshot.cn/v1",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-kimi",
        model_id: "kimi-k2.6",
        name: "kimi-k2.6",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 262144,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-kimi",
        model_id: "kimi-k2.5",
        name: "kimi-k2.5",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling", "Reasoning"],
        max_tokens: 262144,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 6,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-doubao",
    name: i18n.t("browserMock.doubao"),
    provider_type: "openai",
    api_host: "https://ark.cn-beijing.volces.com/api/v3",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-doubao",
        model_id: "doubao-1.5-pro-256k",
        name: "doubao-1.5-pro-256k",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 262144,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-doubao",
        model_id: "doubao-1.5-lite-32k",
        name: "doubao-1.5-lite-32k",
        capabilities: ["TextGeneration", "FunctionCalling"],
        max_tokens: 32768,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 7,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-siliconflow",
    name: i18n.t("browserMock.siliconFlow"),
    provider_type: "openai",
    api_host: "https://api.siliconflow.cn/v1",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-siliconflow",
        model_id: "Pro/deepseek-ai/DeepSeek-R1",
        name: "Pro/deepseek-ai/DeepSeek-R1",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 65536,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-siliconflow",
        model_id: "Pro/deepseek-ai/DeepSeek-V3",
        name: "Pro/deepseek-ai/DeepSeek-V3",
        capabilities: ["TextGeneration", "FunctionCalling"],
        max_tokens: 65536,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-siliconflow",
        model_id: "Qwen/Qwen3-235B-A22B",
        name: "Qwen/Qwen3-235B-A22B",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 262144,
        enabled: false,
        param_overrides: null,
      },
      {
        provider_id: "builtin-siliconflow",
        model_id: "Qwen/Qwen3-32B",
        name: "Qwen/Qwen3-32B",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 262144,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 8,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-glm",
    name: "GLM",
    provider_type: "openai",
    api_host: "https://open.bigmodel.cn/api/paas",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-glm",
        model_id: "glm-5",
        name: "glm-5",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 128000,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-glm",
        model_id: "glm-4-plus",
        name: "glm-4-plus",
        capabilities: ["TextGeneration", "Vision", "FunctionCalling"],
        max_tokens: 128000,
        enabled: false,
        param_overrides: null,
      },
      {
        provider_id: "builtin-glm",
        model_id: "glm-4-flash",
        name: "glm-4-flash",
        capabilities: ["TextGeneration", "Vision"],
        max_tokens: 128000,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 6,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-minimax",
    name: "MiniMax",
    provider_type: "openai",
    api_host: "https://api.minimaxi.com",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-minimax",
        model_id: "MiniMax-M3",
        name: "MiniMax-M3",
        capabilities: ["TextGeneration", "Reasoning", "FunctionCalling"],
        max_tokens: 1000000,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-minimax",
        model_id: "MiniMax-S1",
        name: "MiniMax-S1",
        capabilities: ["TextGeneration"],
        max_tokens: 245760,
        enabled: false,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 7,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
  {
    id: "builtin-nvidia",
    name: "NVIDIA",
    provider_type: "openai",
    api_host: "https://integrate.api.nvidia.com/v1",
    api_path: null,
    enabled: true,
    models: [
      {
        provider_id: "builtin-nvidia",
        model_id: "meta/llama-4-maverick-17b-128e-instruct",
        name: "Llama 4 Maverick",
        capabilities: ["TextGeneration", "FunctionCalling"],
        max_tokens: 128000,
        enabled: true,
        param_overrides: null,
      },
      {
        provider_id: "builtin-nvidia",
        model_id: "deepseek-ai/deepseek-v3",
        name: "DeepSeek V3",
        capabilities: ["TextGeneration", "Reasoning"],
        max_tokens: 128000,
        enabled: true,
        param_overrides: null,
      },
    ],
    keys: [],
    proxy_config: null,
    sort_order: 8,
    created_at: 1700000000000,
    updated_at: 1700000000000,
  },
];

function initProviders(): Record<string, unknown>[] {
  const existing = getStore<Record<string, unknown>[]>("providers", []);
  if (existing.length === 0) {
    setStore("providers", BUILT_IN_PROVIDERS);
    return [...BUILT_IN_PROVIDERS];
  }
  // Restore missing models for built-in providers (e.g. after a bad fetch_remote_models wipe)
  let dirty = false;
  const existingMap = new Map(existing.map((p) => [p.id, p]));
  for (const builtin of BUILT_IN_PROVIDERS) {
    const stored = existingMap.get(builtin.id) as
      | (Provider & { models?: Array<{ model_id: string; name: string }> })
      | undefined;
    if (stored && (!stored.models || stored.models.length === 0)) {
      stored.models = [...builtin.models] as typeof stored.models;
      dirty = true;
    }
  }
  if (dirty) {
    setStore("providers", existing);
  }
  return existing;
}

// ── Default Settings ────────────────────────────────────────────────────

const DEFAULT_SETTINGS = {
  theme_mode: "system",
  primary_color: "#17A93D",
  font_size: 14,
  language: "zh-CN",
  send_on_enter: true,
  stream_response: true,
  global_shortcut: "CmdOrCtrl+Shift+A",
  shortcut_toggle_current_window: "CmdOrCtrl+Shift+A",
  shortcut_toggle_all_windows: "CmdOrCtrl+Shift+Alt+A",
  shortcut_close_window: "CmdOrCtrl+Shift+W",
  shortcut_new_conversation: "CmdOrCtrl+N",
  shortcut_open_settings: "CmdOrCtrl+,",
  shortcut_toggle_model_selector: "CmdOrCtrl+Shift+M",
  shortcut_fill_last_message: "CmdOrCtrl+Shift+ArrowUp",
  shortcut_clear_context: "CmdOrCtrl+Shift+K",
  shortcut_clear_conversation_messages: "CmdOrCtrl+Shift+Backspace",
  shortcut_toggle_gateway: "CmdOrCtrl+Shift+G",
  global_shortcuts_enabled: true,
  shortcut_registration_logs_enabled: false,
  shortcut_trigger_toast_enabled: false,
  proxy_enabled: false,
  proxy_url: "",
  auto_backup: false,
  backup_interval_hours: 24,
  content_safety_enabled: true,
  last_selected_conversation_id: null,
  onboarding_completed: true,
  onboarding_wizard_dismissed: true,
  onboarding_tutorial_completed: true,
};

// ── Command Handler ─────────────────────────────────────────────────────

// ── DynamicUI Mock 辅助函数 ──────────────────────────────────────
// i18n-exempt: Mock data keys for localStorage, not user-facing.
function loadMockDynamicUIData<T>(key: string, defaultValue: T): T {
  try {
    const data = localStorage.getItem(`axagent.mock.dynamicUI.${key}`);
    return data ? (JSON.parse(data) as T) : defaultValue;
  } catch {
    return defaultValue;
  }
}

function saveMockDynamicUIData<T>(key: string, data: T): void {
  try {
    localStorage.setItem(`axagent.mock.dynamicUI.${key}`, JSON.stringify(data));
  } catch (e) {
    console.warn(`Failed to write localStorage key: axagent.mock.dynamicUI.${key}`, e);
  }
}

/** 语义化版本号 patch 自增（不传 version 时使用） */
function bumpPatchVersion(version: string): string {
  const parts = version.split(".");
  if (parts.length !== 3) {
    return version;
  }
  const patch = parseInt(parts[2], 10);
  if (isNaN(patch)) {
    return version;
  }
  return `${parts[0]}.${parts[1]}.${patch + 1}`;
}

/** 生成一个简单的 mock UISchema JSON（Column + Text 结构） */
function buildMockUISchemaJSON(): string {
  return JSON.stringify({
    version: "1.0",
    id: "mock-root",
    type: "Column",
    props: {},
    children: [
      {
        version: "1.0",
        id: "mock-text",
        type: "Text",
        props: { content: "Mock dynamic UI content" },
      },
    ],
  });
}

/**
 * `content_media` 的 7 个 KPI 夹具（浏览器模式专用）。
 *
 * **三态刻意齐全**：`available` × 5（其中 `conversion_rate = 0` 是「真实观测恰为
 * 0」、`word_count = 200` 越限）、`empty` × 1（`completion_rate`）、
 * `no_data_source` × 1（`content_engagement`）。后两态在后端 `value` 是占位 `0.0`，
 * 验收时要能一眼看出它们渲染的是「—」+ 状态词，而不是被伪造成 `0`；而
 * `conversion_rate` 恰好证明「available 且为 0」**必须**显示 `0`。
 *
 * 数值与阈值取自 `config/opc/domain_packs/content_media/runtime.yaml`
 * （`word_count.risk.min = 1000`、`completion_rate.risk.min = 100`），因此该夹具
 * 对应的风控结论是 **1/2 个「已声明生效阈值的受管键」越限 ⇒ `medium`**，
 * 不是 `critical`（不是全部受管键都违规）。
 */
function mockContentMediaKpis(): Array<Record<string, unknown>> {
  const ts = nowTs();
  const k = (
    key: string,
    name: string,
    value: number,
    unit: string | null,
    availability: "available" | "empty" | "no_data_source",
    note?: string,
  ) => ({
    key,
    id: key,
    name,
    value,
    target: null,
    unit,
    timestamp: ts,
    availability,
    ...(note ? { note } : {}),
  });
  return [
    k("content_count", "内容数量", 25, "篇", "available"),
    k("page_views", "页面浏览量", 12500, "次", "available"),
    k("conversion_rate", "转化率", 0, "%", "available"),
    k(
      "content_engagement",
      "内容互动率",
      0,
      "%",
      "no_data_source",
      "无互动数据存储：opc_blog_posts 仅 view_count，无 like/comment/share 列或互动表",
    ),
    k("word_count", "创作字数", 200, "字", "available"),
    k(
      "completion_rate",
      "完成率",
      0,
      "%",
      "empty",
      "opc_kpi_records 中暂无 completion_rate 记录（工作流产出经 post_exec 钩子入库后可见）",
    ),
    k("revision_rounds", "修改轮次", 1, "轮", "available"),
  ];
}

/**
 * 真实后端下发 **snake_case** DTO 的命令（出口不做 camel 转换）。
 *
 * 本文件绝大多数 DTO 在 Rust 侧声明了 `#[serde(rename_all = "camelCase")]`
 * （如 `ScanPolicy`），mock 照着写 camelCase 键即可，出口的 `convertToCamelCase`
 * 对它们是空操作。
 *
 * 但下列 DTO **没有**该属性，生产环境下发的就是下划线键，前端也照此读取。
 * 而 `snakeToCamel` 只删下划线、**不可能反向生成**，于是 mock 无论怎么写都还原不出
 * 后端形态 —— 只能在这里豁免出口转换：
 *
 * * `OpcDomainDecision`（`opc::analysis::OpcDomainDecision`）：`domain_pack_id` /
 *   `decision_type` / `risk_level`。
 * * `SchemaStatus`（`dao::migrations`，经 `db_config.rs` 的 `get_schema_status`）：
 *   `tables_expected` / `pending_apply` / `probe_error` …
 * * `SchemaRepairReport`（同上，经 `repair_schema`）：`tables_scanned` /
 *   `columns_added` / `types_healed` / `errors`。
 * * `DbConfig`（`dao::config`，经 `db_config.rs` 的 `get_db_config`）：`db_type` /
 *   `sqlite_path` / `pg_host` / `pg_port` / … —— 前端 `DbConfigForm` 也按
 *   snake_case 读写（`DatabaseSettings.tsx:8-18`）。
 *
 * ⚠ 后三者**只补桩不登记**不会报任何错，但出口会把 `tables_expected` 转成
 * `tablesExpected` ⇒ `DatabaseSettings` 读到的字段**全是 `undefined`**
 * （`notes.map` 那一步会直接抛错、整页被 `PageErrorBoundary` 兜住）。新增桩时
 * 必须同时登记到这里。
 *
 * 曾试过「snake 与 camel 键双写」：无效，两把键会归并到同一个 camel 键上，
 * snake 键被吃掉，前端读到的仍是 `undefined`。
 *
 * ## 准入判据（成员资格必须可判定，双向锁）
 *
 * **准入判据 = 该命令的「成功响应载荷里含 snake_case 字段」**（不做转换会被出口
 * 归一成 camel 而丢键）。响应无载荷（返回 `()` / 直接 `Err`）的命令**不属于**本集合。
 *
 * ⇒ 同域的 `save_db_config`（返回 `()`）与 `test_db_connection`（成功时返回 `()`，
 * 浏览器模式下更是一直 `Err`）**刻意不登记**：它们没有任何键可被转换，登记与否
 * 等价，若登记进来就会让「是否在集合里」不再等价于「是否需要豁免」，集合与本文
 * 自相矛盾，后来者无法靠成员资格推出该做什么。
 */
const SNAKE_CASE_RESPONSE_COMMANDS = new Set<string>([
  "opc_execute_analysis",
  "get_schema_status",
  "repair_schema",
  "get_db_config",
]);

/**
 * 浏览器模式下 `get_schema_status` 的响应（字段集与 `dao::migrations::SchemaStatus`
 * 的 11 个字段齐全一致）。
 *
 * ⚠ **刻意让 `probe_error` 非 null**：浏览器模式没有真实数据库，就没有结构可探测，
 * 组件据此走 `schemaStatusProbeFailed` 的 warning 分支 —— 这才是诚实的。
 * 绝不能返回「看起来已收敛」的形态（`tables_actual === tables_expected` 且
 * `probe_error: null`），那会让卡片在浏览器模式显示「结构已收敛」，又造出一张
 * 说谎的卡片。
 *
 * ⚠ 每个字段都必须在：组件对 `notes` 直接 `.map`，缺它就是整页崩。
 * `dialect` 给空串与真实后端一致 —— 探测失败时后端也读不出方言。
 */
function mockSchemaStatus(): Record<string, unknown> {
  return {
    dialect: "",
    tables_expected: 0,
    tables_actual: 0,
    pending_apply: 0,
    pending_unsupported: 0,
    pending_manual: 0,
    advisories: 0,
    notes: [],
    applied_version: 0,
    latest_version: 0,
    probe_error: "浏览器模式（localStorage mock）无真实数据库，结构无法探测",
  };
}

/**
 * 浏览器模式下 `db_config.json` 的替身（localStorage key：`axagent_db_config`）。
 *
 * ⚠ 键名是 **snake_case**：`dao::config::DbConfig` 没有 `rename_all = "camelCase"`，
 * 前端 `DbConfigForm` 也照 snake_case 读写。因此 `get_db_config` 必须登记进
 * `SNAKE_CASE_RESPONSE_COMMANDS`，否则出口会把 `db_type` 转成 `dbType`，
 * 表单每个字段都读成 `undefined`（静默错值，不报错）。
 *
 * 值逐字照抄 `DbConfig::default()`（`dao/src/config.rs:37-54`）——
 * `get_db_config` 在 `db_config.json` 不存在时**原样返回该默认值**，
 * 浏览器模式没有配置文件、等价于「从未保存过」，故必须给默认值。
 * 返回空对象会让表单停在 `initialValues`（只有 3 个字段），与真实后端不一致，
 * 也让「用户看到的 host/port/database」变成组件默认而非配置默认 —— 又是一处
 * 只在浏览器模式存在的偏差。
 *
 * ⚠ `None` 在 Rust 侧序列化成 `null`（`DbConfig` 字段没有 `skip_serializing_if`），
 * 所以这里也写 `null` 键而不是省略键：省略会让两种模式的响应**形状**不一致，
 * 而形状不一致正是本轮修的那类 bug 的温床。antd 的 `Input` 走
 * `fixControlledValue`，`null` 会渲染成空串，不会产生受控/非受控切换告警。
 *
 * ⚠ `pg_password_enc` 保留但不含密文（`null`）：真实后端 `get_db_config` 会
 * 先 `take()` 走该字段、把解密后的明文放进 `pg_password`，所以真实响应里
 * 该键存在且为 `null`。浏览器模式没有 master.key、也没有密文，一致。
 */
function mockDbConfig(): Record<string, unknown> {
  return {
    db_type: "sqlite",
    sqlite_path: null,
    pg_host: "localhost",
    pg_port: 5432,
    pg_database: "axagent",
    pg_user: "postgres",
    pg_password: null,
    pg_password_enc: null,
    pg_schema: null,
    use_ssl: false,
    fallback_to_sqlite: true,
  };
}

export async function handleCommand<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  await new Promise((r) => setTimeout(r, 5));

  // 将前端 camelCase 参数转换为后端 snake_case 格式
  const convertedArgs = args ? convertToSnakeCase(args) : args;

  // 调用实际的命令处理逻辑
  const result = await executeCommand<T>(cmd, convertedArgs);

  // 少数命令的响应是 snake_case DTO（见上），转换会破坏字段名 ⇒ 原样返回。
  if (SNAKE_CASE_RESPONSE_COMMANDS.has(cmd)) {
    return result;
  }

  // 将后端 snake_case 返回值转换为前端 camelCase 格式
  return convertToCamelCase(result);
}

async function executeCommand<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  switch (cmd) {
    // ── Settings ──────────────────────────────────────────────────────
    case "get_settings":
      return getStore("settings", DEFAULT_SETTINGS) as T;
    case "save_settings": {
      const settings = (args as { settings?: Partial<Settings> }).settings ?? {};
      const current = getStore<Settings>(
        "settings",
        DEFAULT_SETTINGS as Settings,
      );
      const merged = { ...current, ...settings };
      setStore("settings", merged);
      return merged as T;
    }

    // ── 数据库结构状态（`Settings → 数据库` 卡片） ────────────────────────
    // ⚠ 两个命令都必须在 `SNAKE_CASE_RESPONSE_COMMANDS` 中登记（理由见该处注释）。
    case "get_schema_status":
      // 不返回「已收敛」形态：`probe_error` 非 null 才是浏览器模式的真实处境，
      // 否则卡片会说谎（详见 `mockSchemaStatus` 注释）。
      return mockSchemaStatus() as T;
    case "repair_schema":
      // 必须**抛错**而不是返回空报告：返回 `errors: []` 的报告会被组件读成
      // 「跑过了、修了 0 个」，返回 `errors` 非空则被读成「跑过了、部分失败」——
      // 两种都在暗示修复真的执行过。这里根本没跑，只有 Err 的语义是对的。
      throw new Error("浏览器模式（localStorage mock）无真实数据库，无法执行结构修复");

    // ── 数据库连接配置（`Settings → 数据库` 卡片） ───────────────────────
    // ⚠ 只有 `get_db_config` 登记进 `SNAKE_CASE_RESPONSE_COMMANDS`（它下发
    // `DbConfig` 的下划线键）；另两个响应无载荷，按该集合的准入判据**不属于**它，
    // 理由见集合处注释。
    case "get_db_config":
      // 真实后端在 `db_config.json` 不存在时返回 `DbConfig::default()`
      // （`db_config.rs:40-42`），这里以同一份默认值兜底（详见 `mockDbConfig`）。
      // 修前落 default 分支的 `get_*` ⇒ `{}`，表单所有字段读成 `undefined`。
      return getStore("db_config", mockDbConfig()) as T;
    case "save_db_config": {
      // 真实后端是**整体覆盖** `db_config.json`，不做字段级合并（`db_config.rs:78-107`），
      // 这里保持一致：存什么、`get_db_config` 就回什么（往返一致）。
      //
      // ⚠ 刻意**不模拟**密码加密：真实后端用 master.key（Aes256Gcm）把 `pg_password`
      // 加密成 `pg_password_enc` 后落盘、明文不落盘；浏览器模式没有 master.key，
      // 编一个假密文比留 `null` 更容易骗人（会被读成「已加密」）。故这里只存不加密，
      // 让 `pg_password` 原样往返 —— 「没有加密能力」是浏览器模式的真实处境。
      const config = (args as { config?: Record<string, unknown> }).config ?? {};
      setStore("db_config", config);
      // 真实后端返回 `Result<(), String>` ⇒ 序列化为 `null`（前端也不读返回值）。
      return null as T;
    }
    case "test_db_connection":
      // 必须**抛错**：真实后端在这里会真的建连接并跑 `SELECT 1`，
      // 失败时返回结构化错误码（`db_config.rs` 的 `test_db_connection`）。
      // 返回任何成功值都会让 `handleTest` 走到 `message.success(...)` ⇒ 显示一个
      // 从未发生过的「连接成功」；只有 Err 的语义是对的。
      //
      // 这里抛的是**纯文本** Error（不含 `{code,...}` JSON）：浏览器模式没有真实
      // 数据库、也就没有真实 sqlx 错误可归因，编一个错误码等于伪造证据。
      // `handleTest` 的 catch 走 `translateBackendError` 时无码可查 ⇒ 原样回退原文，
      // 正是应有的表现。
      throw new Error("浏览器模式（localStorage mock）无真实数据库，无法测试连接");

    // ── Providers ─────────────────────────────────────────────────────
    case "list_providers":
      return initProviders() as T;
    case "create_provider": {
      const input = (args as { input?: Partial<Provider> }).input
        ?? ({} as Partial<Provider>);
      const id = genId();
      const now = nowTs();
      const provider: Provider = {
        id,
        name: input.name ?? "",
        provider_type: input.provider_type ?? "",
        api_host: input.api_host ?? "",
        enabled: input.enabled ?? true,
        models: input.models ?? [],
        keys: [],
        proxy_config: null,
        created_at: now,
        updated_at: now,
      };
      const providers = getStore<Provider[]>("providers", []);
      providers.push(provider);
      setStore("providers", providers);
      return provider as T;
    }
    case "update_provider": {
      const { id, input } = args as { id?: string; input?: Partial<Provider> };
      const providers = getStore<Provider[]>("providers", []);
      const idx = providers.findIndex((p) => p.id === id);
      if (idx === -1) {
        throw new Error("Provider not found");
      }
      if (input?.name !== undefined) {
        providers[idx].name = input.name;
      }
      if (input?.provider_type !== undefined) {
        providers[idx].provider_type = input.provider_type;
      }
      if (input?.api_host !== undefined) {
        providers[idx].api_host = input.api_host;
      }
      if (input?.enabled !== undefined) {
        providers[idx].enabled = input.enabled;
      }
      if (input?.api_path !== undefined) {
        providers[idx].api_path = input.api_path;
      }
      if (input?.sort_order !== undefined) {
        providers[idx].sort_order = input.sort_order;
      }
      providers[idx].updated_at = nowTs();
      setStore("providers", providers);
      return providers[idx] as T;
    }
    case "delete_provider": {
      const { id } = args as { id?: string };
      const providers = getStore<Provider[]>("providers", []).filter(
        (p) => p.id !== id,
      );
      setStore("providers", providers);
      return undefined as T;
    }
    case "reorder_providers": {
      const { providerIds } = args as { providerIds?: string[] };
      const providers = getStore<Provider[]>("providers", []);
      if (providerIds) {
        const providerMap = new Map(providers.map((p) => [p.id, p]));
        for (let i = 0; i < providerIds.length; i++) {
          const p = providerMap.get(providerIds[i]);
          if (p) {
            p.sort_order = i;
          }
        }
        providers.sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0));
        setStore("providers", providers);
      }
      return undefined as T;
    }
    case "toggle_provider": {
      const { id, enabled } = args as { id?: string; enabled?: boolean };
      const providers = getStore<Provider[]>("providers", []);
      const idx = providers.findIndex((p) => p.id === id);
      if (idx !== -1) {
        providers[idx].enabled = enabled ?? false;
        providers[idx].updated_at = nowTs();
        setStore("providers", providers);
      }
      return undefined as T;
    }
    case "list_narrative_structures": {
      const { isTemplate, genre } = args as { isTemplate?: boolean; genre?: string };
      const all = getStore<NarrativeStructureRecord[]>("narrative_structures", []);
      return all.filter((n) =>
        (isTemplate === undefined || n.isTemplate === isTemplate)
        && (genre === undefined || n.genre === genre)
      ) as T;
    }
    case "get_narrative_structure": {
      const { id } = args as { id?: string };
      const found = getStore<NarrativeStructureRecord[]>("narrative_structures", []).find((n) => n.id === id) ?? null;
      return found as T;
    }
    case "create_narrative_structure": {
      const input = (args as { input?: CreateNarrativeRequest }).input ?? {} as CreateNarrativeRequest;
      const now = nowTs();
      const rec: NarrativeStructureRecord = {
        id: input.id || genId(),
        name: input.name,
        description: input.description,
        genre: input.genre,
        structure: input.structure,
        isTemplate: input.isTemplate ?? false,
        version: 1,
        createdAt: now,
        updatedAt: now,
      };
      const all = getStore<NarrativeStructureRecord[]>("narrative_structures", []);
      all.push(rec);
      setStore("narrative_structures", all);
      return rec as T;
    }
    case "update_narrative_structure": {
      const input = (args as { input?: Partial<NarrativeStructureRecord> }).input ?? {};
      const all = getStore<NarrativeStructureRecord[]>("narrative_structures", []);
      const idx = all.findIndex((n) => n.id === input.id);
      if (idx !== -1) {
        all[idx] = {
          ...all[idx],
          ...input,
          version: (all[idx].version ?? 1) + 1,
          updatedAt: nowTs(),
        };
        setStore("narrative_structures", all);
        return all[idx] as T;
      }
      throw new Error("NarrativeStructure not found");
    }
    case "delete_narrative_structure": {
      const { id } = args as { id?: string };
      const all = getStore<NarrativeStructureRecord[]>("narrative_structures", []);
      setStore(
        "narrative_structures",
        all.filter((n) => n.id !== id),
      );
      return undefined as T;
    }
    case "save_skill_workflow_from_llm": {
      // 浏览器 mock：直接保存成功，不模拟相似审查
      return {
        needsReview: false,
        workflowId: genId(),
        similarWorkflows: [],
      } as T;
    }
    case "add_provider_key": {
      // 命令参数名：Tauri 侧为原生 snake_case（provider_id / raw_key），
      // 部分调用方写 camelCase。mock 分支两种都接受，否则取到 undefined
      // 会导致 key 被静默丢弃（找不到目标 provider，函数仍正常返回）。
      const rawArgs = args as {
        providerId?: string;
        provider_id?: string;
        rawKey?: string;
        raw_key?: string;
      };
      const providerId = rawArgs.providerId ?? rawArgs.provider_id ?? "";
      const rawKey = rawArgs.rawKey ?? rawArgs.raw_key ?? "";
      // SECURITY (S5): 浏览器 mock 模式下，对 API Key 进行 base64 编码存储，防止明文泄露
      const encodedKey = rawKey ? btoa(rawKey) : "";
      console.warn(
        "[browserMock] SECURITY: API key is stored with obfuscation in localStorage. Do NOT use browser mock mode in production.",
      );
      const key: ProviderKey = {
        id: genId(),
        provider_id: providerId ?? "",
        key_encrypted: encodedKey,
        key_prefix: (rawKey ?? "").substring(0, 8) + "...",
        enabled: true,
        last_validated_at: null,
        last_error: null,
        rotation_index: 0,
        created_at: nowTs(),
      };
      const providers = getStore<Provider[]>("providers", []);
      const idx = providers.findIndex((p) => p.id === providerId);
      if (idx !== -1) {
        providers[idx].keys.push(key);
        setStore("providers", providers);
      }
      return key as T;
    }
    case "delete_provider_key": {
      const { keyId } = args as { keyId?: string };
      const providers = getStore<Provider[]>("providers", []);
      for (const p of providers) {
        p.keys = p.keys.filter((k) => k.id !== keyId);
      }
      setStore("providers", providers);
      return undefined as T;
    }
    case "toggle_provider_key": {
      const { keyId, enabled } = args as { keyId?: string; enabled?: boolean };
      const providers = getStore<Provider[]>("providers", []);
      for (const p of providers) {
        for (const k of p.keys) {
          if (k.id === keyId) {
            k.enabled = enabled ?? true;
          }
        }
      }
      setStore("providers", providers);
      return undefined as T;
    }
    case "validate_provider_key":
      return true as T;
    case "save_models": {
      const { providerId, models } = args as {
        providerId?: string;
        models?: Array<{
          model_id: string;
          name: string;
          mode?: string;
          enabled?: boolean;
        }>;
      };
      const providers = getStore<Provider[]>("providers", []);
      const idx = providers.findIndex((p) => p.id === providerId);
      if (idx !== -1 && models) {
        providers[idx].models = models;
        setStore("providers", providers);
      }
      return undefined as T;
    }
    case "toggle_model": {
      const { providerId, modelId, enabled } = args as {
        providerId?: string;
        modelId?: string;
        enabled?: boolean;
      };
      const providers = getStore<Provider[]>("providers", []);
      const pIdx = providers.findIndex((p) => p.id === providerId);
      if (pIdx !== -1) {
        const model = providers[pIdx].models.find(
          (m) => m.model_id === modelId,
        );
        if (model) {
          model.enabled = enabled;
          setStore("providers", providers);
          return model as T;
        }
      }
      throw new Error("Model not found");
    }
    case "update_model_params": {
      const { providerId, modelId, overrides } = args as {
        providerId?: string;
        modelId?: string;
        overrides?: Record<string, unknown>;
      };
      const providers = getStore<Provider[]>("providers", []);
      const pIdx = providers.findIndex((p) => p.id === providerId);
      if (pIdx !== -1) {
        const model = providers[pIdx].models.find(
          (m) => m.model_id === modelId,
        );
        if (model) {
          (model as Record<string, unknown>).param_overrides = overrides;
          setStore("providers", providers);
          return model as T;
        }
      }
      throw new Error("Model not found");
    }
    case "fetch_remote_models": {
      const providers = getStore<Provider[]>("providers", []);
      const target = providers.find(
        (p) => p.id === (args as { providerId?: string }).providerId,
      );
      return (target?.models ?? []) as T;
    }

    // ── Conversations ─────────────────────────────────────────────────
    case "list_conversations":
      return getStore<Conversation[]>("conversations", []).filter(
        (c) => !c.isArchived,
      ) as T;
    case "list_archived_conversations":
      return getStore<Conversation[]>("conversations", []).filter(
        (c) => c.isArchived,
      ) as T;
    case "create_conversation": {
      const { title, modelId, providerId, systemPrompt } = args as Record<
        string,
        unknown
      >;
      const conv = {
        id: genId(),
        title,
        model_id: modelId,
        provider_id: providerId,
        system_prompt: systemPrompt || null,
        temperature: null,
        max_tokens: null,
        top_p: null,
        frequency_penalty: null,
        search_enabled: false,
        search_provider_id: null,
        thinking_budget: null,
        enabled_mcp_server_ids: [],
        enabled_knowledge_base_ids: [],
        enabled_memory_namespace_ids: [],
        message_count: 0,
        is_pinned: false,
        is_archived: false,
        created_at: nowTs(),
        updated_at: nowTs(),
      };
      const convs = getStore<Record<string, unknown>[]>("conversations", []);
      convs.push(conv);
      setStore("conversations", convs);
      return conv as T;
    }
    case "update_conversation": {
      const { id, input } = args as {
        id?: string;
        input?: Partial<Conversation>;
      };
      const convs = getStore<Conversation[]>("conversations", []);
      const idx = convs.findIndex((c) => c.id === id);
      if (idx !== -1 && input) {
        if (input.title !== undefined) {
          convs[idx].title = input.title;
        }
        if (input.categoryId !== undefined) {
          convs[idx].categoryId = input.categoryId;
        }
        if (input.providerId !== undefined) {
          convs[idx].providerId = input.providerId;
        }
        if (input.modelId !== undefined) {
          convs[idx].modelId = input.modelId;
        }
        if (input.temperature !== undefined) {
          convs[idx].temperature = input.temperature;
        }
        if (input.maxTokens !== undefined) {
          convs[idx].maxTokens = input.maxTokens;
        }
        if (input.topP !== undefined) {
          convs[idx].topP = input.topP;
        }
        if (input.frequencyPenalty !== undefined) {
          convs[idx].frequencyPenalty = input.frequencyPenalty;
        }
        convs[idx].updatedAt = nowTs();
        setStore("conversations", convs);
        return convs[idx] as T;
      }
      throw new Error("Conversation not found");
    }
    case "delete_conversation": {
      const { id } = args as { id?: string };
      const convs = getStore<Conversation[]>("conversations", []).filter(
        (c) => c.id !== id,
      );
      setStore("conversations", convs);
      const msgs = getStore<Message[]>("messages", []).filter(
        (m) => m.conversationId !== id,
      );
      setStore("messages", msgs);
      return undefined as T;
    }
    case "toggle_pin_conversation": {
      const { id } = args as { id?: string };
      const convs = getStore<Conversation[]>("conversations", []);
      const idx = convs.findIndex((c) => c.id === id);
      if (idx !== -1) {
        convs[idx].isPinned = !convs[idx].isPinned;
        convs[idx].updatedAt = nowTs();
        setStore("conversations", convs);
        return convs[idx] as T;
      }
      throw new Error("Conversation not found");
    }
    case "toggle_archive_conversation": {
      const { id } = args as { id?: string };
      const convs = getStore<Conversation[]>("conversations", []);
      const idx = convs.findIndex((c) => c.id === id);
      if (idx !== -1) {
        convs[idx].isArchived = !convs[idx].isArchived;
        convs[idx].updatedAt = nowTs();
        setStore("conversations", convs);
        return convs[idx] as T;
      }
      throw new Error("Conversation not found");
    }
    case "list_conversation_categories":
      return getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      ) as T;
    case "create_conversation_category": {
      const { input } = args as { input: ConversationCategory };
      const cats = getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      );
      const maxOrder = cats.reduce(
        (m: number, c) => Math.max(m, c.sortOrder ?? 0),
        -1,
      );
      const cat: ConversationCategory = {
        id: genId(),
        name: input.name,
        iconType: input.iconType ?? null,
        iconValue: input.iconValue ?? null,
        systemPrompt: input.systemPrompt ?? null,
        defaultModel: input.defaultModel ?? null,
        defaultTemperature: input.defaultTemperature ?? null,
        defaultMaxTokens: input.defaultMaxTokens ?? null,
        defaultTopP: input.defaultTopP ?? null,
        defaultFrequencyPenalty: input.defaultFrequencyPenalty ?? null,
        sortOrder: maxOrder + 1,
        isCollapsed: false,
        createdAt: nowTs(),
        updatedAt: nowTs(),
      };
      cats.push(cat);
      setStore("conversation_categories", cats);
      return cat as T;
    }
    case "update_conversation_category": {
      const { id, input } = args as {
        id: string;
        input: Partial<ConversationCategory>;
      };
      const cats = getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      );
      const idx = cats.findIndex((c) => c.id === id);
      if (idx !== -1) {
        if (input.name !== undefined) {
          cats[idx].name = input.name;
        }
        if (input.iconType !== undefined) {
          cats[idx].iconType = input.iconType;
        }
        if (input.iconValue !== undefined) {
          cats[idx].iconValue = input.iconValue;
        }
        if (input.systemPrompt !== undefined) {
          cats[idx].systemPrompt = input.systemPrompt;
        }
        if (input.defaultModel !== undefined) {
          cats[idx].defaultModel = input.defaultModel;
        }
        if (input.defaultTemperature !== undefined) {
          cats[idx].defaultTemperature = input.defaultTemperature;
        }
        if (input.defaultMaxTokens !== undefined) {
          cats[idx].defaultMaxTokens = input.defaultMaxTokens;
        }
        if (input.defaultTopP !== undefined) {
          cats[idx].defaultTopP = input.defaultTopP;
        }
        if (input.defaultFrequencyPenalty !== undefined) {
          cats[idx].defaultFrequencyPenalty = input.defaultFrequencyPenalty;
        }
        cats[idx].updatedAt = nowTs();
        setStore("conversation_categories", cats);
        return cats[idx] as T;
      }
      throw new Error("Category not found");
    }
    case "delete_conversation_category": {
      const { id } = args as { id: string };
      const cats = getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      ).filter((c) => c.id !== id);
      setStore("conversation_categories", cats);
      const convs = getStore<Conversation[]>("conversations", []);
      convs.forEach((c) => {
        if (c.categoryId === id) {
          c.categoryId = null;
        }
      });
      setStore("conversations", convs);
      return undefined as T;
    }
    case "reorder_conversation_categories": {
      const { categoryIds } = args as { categoryIds: string[] };
      const cats = getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      );
      const catMap = new Map(cats.map((c) => [c.id, c]));
      for (let i = 0; i < categoryIds.length; i++) {
        const c = catMap.get(categoryIds[i]);
        if (c) {
          c.sortOrder = i;
        }
      }
      cats.sort((a, b) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0));
      setStore("conversation_categories", cats);
      return undefined as T;
    }
    case "set_conversation_category_collapsed": {
      const { id, collapsed } = args as { id?: string; collapsed?: boolean };
      const cats = getStore<ConversationCategory[]>(
        "conversation_categories",
        [],
      );
      const idx = cats.findIndex((c) => c.id === id);
      if (idx !== -1) {
        cats[idx].isCollapsed = collapsed ?? false;
        cats[idx].updatedAt = nowTs();
        setStore("conversation_categories", cats);
      }
      return undefined as T;
    }
    case "agent_get_session": {
      // 浏览器 mock：与会话层持久化对齐后端 upsert 语义（有则返回，无则创建）
      const { conversationId } = (args ?? {}) as { conversationId?: string };
      if (!conversationId) {
        return null as T;
      }
      const sessions = getStore<MockAgentSession[]>("agent_sessions", []);
      const existing = sessions.find((s) => s.conversationId === conversationId);
      if (existing) {
        existing.updatedAt = nowTs();
        setStore("agent_sessions", sessions);
        return {
          conversationId: existing.conversationId,
          name: existing.name,
          metadata: existing.metadata,
          createdAt: existing.createdAt,
          lastActiveAt: existing.updatedAt,
        } as T;
      }
      const createdAt = nowTs();
      const created: MockAgentSession = {
        conversationId,
        name: null,
        metadata: null,
        cwd: null,
        permissionMode: "default",
        createdAt,
        updatedAt: createdAt,
      };
      setStore("agent_sessions", [...sessions, created]);
      return {
        conversationId: created.conversationId,
        name: created.name,
        metadata: created.metadata,
        createdAt: created.createdAt,
        lastActiveAt: created.updatedAt,
      } as T;
    }
    case "agent_update_session": {
      // 浏览器 mock：持久化会话字段（cwd / permissionMode / name / metadata），模拟后端 upsert
      const { conversationId, cwd, permissionMode, name, metadata } = (args ?? {}) as {
        conversationId?: string;
        cwd?: string | null;
        permissionMode?: string | null;
        name?: string | null;
        metadata?: Record<string, unknown> | null;
      };
      if (!conversationId) {
        return {
          conversationId: "",
          name: null,
          metadata: null,
          cwd: null,
          permissionMode: "default",
        } as T;
      }
      const sessions = getStore<MockAgentSession[]>("agent_sessions", []);
      const now = nowTs();
      let target = sessions.find((s) => s.conversationId === conversationId);
      if (target) {
        if (cwd !== undefined) {
          target.cwd = cwd;
        }
        if (permissionMode !== undefined) {
          target.permissionMode = permissionMode ?? "default";
        }
        if (name !== undefined) {
          target.name = name;
        }
        if (metadata !== undefined) {
          target.metadata = metadata;
        }
        target.updatedAt = now;
      } else {
        target = {
          conversationId,
          name: name ?? null,
          metadata: metadata ?? null,
          cwd: cwd ?? null,
          permissionMode: permissionMode ?? "default",
          createdAt: now,
          updatedAt: now,
        };
        sessions.push(target);
      }
      setStore("agent_sessions", sessions);
      return {
        conversationId: target.conversationId,
        name: target.name,
        metadata: target.metadata,
        cwd: target.cwd,
        permissionMode: target.permissionMode,
      } as T;
    }
    case "agent_ensure_workspace": {
      const workspacePath = "/mock/workspace/" + Date.now();
      return { workspacePath } as T;
    }
    case "list_agency_experts": {
      return [] as T;
    }
    case "cognitive_query": {
      // 兼容 camelCase 和 snake_case 参数格式
      const rawArgs = (args ?? {}) as {
        request?: {
          conversationId?: string;
          conversation_id?: string;
          input?: string;
        };
        conversationId?: string;
        conversation_id?: string;
        input?: string;
      };
      // 兼容 camelCase 与 snake_case：优先取嵌套的 `request`，其次把顶层直接当 `request`
      const req = rawArgs.request ?? rawArgs;
      const conversationId = req.conversationId ?? req.conversation_id ?? "";
      // 模拟认知编排器：根据用户输入智能匹配路由
      const userInput = req?.input ?? "";
      const route = mockCognitiveRoute(userInput);
      const assistantId = genId();
      emitBrowserEvent("agent-message-id", { conversationId, assistantMessageId: assistantId });
      emitBrowserEvent("agent-done", {
        conversationId,
        assistantMessageId: assistantId,
        text: i18n.t("browserMock.planCompleted"),
        thinking: "",
        usage: { inputTokens: 1, outputTokens: 1 },
      });
      return {
        routePath: route.routePath,
        domain: route.domain,
        cluster: route.cluster,
        capabilityId: route.capabilityId,
        confidence: 0.95,
        isLlmFallback: false,
        circuitBroken: false,
        circuitBreakReason: null,
        fallbackPath: null,
        candidates: route.candidates,
        executionMode: route.executionMode,
        stageRecords: [
          { stage: "l1_domain", result: route.domain, elapsedMs: 10 },
          { stage: "l2_cluster", result: route.cluster, elapsedMs: 20 },
          { stage: "l3_capability", result: route.capabilityId || "fallback", elapsedMs: 30 },
        ],
        totalElapsedMs: 60,
        execution: { kind: "agent", conversationId, assistantMessageId: assistantId },
      } as T;
    }
    case "agent_query": {
      // Browser mock: return immediately without error
      return undefined as T;
    }
    // ── agent 运行控制 mock（浏览器回退模式，避免抛错）──
    case "agent_pause":
    case "agent_resume":
    case "agent_cancel":
    case "agent_steer":
    case "agent_approve":
    case "agent_respond_ask":
    case "agent_backup_and_clear_sdk_context":
    case "agent_restore_sdk_context_from_backup": {
      return undefined as T;
    }
    case "agent_is_running":
    case "agent_is_paused": {
      return false as T;
    }
    case "agent_runtime_stats": {
      return {
        conversationId: (args as { conversation_id?: string } | undefined)?.conversation_id ?? "",
        running: false,
        paused: false,
        activeSessions: 0,
        pendingPermissions: 0,
        pendingAskUser: 0,
        activeToolCalls: 0,
        executionProgress: null,
      } as T;
    }
    case "agent_resolve_model": {
      return "gpt-4o-mini" as T;
    }
    case "simple_chat_completion": {
      // 模拟 LLM 生成智能体配置（AgentGeneratorModal 用）
      const mockJson = JSON.stringify({
        agent_type: "general-assistant",
        display_name: i18n.t("agentGenerator.mock.displayName"),
        description: i18n.t("agentGenerator.mock.description"),
        system_prompt: i18n.t("agentGenerator.mock.systemPrompt"),
        permissions: ["read", "write"],
        preferred_model: "gpt-4o-mini",
      });
      return mockJson as T;
    }
    case "plan_list": {
      return [] as T;
    }
    case "plan_generate": {
      // Browser mock: return a draft plan for review
      const { conversationId, content } = args as { conversationId: string; content: string };
      const planId = genId();
      const now = Date.now();
      return {
        id: planId,
        conversationId: conversationId,
        userMessageId: genId(),
        title: content?.slice(0, 60) || "Mock Plan",
        steps: [
          {
            id: genId(),
            title: i18n.t("browserMock.analyzeRequirements"),
            description: i18n.t("browserMock.understandGoal"),
            status: "pending",
            estimatedTools: ["Read"],
            result: null,
          },
          {
            id: genId(),
            title: i18n.t("browserMock.designPlan"),
            description: i18n.t("browserMock.implementSteps"),
            status: "pending",
            estimatedTools: ["Write"],
            result: null,
          },
          {
            id: genId(),
            title: i18n.t("browserMock.verifyResult"),
            description: i18n.t("browserMock.confirmCompletion"),
            status: "pending",
            estimatedTools: ["Bash"],
            result: null,
          },
        ],
        status: "reviewing",
        isActive: true,
        createdUnderStrategy: "plan",
        createdAt: now,
        updatedAt: now,
      } as T;
    }
    case "plan_execute": {
      // Browser mock: return immediately, execution handled by events
      return undefined as T;
    }
    case "plan_cancel": {
      return undefined as T;
    }
    case "plan_activate": {
      const { planId } = args as { planId: string; conversationId: string };
      const now = Date.now();
      return {
        id: planId,
        conversationId: (args as { conversationId: string }).conversationId,
        userMessageId: genId(),
        title: "Restored Plan",
        steps: [],
        status: "reviewing",
        isActive: true,
        createdUnderStrategy: "plan",
        createdAt: now,
        updatedAt: now,
      } as T;
    }
    case "plan_modify_step": {
      return undefined as T;
    }
    case "plan_authorize": {
      // P0-A 授权位写入。handleCommand 已把 args 递归转为 snake_case，
      // 且调用方以 { request: {...} } 包裹 → 先解出 request 层。
      const req = ((args as { request?: Record<string, unknown> }).request ?? args) as {
        plan_id?: string;
        conversation_id?: string;
        approved?: boolean;
        authorized_by?: string;
      };
      const approved = req.approved ?? false;
      const now = Date.now();
      return {
        id: req.plan_id ?? genId(),
        conversationId: req.conversation_id ?? "",
        userMessageId: genId(),
        title: "Mock Plan",
        steps: [],
        status: approved ? "approved" : "cancelled",
        executionAuthorized: approved,
        authorizedAt: approved ? now : undefined,
        authorizedBy: approved ? (req.authorized_by ?? "user") : undefined,
        isActive: approved,
        createdUnderStrategy: "plan",
        createdAt: now,
        updatedAt: now,
      } as T;
    }
    case "send_message": {
      const raw = (args as { params?: unknown }).params ?? args;
      const { conversationId, content, attachments } = raw as {
        conversationId: string;
        content: string;
        attachments?: unknown[];
      };
      const userMsgId = genId();
      const userMsg = {
        id: userMsgId,
        conversationId: conversationId,
        role: "user",
        content,
        thinking: null,
        attachments: attachments || [],
        createdAt: nowTs(),
        parentMessageId: null,
        versionIndex: 0,
        isActive: true,
      };
      const msgs = getStore<Record<string, unknown>[]>("messages", []);
      msgs.push(userMsg);

      // Generate a simulated AI response in browser mode
      const aiMsg = {
        id: genId(),
        conversationId: conversationId,
        role: "assistant",
        content: generateBrowserResponse(content),
        thinking: null,
        attachments: [],
        createdAt: nowTs() + 1,
        parentMessageId: userMsgId,
        versionIndex: 0,
        isActive: true,
      };
      msgs.push(aiMsg);
      setStore("messages", msgs);
      return userMsg as T;
    }
    case "list_messages": {
      const { conversationId } = args as { conversationId?: string };
      const msgs = getStore<Message[]>("messages", []).filter(
        (m) => m.conversationId === conversationId,
      );
      return msgs as T;
    }
    case "list_messages_page": {
      const {
        conversationId,
        limit = 10,
        beforeMessageId = null,
      } = args as {
        conversationId: string;
        limit?: number;
        beforeMessageId?: string | null;
      };
      const allMessages = getStore<Message[]>("messages", [])
        .filter((m) => m.conversationId === conversationId)
        .sort((a, b) => a.createdAt - b.createdAt);
      const cursorIndex = beforeMessageId
        ? allMessages.findIndex((m) => m.id === beforeMessageId)
        : allMessages.length;
      const endIndex = cursorIndex >= 0 ? cursorIndex : allMessages.length;
      const startIndex = Math.max(0, endIndex - (limit ?? 10));
      const pageMessages = allMessages.slice(startIndex, endIndex);
      return {
        messages: pageMessages,
        has_older: startIndex > 0,
        oldest_message_id: pageMessages[0]?.id ?? null,
      } as T;
    }
    case "search_conversations": {
      const { query } = args as { query: string };
      const convs = getStore<Conversation[]>("conversations", []);
      const results = convs.flatMap((c) =>
        c.title.toLowerCase().includes(query.toLowerCase())
          ? [{ conversation_id: c.id, title: c.title, snippet: "" }]
          : []
      );
      return results as T;
    }
    case "regenerate_message": {
      const regenRaw = (args as { params?: unknown }).params ?? args;
      const { conversationId: regenConvId, userMessageId: regenUserMsgId } = regenRaw as {
        conversationId?: string;
        userMessageId?: string;
      };
      const regenMsgs = getStore<Message[]>("messages", []);
      const convMsgs = regenMsgs.filter(
        (m) => m.conversationId === regenConvId,
      );
      let lastUserMsg: Message | null = null;
      if (regenUserMsgId) {
        // Fix: when userMessageId is explicitly specified, only use that exact message;
        // do NOT silently fall back to the last user message if not found.
        lastUserMsg = convMsgs.find(
          (m) => m.id === regenUserMsgId && m.role === "user",
        ) ?? null;
      } else {
        // Fallback: no specific userMessageId → find the last user message
        for (let i = convMsgs.length - 1; i >= 0; i--) {
          if (convMsgs[i].role === "user") {
            lastUserMsg = convMsgs[i];
            break;
          }
        }
      }
      if (lastUserMsg) {
        const existingVersions = regenMsgs.filter(
          (m) => m.parentMessageId === lastUserMsg!.id && m.role === "assistant",
        );
        const nextVersion = existingVersions.length;
        for (const m of regenMsgs) {
          if (
            m.parentMessageId === lastUserMsg!.id
            && m.role === "assistant"
          ) {
            m.isActive = false;
          }
        }
        // Create new AI version
        const newAiMsg: Message = {
          id: genId(),
          conversationId: regenConvId!,
          role: "assistant",
          content: generateBrowserResponse(lastUserMsg!.content),
          providerId: null,
          modelId: null,
          tokenCount: null,
          thinking: null,
          attachments: [],
          toolCallsJson: null,
          toolCallId: null,
          createdAt: nowTs(),
          parentMessageId: lastUserMsg!.id,
          versionIndex: nextVersion,
          isActive: true,
          status: "complete",
        };
        regenMsgs.push(newAiMsg);
        setStore("messages", regenMsgs);
      }
      return undefined as T;
    }
    case "list_message_versions": {
      const { parentMessageId } = args as { parentMessageId?: string };
      const allMsgs = getStore<Message[]>("messages", []);
      return allMsgs.filter(
        (m) => m.parentMessageId === parentMessageId,
      ) as T;
    }
    case "switch_message_version": {
      const { parentMessageId: switchParent, messageId: switchTarget } = args as {
        parentMessageId?: string;
        messageId?: string;
      };
      const switchMsgs = getStore<Message[]>("messages", []);
      for (const m of switchMsgs) {
        if (m.parentMessageId === switchParent && m.role === "assistant") {
          m.isActive = m.id === switchTarget;
        }
      }
      setStore("messages", switchMsgs);
      return undefined as T;
    }
    case "delete_message_group": {
      const { userMessageId } = args as { userMessageId?: string };
      const delMsgs = getStore<Message[]>("messages", []);
      const filtered = delMsgs.filter(
        (m) => m.id !== userMessageId && m.parentMessageId !== userMessageId,
      );
      setStore("messages", filtered);
      return undefined as T;
    }

    // ── Gateway ───────────────────────────────────────────────────────
    case "list_gateway_keys":
      return getStore<GatewayKey[]>("gateway_keys", []) as T;
    case "create_gateway_key": {
      const input = (args as { input?: Partial<GatewayKey> }).input ?? {};
      const key: GatewayKey = {
        id: genId(),
        name: input.name ?? "",
        keyHash: "",
        keyPrefix: "",
        enabled: input.enabled ?? true,
        createdAt: nowTs(),
        lastUsedAt: null,
        hasEncryptedKey: true,
      };
      const keys = getStore<GatewayKey[]>("gateway_keys", []);
      keys.push(key);
      setStore("gateway_keys", keys);
      return {
        gateway_key: key,
        plain_key: `sk-mock-plain-key-${genId().substring(0, 8)}`,
      } as T;
    }
    case "delete_gateway_key": {
      const { id } = args as { id?: string };
      const keys = getStore<GatewayKey[]>("gateway_keys", []).filter(
        (k) => k.id !== id,
      );
      setStore("gateway_keys", keys);
      return undefined as T;
    }
    case "toggle_gateway_key": {
      const { id, enabled } = args as { id?: string; enabled?: boolean };
      const keys = getStore<GatewayKey[]>("gateway_keys", []);
      const idx = keys.findIndex((k) => k.id === id);
      if (idx !== -1) {
        keys[idx].enabled = enabled ?? false;
        setStore("gateway_keys", keys);
      }
      return undefined as T;
    }
    case "get_gateway_metrics":
      // ⚠️ 字段名必须与**权威 DTO** 逐字一致：
      // 后端 `commands/gateway.rs::get_gateway_metrics` 返回 `axagent_harness::types::gateway::GatewayMetrics`，
      // 该结构体带 `#[serde(rename_all = "camelCase")]`，前端 `src/types/index.ts::GatewayMetrics` 也是 camelCase。
      // 此前 mock 返回的是另一套 snake_case 字段（total_requests / avg_latency_ms / uptime_seconds…），
      // 与实际 DTO **零字段重叠** ⇒ 浏览器模式下 GatewayMonitor 读 totalRequests 等恒为 undefined。
      return {
        totalRequests: 0,
        totalTokens: 0,
        totalRequestTokens: 0,
        totalResponseTokens: 0,
        activeConnections: 0,
        todayRequests: 0,
        todayTokens: 0,
        todayRequestTokens: 0,
        todayResponseTokens: 0,
        totalCostUsd: 0,
        todayCostUsd: 0,
      } as T;
    case "get_gateway_usage_by_key":
    case "get_gateway_usage_by_provider":
    case "get_gateway_usage_by_day":
      return [] as T;
    case "get_gateway_status":
      return {
        is_running: false,
        listen_address: "127.1.0.0",
        port: 3000,
        ssl_enabled: false,
        started_at: null,
        https_port: null,
        force_ssl: false,
      } as T;
    case "get_connected_programs":
      return [] as T;
    case "start_gateway":
    case "stop_gateway":
      return undefined as T;

    case "list_search_providers":
      return getStore("search_providers", []) as T;
    case "create_search_provider": {
      const sps = getStore<SearchProvider[]>("search_providers", []);
      const spInput = (args as { input?: CreateSearchProviderInput }).input
        ?? ({} as CreateSearchProviderInput);
      const sp: SearchProvider = {
        id: genId(),
        name: spInput.name,
        providerType: spInput.providerType,
        endpoint: spInput.endpoint,
        hasApiKey: !!spInput.apiKey,
        enabled: spInput.enabled ?? true,
        resultLimit: spInput.resultLimit ?? 10,
        timeoutMs: spInput.timeoutMs ?? 5000,
      };
      sps.push(sp);
      setStore("search_providers", sps);
      return sp as T;
    }
    case "update_search_provider": {
      const sps2 = getStore<SearchProvider[]>("search_providers", []);
      const spUpdateId = (args as { id?: string }).id;
      const spInput = (args as { input?: Partial<CreateSearchProviderInput> }).input ?? {};
      const spi = sps2.findIndex((s) => s.id === spUpdateId);
      if (spi >= 0) {
        if (spInput.name !== undefined) {
          sps2[spi].name = spInput.name;
        }
        if (spInput.endpoint !== undefined) {
          sps2[spi].endpoint = spInput.endpoint;
        }
        if (spInput.enabled !== undefined) {
          sps2[spi].enabled = spInput.enabled;
        }
        if (spInput.region !== undefined) {
          sps2[spi].region = spInput.region;
        }
        if (spInput.language !== undefined) {
          sps2[spi].language = spInput.language;
        }
        if (spInput.safeSearch !== undefined) {
          sps2[spi].safeSearch = spInput.safeSearch;
        }
        if (spInput.resultLimit !== undefined) {
          sps2[spi].resultLimit = spInput.resultLimit;
        }
        if (spInput.timeoutMs !== undefined) {
          sps2[spi].timeoutMs = spInput.timeoutMs;
        }
        if (spInput.apiKey !== undefined) {
          sps2[spi].hasApiKey = !!spInput.apiKey;
        }
        setStore("search_providers", sps2);
        return sps2[spi] as T;
      }
      return undefined as T;
    }
    case "delete_search_provider": {
      const sps3 = getStore<SearchProvider[]>("search_providers", []);
      setStore(
        "search_providers",
        sps3.filter((s) => s.id !== (args as { id?: string })?.id),
      );
      return undefined as T;
    }
    case "test_search_provider":
      // 字段名与后端 `commands/search.rs::test_search_provider` 的成功体保持一致：
      // 后端返回 `{ ok, latencyMs, resultCount }`（camelCase）。此前 mock 写的是
      // `latency_ms`，而 `SearchProviderSettings` 读 `result.latencyMs`
      // ⇒ 浏览器模式下连通性测试恒显示「undefinedms」。
      return { ok: true, latencyMs: 120, resultCount: 0 } as T;

    // ── Phase 2: MCP Servers ──────────────────────────────────────────
    case "list_local_tools":
      return [
        {
          groupId: "builtin-file-read",
          groupName: i18n.t("browserMock.fileRead"),
          description: i18n.t("browserMock.fileReadDesc"),
          enabled: true,
          tools: [
            {
              name: "FileRead",
              description: i18n.t("browserMock.fileReadDesc"),
              category: "file_read",
              isDestructive: false,
              isReadOnly: true,
              isConcurrencySafe: true,
              enabled: true,
            },
            {
              name: "Glob",
              description: i18n.t("browserMock.globDesc"),
              category: "file_read",
              isDestructive: false,
              isReadOnly: true,
              isConcurrencySafe: true,
              enabled: true,
            },
            {
              name: "Grep",
              description: i18n.t("browserMock.grepDesc"),
              category: "file_read",
              isDestructive: false,
              isReadOnly: true,
              isConcurrencySafe: true,
              enabled: true,
            },
          ],
        },
        {
          groupId: "builtin-file-write",
          groupName: i18n.t("browserMock.fileWrite"),
          description: i18n.t("browserMock.fileWriteDesc"),
          enabled: true,
          tools: [
            {
              name: "FileWrite",
              description: i18n.t("browserMock.fileWriteDesc"),
              category: "file_write",
              isDestructive: true,
              isReadOnly: false,
              isConcurrencySafe: false,
              enabled: true,
            },
            {
              name: "FileEdit",
              description: i18n.t("browserMock.fileEditDesc"),
              category: "file_write",
              isDestructive: true,
              isReadOnly: false,
              isConcurrencySafe: false,
              enabled: false,
            },
            {
              name: "DeleteFile",
              description: i18n.t("browserMock.deleteFileDesc"),
              category: "file_write",
              isDestructive: true,
              isReadOnly: false,
              isConcurrencySafe: false,
              enabled: true,
            },
          ],
        },
        {
          groupId: "builtin-shell",
          groupName: i18n.t("browserMock.shellCommand"),
          description: i18n.t("browserMock.shellCommandDesc"),
          enabled: false,
          tools: [
            {
              name: "Bash",
              description: i18n.t("browserMock.bashDesc"),
              category: "shell",
              isDestructive: true,
              isReadOnly: false,
              isConcurrencySafe: false,
              enabled: true,
            },
          ],
        },
      ] as T;
    case "toggle_local_tool_group":
      return {
        groupId: (args as Record<string, unknown>)?.groupId,
        groupName: "",
        description: "",
        enabled: true,
        tools: [],
      } as T;
    case "toggle_single_tool":
      return [] as T;
    case "list_mcp_servers":
      return getStore("mcp_servers", []) as T;
    case "create_mcp_server": {
      const mcps = getStore<Record<string, unknown>[]>("mcp_servers", []);
      const mcp = {
        id: genId(),
        ...(args as Record<string, unknown>),
        status: "disconnected",
        created_at: nowTs(),
        updated_at: nowTs(),
      };
      mcps.push(mcp);
      setStore("mcp_servers", mcps);
      return mcp as T;
    }
    case "update_mcp_server": {
      const mcps2 = getStore<Record<string, unknown>[]>("mcp_servers", []);
      const mi = mcps2.findIndex(
        (m) => m.id === (args as Record<string, unknown>)?.id,
      );
      if (mi >= 0) {
        Object.assign(mcps2[mi], args, { updated_at: nowTs() });
        setStore("mcp_servers", mcps2);
        return mcps2[mi] as T;
      }
      return undefined as T;
    }
    case "delete_mcp_server": {
      const mcps3 = getStore<Record<string, unknown>[]>("mcp_servers", []);
      setStore(
        "mcp_servers",
        mcps3.filter((m) => m.id !== (args as Record<string, unknown>)?.id),
      );
      return undefined as T;
    }
    case "list_mcp_tools":
      return [
        { name: "web_search", description: "Search the web", parameters: {} },
        {
          name: "calculator",
          description: "Evaluate math expressions",
          parameters: {},
        },
      ] as T;
    case "test_mcp_server":
      return { ok: true, error: undefined } as T;
    case "list_tool_executions":
      return [] as T;

    // ── Phase 2: Knowledge Base ───────────────────────────────────────
    case "list_knowledge_bases":
      return getStore<KnowledgeBase[]>("knowledge_bases", []) as T;
    case "create_knowledge_base": {
      const input = (args as { input?: CreateKnowledgeBaseInput }).input
        ?? ({} as CreateKnowledgeBaseInput);
      const kbs = getStore<KnowledgeBase[]>("knowledge_bases", []);
      const kb: KnowledgeBase & {
        documents: KnowledgeDocument[];
        created_at: number;
        updated_at: number;
      } = {
        id: genId(),
        name: input.name,
        description: input.description,
        embeddingProvider: input.embeddingProvider,
        enabled: input.enabled ?? true,
        sortOrder: kbs.length,
        documents: [],
        created_at: nowTs(),
        updated_at: nowTs(),
      };
      kbs.push(kb);
      setStore("knowledge_bases", kbs);
      return kb as T;
    }
    case "update_knowledge_base": {
      const kbs2 = getStore<KnowledgeBase[]>("knowledge_bases", []);
      const { id, input } = args as {
        id: string;
        input?: Partial<KnowledgeBase>;
      };
      const ki = kbs2.findIndex((k) => k.id === id);
      if (ki >= 0) {
        if (input?.name !== undefined) {
          kbs2[ki].name = input.name;
        }
        if (input?.description !== undefined) {
          kbs2[ki].description = input.description;
        }
        if (input?.enabled !== undefined) {
          kbs2[ki].enabled = input.enabled;
        }
        setStore("knowledge_bases", kbs2);
        return kbs2[ki] as T;
      }
      return undefined as T;
    }
    case "delete_knowledge_base": {
      const kbs3 = getStore<KnowledgeBase[]>("knowledge_bases", []);
      setStore(
        "knowledge_bases",
        kbs3.filter((k) => k.id !== (args as { id?: string })?.id),
      );
      return undefined as T;
    }
    case "add_knowledge_document": {
      const kbs4 = getStore<
        (KnowledgeBase & {
          documents: KnowledgeDocument[];
          updated_at: number;
        })[]
      >("knowledge_bases", []);
      const { baseId, ...docInput } = args as {
        baseId?: string;
        title?: string;
        sourcePath?: string;
      };
      const kbi = kbs4.findIndex((k) => k.id === baseId);
      if (kbi >= 0) {
        const doc: KnowledgeDocument = {
          id: genId(),
          knowledgeBaseId: baseId!,
          title: docInput.title ?? "Untitled",
          sourcePath: docInput.sourcePath ?? "",
          mimeType: "text/plain",
          sizeBytes: 0,
          indexingStatus: "pending",
          docType: "document",
          contentHash: "",
        };
        kbs4[kbi].documents = [...(kbs4[kbi].documents || []), doc];
        kbs4[kbi].updated_at = nowTs();
        setStore("knowledge_bases", kbs4);
        return doc as T;
      }
      return undefined as T;
    }
    case "list_knowledge_documents": {
      const kbs5 = getStore<
        (KnowledgeBase & {
          documents: KnowledgeDocument[];
          updated_at: number;
        })[]
      >("knowledge_bases", []);
      const target = kbs5.find(
        (k) => k.id === (args as { base_id?: string })?.base_id,
      );
      return (target?.documents ?? []) as T;
    }
    case "delete_knowledge_document": {
      const kbs6 = getStore<
        (KnowledgeBase & {
          documents: KnowledgeDocument[];
          updated_at: number;
        })[]
      >("knowledge_bases", []);
      const delDocId = (args as { id?: string })?.id;
      for (const kb of kbs6) {
        const docs = kb.documents || [];
        const filtered = docs.filter((d) => d.id !== delDocId);
        if (filtered.length !== docs.length) {
          kb.documents = filtered;
          kb.updated_at = nowTs();
          break;
        }
      }
      setStore("knowledge_bases", kbs6);
      return undefined as T;
    }
    case "search_knowledge_base":
      return [] as T;
    case "rebuild_knowledge_index":
    case "clear_knowledge_index":
      return undefined as T;
    case "extract_entities_for_kb":
    case "extract_entities_from_documents":
      return {
        status: "success",
        documentCount: 0,
        taskCount: 0,
        message: "实体抽取仅在桌面模式下可用（浏览器模式 mock）",
      } as T;
    case "graph_enhanced_search":
      return {
        query: args?.query ?? "",
        entities: [],
        relations: [],
        chunks: [],
      } as T;

    // ── Phase 2: Memory ───────────────────────────────────────────────
    case "list_memory_namespaces":
      return getStore<MemoryNamespace[]>("memory_namespaces", []) as T;
    case "create_memory_namespace": {
      const input = (args as { input?: CreateMemoryNamespaceInput }).input
        ?? ({} as CreateMemoryNamespaceInput);
      const mns = getStore<MemoryNamespace[]>("memory_namespaces", []);
      const mn: MemoryNamespace & {
        items: MemoryItem[];
        created_at: number;
        updated_at: number;
      } = {
        id: genId(),
        name: input.name,
        scope: input.scope ?? "global",
        embeddingProvider: input.embeddingProvider,
        sortOrder: mns.length,
        items: [],
        created_at: nowTs(),
        updated_at: nowTs(),
      };
      mns.push(mn);
      setStore("memory_namespaces", mns);
      return mn as T;
    }
    case "delete_memory_namespace": {
      const mns2 = getStore<MemoryNamespace[]>("memory_namespaces", []);
      setStore(
        "memory_namespaces",
        mns2.filter((n) => n.id !== (args as { id?: string })?.id),
      );
      return undefined as T;
    }
    case "add_memory_item": {
      const mns3 = getStore<
        (MemoryNamespace & { items: MemoryItem[]; updated_at: number })[]
      >("memory_namespaces", []);
      const inputMem = (args as { input?: CreateMemoryItemInput }).input
        ?? ({} as CreateMemoryItemInput);
      const mni = mns3.findIndex((n) => n.id === inputMem?.namespaceId);
      if (mni >= 0) {
        const item: MemoryItem = {
          id: genId(),
          namespaceId: inputMem.namespaceId!,
          title: inputMem.title ?? "",
          content: inputMem.content ?? "",
          source: inputMem.source ?? "manual",
          indexStatus: "pending",
          tier: "working",
          importance: 0.5,
          memoryNature: "semantic",
          tags: [],
          accessCount: 0,
          decayRate: 0.02,
          updatedAt: new Date().toISOString(),
          applicabilityTags: inputMem.applicabilityTags ?? [],
          confirmed: inputMem.confirmed ?? 0,
        };
        mns3[mni].items = [...(mns3[mni].items || []), item];
        mns3[mni].updated_at = nowTs();
        setStore("memory_namespaces", mns3);
        return item as T;
      }
      return undefined as T;
    }
    case "list_memory_items": {
      const mns4 = getStore<(MemoryNamespace & { items: MemoryItem[] })[]>(
        "memory_namespaces",
        [],
      );
      const ns = mns4.find(
        (n) => n.id === (args as { namespaceId?: string })?.namespaceId,
      );
      return (ns?.items ?? []) as T;
    }
    case "delete_memory_item": {
      const mns5 = getStore<
        (MemoryNamespace & { items: MemoryItem[]; updated_at: number })[]
      >("memory_namespaces", []);
      const delItemId = (args as { id?: string })?.id;
      for (const mns of mns5) {
        const items = mns.items || [];
        const filtered = items.filter((i) => i.id !== delItemId);
        if (filtered.length !== items.length) {
          mns.items = filtered;
          mns.updated_at = nowTs();
          break;
        }
      }
      setStore("memory_namespaces", mns5);
      return undefined as T;
    }
    case "search_memory":
      return [] as T;
    case "rebuild_memory_index":
    case "clear_memory_index":
      return undefined as T;
    case "list_knowledge_graph":
      // 构造示例知识图谱数据，供浏览器预览图谱视图渲染
      return {
        entities: [
          {
            id: "ent_1",
            name: "AxInvest Project",
            entity_type: "project",
            properties: {},
            aliases: ["AxInvest", "axinvest"],
            mention_count: 12,
            confidence: 0.95,
          },
          {
            id: "ent_2",
            name: "Knowledge Graph View",
            entity_type: "concept",
            properties: {},
            aliases: ["graph"],
            mention_count: 8,
            confidence: 0.9,
          },
          {
            id: "ent_3",
            name: "Tauri v2",
            entity_type: "concept",
            properties: {},
            aliases: [],
            mention_count: 5,
            confidence: 0.88,
          },
          {
            id: "ent_4",
            name: "Memory Module",
            entity_type: "file",
            properties: {},
            aliases: [],
            mention_count: 6,
            confidence: 0.8,
          },
        ],
        relationships: [
          { id: "rel_1", source_id: "ent_1", target_id: "ent_2", relation_type: "implements", weight: 1 },
          { id: "rel_2", source_id: "ent_1", target_id: "ent_3", relation_type: "depends_on", weight: 1 },
          { id: "rel_3", source_id: "ent_1", target_id: "ent_4", relation_type: "contains", weight: 1 },
          { id: "rel_4", source_id: "ent_4", target_id: "ent_2", relation_type: "related_to", weight: 1 },
        ],
      } as T;

    // ── Fleet (办公室) ─────────────────────────────────────────────────
    case "fleet_list": {
      return getStore<Fleet[]>("fleets", []) as T;
    }
    case "fleet_get": {
      const fleetId = (args as { fleet_id?: string }).fleet_id ?? "";
      const fleet = getStore<Fleet[]>("fleets", []).find((f) => f.id === fleetId);
      return (fleet ?? null) as T;
    }
    case "fleet_create": {
      const input = (args as { input?: Partial<Fleet> }).input ?? {};
      const fleets = getStore<Fleet[]>("fleets", []);
      const fleet: Fleet = {
        id: genId(),
        name: (input as Record<string, unknown>).name as string ?? "New Fleet",
        sceneTemplateSlug: (input as Record<string, unknown>).scene_template_slug as string ?? "default",
        status: "active",
        createdAt: nowTs(),
        updatedAt: nowTs(),
        metadata: {},
      };
      fleets.push(fleet);
      setStore("fleets", fleets);
      return fleet as T;
    }
    case "fleet_update_status": {
      const { fleet_id: fleetId, status } = args as { fleet_id?: string; status?: string };
      if (fleetId && status) {
        const fleets = getStore<Fleet[]>("fleets", []);
        setStore(
          "fleets",
          fleets.map((f) => (f.id === fleetId ? { ...f, status, updatedAt: nowTs() } : f)),
        );
      }
      return undefined as T;
    }
    case "fleet_delete": {
      const fleetId = (args as { fleet_id?: string }).fleet_id ?? "";
      setStore("fleets", getStore<Fleet[]>("fleets", []).filter((f) => f.id !== fleetId));
      // 级联删除成员缓存
      for (let i = localStorage.length - 1; i >= 0; i--) {
        const key = localStorage.key(i);
        if (key && key.startsWith(`axagent_fleet_members:${fleetId}`)) {
          localStorage.removeItem(key);
        }
      }
      return undefined as T;
    }
    case "fleet_reset_daily_tokens": {
      const fleetId = (args as { fleet_id?: string }).fleet_id ?? "";
      const members = getStore<FleetMember[]>(`fleet_members:${fleetId}`, []);
      setStore(
        `fleet_members:${fleetId}`,
        members.map((m) => ({ ...m, todayTokens: 0 })),
      );
      return undefined as T;
    }
    case "fleet_list_members": {
      const fleetId = (args as { fleet_id?: string }).fleet_id ?? "";
      return getStore<FleetMember[]>(`fleet_members:${fleetId}`, []) as T;
    }
    case "fleet_list_messages": {
      // 键名兼容（同 dispatch）：snake_case 是 mock 既有约定，camelCase 也应可用
      const a = args as Record<string, unknown>;
      const fleetId = (a.fleetId ?? a.fleet_id ?? "") as string;
      // 会话缺省为群聊（与后端 `fleet_list_messages` 的 `conversation_id` 缺省一致）
      const conversationId = (a.conversationId ?? a.conversation_id ?? "group") as string;
      const afterSeq = (a.afterSeq ?? a.after_seq) as number | undefined;
      const all = getStore<FleetMessage[]>(`fleet_messages:${fleetId}:${conversationId}`, []);
      const filtered = afterSeq == null ? all : all.filter((m) => m.seq > afterSeq);
      return [...filtered].sort((x, y) => x.seq - y.seq) as T;
    }
    case "fleet_get_member": {
      const memberId = (args as { member_id?: string }).member_id ?? "";
      for (let i = localStorage.length - 1; i >= 0; i--) {
        const key = localStorage.key(i);
        if (!key || !key.startsWith("axagent_fleet_members:")) {
          continue;
        }
        const found = getStore<FleetMember[]>(key.replace("axagent_", ""), [])
          .find((m) => m.id === memberId);
        if (found) {
          return found as T;
        }
      }
      return null as T;
    }
    case "fleet_add_member": {
      const input = (args as { input?: Partial<FleetMember> }).input ?? {};
      const fleetId = (input as Record<string, unknown>).fleet_id as string ?? "";
      const members = getStore<FleetMember[]>(`fleet_members:${fleetId}`, []);
      // 与后端一致：同舰队内 slug 必须唯一（路由与事件回写的键）
      const slug = ((input as Record<string, unknown>).agent_slug as string ?? "assistant").trim();
      if (members.some((m) => m.agentSlug === slug)) {
        throw new Error(
          JSON.stringify({
            code: "FLEET_SLUG_EXISTS",
            params: { slug },
          }),
        );
      }
      const member: FleetMember = {
        id: genId(),
        fleetId,
        agentId: (input as Record<string, unknown>).agent_id as string ?? genId(),
        agentSlug: slug,
        displayName: (input as Record<string, unknown>).display_name as string ?? "Assistant",
        role: (input as Record<string, unknown>).role as string ?? "",
        agentProfileId: (input as Record<string, unknown>).agent_profile_id as string | undefined,
        roomId: (input as Record<string, unknown>).room_id as string ?? "workspace",
        status: "idle",
        joinedAt: nowTs(),
        todayTokens: 0,
        totalTokens: 0,
      };
      members.push(member);
      setStore(`fleet_members:${fleetId}`, members);
      return member as T;
    }
    case "fleet_remove_member": {
      const { member_id: memberId } = args as { member_id?: string };
      if (memberId) {
        // 遍历所有 fleet_members 存储，移除对应成员
        for (let i = localStorage.length - 1; i >= 0; i--) {
          const key = localStorage.key(i);
          if (!key || !key.startsWith("axagent_fleet_members:")) {
            continue;
          }
          const members = getStore<FleetMember[]>(key.replace("axagent_", ""), []);
          const next = members.filter((m) => m.id !== memberId);
          setStore(key.replace("axagent_", ""), next);
        }
      }
      return undefined as T;
    }
    case "fleet_update_member_status": {
      const { member_id: memberId, status } = args as { member_id?: string; status?: string };
      if (memberId && status) {
        for (let i = localStorage.length - 1; i >= 0; i--) {
          const key = localStorage.key(i);
          if (!key || !key.startsWith("axagent_fleet_members:")) {
            continue;
          }
          const members = getStore<FleetMember[]>(key.replace("axagent_", ""), []);
          const next = members.map((m) => (m.id === memberId ? { ...m, status } : m));
          setStore(key.replace("axagent_", ""), next);
        }
      }
      return undefined as T;
    }
    case "fleet_dispatch":
    case "fleet_direct_message": {
      const input = (args as { input?: { fleet_id?: string; user_message?: string; agent_slug?: string } })
        .input ?? {};
      const onEvent = (args as { on_event?: MockChannel }).on_event;
      const inputRaw = input as Record<string, unknown>;
      // 键名兼容：mock 的既有约定是 snake_case 入参，但直接以 camelCase 调用也应可用
      const fleetId = (inputRaw.fleetId ?? inputRaw.fleet_id ?? "") as string;
      const members = getStore<FleetMember[]>(`fleet_members:${fleetId}`, []);
      const push = (evt: unknown) => onEvent?.onmessage?.(evt);
      if (members.length === 0) {
        push({ type: "error", message: i18n.t("browserMock.fleetNoMembers") });
        return undefined as T;
      }
      // 直接 DM 时定位目标成员，否则取第一个
      const targetAgentSlug = (inputRaw.agentSlug ?? inputRaw.agent_slug) as string | undefined;
      const target = targetAgentSlug
        ? members.find((m) => m.agentSlug === targetAgentSlug) ?? members[0]
        : members[0];
      const msg = (inputRaw.userMessage ?? inputRaw.user_message ?? "") as string;

      // ── 消息持久化（localStorage 模拟后端 fleet_messages 表）──
      // 浏览器模式必须与真实后端同构：用户消息与 agent 回复都要进同一份历史，
      // 否则 ChatPanel 的「以库为真源」在浏览器模式下会恒为空。
      //
      // 会话隔离同样必须同构：群聊落 `"group"`，私信落 `"dm:<slug>"`。
      // 键里带上 conversationId —— 只按 fleetId 存会让 DM 与群聊共用一个数组。
      const isDirectMessage = cmd === "fleet_direct_message";
      const conversationId = isDirectMessage
        ? `dm:${target.agentSlug}`
        : "group";
      const msgKey = `fleet_messages:${fleetId}:${conversationId}`;
      const storedMsgs = getStore<FleetMessage[]>(msgKey, []);
      let nextSeq = storedMsgs.reduce((max, m) => Math.max(max, m.seq), 0);
      const appendMsg = (
        partial: Omit<FleetMessage, "id" | "fleetId" | "conversationId" | "seq" | "createdAt">,
      ) => {
        nextSeq += 1;
        storedMsgs.push({
          id: genId(),
          fleetId,
          conversationId,
          seq: nextSeq,
          createdAt: Date.now(),
          ...partial,
        });
        setStore(msgKey, storedMsgs);
      };

      appendMsg({
        authorKind: "human",
        authorId: "local-user",
        content: msg,
      });

      push({
        type: "routing",
        agentSlug: target.agentSlug,
        agentId: target.agentId,
        roomId: target.roomId,
        taskSummary: msg,
      });
      push({ type: "agent_status", agentSlug: target.agentSlug, agentId: target.agentId, status: "busy" });
      const agentReply = i18n.t("browserMock.fleetMessageReceived", {
        name: target.displayName,
        message: msg,
      });
      appendMsg({
        authorKind: "agent",
        authorId: target.agentId,
        authorSlug: target.agentSlug,
        authorDisplayName: target.displayName,
        content: agentReply,
      });
      push({
        type: "agent_message",
        agentSlug: target.agentSlug,
        agentId: target.agentId,
        content: agentReply,
      });
      push({
        type: "token_usage",
        agentSlug: target.agentSlug,
        agentId: target.agentId,
        inputTokens: 12,
        outputTokens: 34,
      });
      push({ type: "agent_status", agentSlug: target.agentSlug, agentId: target.agentId, status: "idle" });
      push({ type: "complete" });
      return undefined as T;
    }

    // ── Phase 2: Artifacts ────────────────────────────────────────────
    case "list_artifacts": {
      const allArtifacts = getStore<Artifact[]>("artifacts", []);
      const convId = (args as { conversationId?: string })?.conversationId;
      return (
        convId
          ? allArtifacts.filter((a) => a.conversationId === convId)
          : allArtifacts
      ) as T;
    }
    case "create_artifact": {
      const input = (args as { input?: Partial<Artifact> }).input ?? {};
      const arts = getStore<Artifact[]>("artifacts", []);
      const art: Artifact = {
        id: genId(),
        conversationId: input.conversationId ?? "",
        title: input.title ?? "Untitled",
        content: input.content ?? "",
        kind: input.kind ?? "note",
        format: input.format ?? "text",
        language: input.language,
        previewMode: input.previewMode,
        metadata: input.metadata,
        pinned: false,
        updatedAt: new Date().toISOString(),
      };
      arts.push(art);
      setStore("artifacts", arts);
      return art as T;
    }
    case "update_artifact": {
      const arts2 = getStore<Artifact[]>("artifacts", []);
      const artInput = (args as { id?: string; input?: Partial<Artifact> })
        .input;
      const ai = arts2.findIndex((a) => a.id === (args as { id?: string }).id);
      if (ai >= 0 && artInput) {
        if (artInput.title !== undefined) {
          arts2[ai].title = artInput.title;
        }
        if (artInput.content !== undefined) {
          arts2[ai].content = artInput.content;
        }
        if (artInput.format !== undefined) {
          arts2[ai].format = artInput.format;
        }
        if (artInput.language !== undefined) {
          arts2[ai].language = artInput.language;
        }
        if (artInput.previewMode !== undefined) {
          arts2[ai].previewMode = artInput.previewMode;
        }
        if (artInput.pinned !== undefined) {
          arts2[ai].pinned = artInput.pinned;
        }
        arts2[ai].updatedAt = new Date().toISOString();
        setStore("artifacts", arts2);
        return arts2[ai] as T;
      }
      return undefined as T;
    }
    case "delete_artifact": {
      const arts3 = getStore<Artifact[]>("artifacts", []);
      setStore(
        "artifacts",
        arts3.filter((a) => a.id !== (args as { id?: string })?.id),
      );
      return undefined as T;
    }

    // ── Phase 2: Conversation Branching ───────────────────────────────
    case "fork_conversation": {
      const convs = getStore<Record<string, unknown>[]>("conversations", []);
      const source = convs.find(
        (c) => c.id === (args as Record<string, unknown>)?.conversationId,
      );
      if (source) {
        const forked = {
          ...JSON.parse(JSON.stringify(source)),
          id: genId(),
          parent_id: source.id,
          title: (args as Record<string, unknown>)?.title
            ?? `Fork of ${source.title}`,
          created_at: nowTs(),
          updated_at: nowTs(),
        };
        convs.push(forked);
        setStore("conversations", convs);
        return forked as T;
      }
      return undefined as T;
    }
    case "list_branches": {
      const convs2 = getStore<Record<string, unknown>[]>("conversations", []);
      const parentId = (args as Record<string, unknown>)?.conversationId;
      return convs2.filter(
        (c) => c.parent_id === parentId || c.id === parentId,
      ) as T;
    }
    case "compare_branches": {
      const brA = (args as Record<string, unknown>)?.branchA;
      const brB = (args as Record<string, unknown>)?.branchB;
      return { branch_a: brA, branch_b: brB, differences: [] } as T;
    }

    // ── Phase 2: Context Sources ──────────────────────────────────────
    case "list_context_sources":
      return getStore("context_sources", []) as T;
    case "add_context_source": {
      const css = getStore<Record<string, unknown>[]>("context_sources", []);
      const cs = {
        id: genId(),
        ...(args as Record<string, unknown>),
        enabled: true,
        created_at: nowTs(),
        updated_at: nowTs(),
      };
      css.push(cs);
      setStore("context_sources", css);
      return cs as T;
    }
    case "remove_context_source": {
      const css2 = getStore<Record<string, unknown>[]>("context_sources", []);
      setStore(
        "context_sources",
        css2.filter((c) => c.id !== (args as Record<string, unknown>)?.id),
      );
      return undefined as T;
    }
    case "toggle_context_source": {
      const css3 = getStore<Record<string, unknown>[]>("context_sources", []);
      const csi = css3.findIndex(
        (c) => c.id === (args as Record<string, unknown>)?.id,
      );
      if (csi >= 0) {
        css3[csi].enabled = !css3[csi].enabled;
        css3[csi].updated_at = nowTs();
        setStore("context_sources", css3);
        return css3[csi] as T;
      }
      return undefined as T;
    }

    // ── Phase 2: Backup ──────────────────────────────────────────────
    case "create_backup": {
      const bkps = getStore<Record<string, unknown>[]>("backups", []);
      const bkp = {
        id: genId(),
        version: (args as Record<string, unknown>)?.format || "json",
        createdAt: new Date().toISOString(),
        encrypted: false,
        checksum: "mock-checksum",
        objectCountsJson: "{}",
        sourceAppVersion: "0.1.0",
        filePath: "/mock/path/axagent-backup.json",
        fileSize: 1024,
      };
      bkps.push(bkp);
      setStore("backups", bkps);
      return bkp as T;
    }
    case "list_backups":
      return getStore<BackupManifest[]>("backups", []) as T;
    case "delete_backup": {
      const backups = getStore<BackupManifest[]>("backups", []);
      const bkpId = (args as { backup_id?: string })?.backup_id;
      setStore(
        "backups",
        backups.filter((b) => b.id !== bkpId),
      );
      return undefined as T;
    }
    case "batch_delete_backups": {
      const allBkps = getStore<BackupManifest[]>("backups", []);
      const idsToDelete = (args as { backup_ids?: string[] })?.backup_ids || [];
      setStore(
        "backups",
        allBkps.filter((b) => !idsToDelete.includes(b.id)),
      );
      return undefined as T;
    }
    case "restore_backup":
      return undefined as T;
    case "get_backup_settings":
      return {
        enabled: false,
        intervalHours: 24,
        maxCount: 10,
        backupDir: "/mock/backups",
      } as T;
    case "update_backup_settings":
      return undefined as T;

    // ── Files Page ─────────────────────────────────────────────────────
    case "list_files_page_entries": {
      const category = (args as { category?: string })?.category;
      if (category === "backups") {
        const backups = getStore<BackupManifest[]>("backups", []);
        return backups.map((backup) => ({
          id: `backup_manifest::${backup.id}`,
          name: backup.filePath?.split("/").pop()
            || `backup-${backup.createdAt}.${backup.version}`,
          path: backup.filePath || "",
          size: backup.fileSize,
          createdAt: backup.createdAt,
          category: "backups",
          hasThumbnail: false,
          missing: !backup.filePath,
        })) as T;
      }
      return [] as T;
    }
    case "open_files_page_entry":
    case "reveal_files_page_entry":
      return undefined as T;
    case "cleanup_missing_files_page_entry": {
      const entryId = (args as { entry_id?: string })?.entry_id;
      if (entryId?.startsWith("backup_manifest::")) {
        const backupId = entryId.slice("backup_manifest::".length);
        const backups = getStore<BackupManifest[]>("backups", []);
        setStore(
          "backups",
          backups.filter((b) => b.id !== backupId),
        );
      }
      return undefined as T;
    }

    case "get_program_policies":
      return getStore<ProgramPolicy[]>("program_policies", []) as T;
    case "save_program_policy": {
      const sppList = getStore<ProgramPolicy[]>("program_policies", []);
      const sppInput = (args as { input?: SaveProgramPolicyInput }).input
        ?? ({} as SaveProgramPolicyInput);
      const sppIdx = sppList.findIndex(
        (p) => p.programName === sppInput.programName,
      );
      if (sppIdx >= 0) {
        sppList[sppIdx] = {
          ...sppList[sppIdx],
          allowedProviderIdsJson: JSON.stringify(
            sppInput.allowedProviderIds ?? [],
          ),
          allowedModelIdsJson: JSON.stringify(sppInput.allowedModelIds ?? []),
          defaultProviderId: sppInput.defaultProviderId,
          defaultModelId: sppInput.defaultModelId,
          rateLimitPerMinute: sppInput.rateLimitPerMinute,
        };
        setStore("program_policies", sppList);
        return sppList[sppIdx] as T;
      }
      const sppNew: ProgramPolicy = {
        id: genId(),
        programName: sppInput.programName,
        allowedProviderIdsJson: JSON.stringify(
          sppInput.allowedProviderIds ?? [],
        ),
        allowedModelIdsJson: JSON.stringify(sppInput.allowedModelIds ?? []),
        defaultProviderId: sppInput.defaultProviderId,
        defaultModelId: sppInput.defaultModelId,
        rateLimitPerMinute: sppInput.rateLimitPerMinute,
      };
      sppList.push(sppNew);
      setStore("program_policies", sppList);
      return sppNew as T;
    }
    case "delete_program_policy": {
      const pps3 = getStore<ProgramPolicy[]>("program_policies", []);
      setStore(
        "program_policies",
        pps3.filter((p) => p.id !== (args as { id?: string })?.id),
      );
      return undefined as T;
    }

    // ── Phase 2: Gateway Diagnostics & Templates ──────────────────────
    case "get_gateway_diagnostics":
      return [
        {
          id: "1",
          category: "port",
          status: "ok",
          message: "Gateway port is available",
          createdAt: nowTs(),
        },
        {
          id: "2",
          category: "auth",
          status: "ok",
          message: "Authentication configured",
          createdAt: nowTs(),
        },
        {
          id: "3",
          category: "proxy",
          status: "ok",
          message: "Proxy settings valid",
          createdAt: nowTs(),
        },
        {
          id: "4",
          category: "provider_latency",
          status: "warning",
          message: "No providers configured",
          createdAt: nowTs(),
        },
      ] as T;
    case "list_gateway_templates":
      return getStore("gateway_templates", [
        {
          id: "tpl-cursor",
          name: "Cursor IDE",
          target: "cursor",
          format: "json",
          content: '{\n  "openai.apiKey": "{{key}}",\n  "openai.apiBaseUrl": "http://localhost:{{port}}/v1"\n}',
          copyHint: i18n.t("browserMock.copyHintCursor"),
          created_at: nowTs(),
          updated_at: nowTs(),
        },
        {
          id: "tpl-vscode",
          name: "VS Code Continue",
          target: "vscode",
          format: "json",
          content:
            '{\n  "models": [{\n    "provider": "openai",\n    "apiBase": "http://localhost:{{port}}/v1",\n    "apiKey": "{{key}}"\n  }]\n}',
          copyHint: i18n.t("browserMock.copyHintContinue"),
          created_at: nowTs(),
          updated_at: nowTs(),
        },
        {
          id: "tpl-claude",
          name: "Claude Code CLI",
          target: "claude_code",
          format: "text",
          content: "ANTHROPIC_BASE_URL=http://localhost:{{port}}/v1\nANTHROPIC_AUTH_TOKEN={{key}}",
          copyHint: i18n.t("browserMock.copyHintEnv"),
          created_at: nowTs(),
          updated_at: nowTs(),
        },
        {
          id: "tpl-openai",
          name: "OpenAI Compatible",
          target: "openai_compatible",
          format: "text",
          content: "API Base: http://localhost:{{port}}/v1\nAPI Key: {{key}}",
          copyHint: i18n.t("browserMock.copyHintOpenAI"),
          created_at: nowTs(),
          updated_at: nowTs(),
        },
      ]) as T;
    case "copy_gateway_template": {
      const cgtList = getStore<Record<string, unknown>[]>(
        "gateway_templates",
        [],
      );
      const cgtMatch = cgtList.find(
        (t) => t.id === (args as Record<string, unknown>)?.templateId,
      );
      return (cgtMatch?.content
        ?? "# Gateway Template Configuration\n\nNo template found.") as T;
    }
    case "get_desktop_capabilities":
      return [
        { key: "tray", supported: false },
        { key: "global_shortcut", supported: true },
        { key: "protocol_handler", supported: false },
        { key: "mini_window", supported: false },
        { key: "notification", supported: "Notification" in globalThis },
      ] as T;
    case "get_window_state":
      return {
        width: globalThis.innerWidth ?? 1280,
        height: globalThis.innerHeight ?? 800,
        focused: true,
        fullscreen: false,
      } as T;
    case "send_desktop_notification": {
      if (
        typeof Notification !== "undefined"
        && Notification.permission === "granted"
      ) {
        new Notification((args as { title?: string })?.title ?? "AxInvest", {
          body: (args as { body?: string })?.body ?? "",
        });
      }
      return undefined as T;
    }
    case "set_always_on_top":
      return undefined as T;
    case "set_close_to_tray":
      return undefined as T;
    case "apply_startup_settings":
      return undefined as T;
    case "get_workspace_snapshot": {
      const convId = (args as Record<string, unknown>)?.conversationId as string;
      const branches = getStore<ConversationBranch[]>(`branches_${convId}`, []);
      return {
        searchPolicy: { enabled: false, queryMode: "manual", resultLimit: 10 },
        toolBinding: { serverIds: [], approvalMode: "inherit" },
        knowledgeBinding: { knowledgeBaseIds: [], autoAttach: false },
        memoryPolicy: { enabled: false, writeBack: false },
        toggles: {
          searchEnabled: false,
          enabledKnowledgeBaseIds: [],
          enabledMcpServerIds: [],
          enabledWikiIds: [],
          memoryEnabled: false,
          memoryWriteBack: false,
        },
        researchMode: false,
        pinnedArtifactIds: [],
        branches,
        activeBranchId: null,
      } as T;
    }
    case "update_workspace_snapshot":
      return undefined as T;

    // ── Proxy Test ────────────────────────────────────────────────────────
    case "test_proxy": {
      const addr = (args as Record<string, unknown>)?.proxyAddress;
      if (!addr) {
        return { ok: false, error: "No address" } as T;
      }
      await new Promise((r) => setTimeout(r, 500));
      return {
        ok: true,
        latency_ms: 120 + Math.floor(Math.random() * 200),
      } as T;
    }

    // ── Skills ────────────────────────────────────────────────────────
    case "list_skills":
      return [
        {
          name: "superpowers:brainstorming",
          description: "You MUST use this before any creative work",
          author: "AxInvest",
          version: "1.0.0",
          source: "builtin",
          sourcePath: "builtin://superpowers-brainstorming",
          enabled: true,
          hasUpdate: false,
          userInvocable: true,
          argumentHint: null,
          whenToUse: null,
          group: "superpowers",
          frontend: null,
        },
        {
          name: "superpowers:systematic-debugging",
          description: "Use when encountering any bug, test failure, or unexpected behavior",
          author: "AxInvest",
          version: "1.0.0",
          source: "builtin",
          sourcePath: "builtin://superpowers-debugging",
          enabled: true,
          hasUpdate: false,
          userInvocable: true,
          argumentHint: null,
          whenToUse: null,
          group: "superpowers",
          frontend: null,
        },
        {
          name: "superpowers:writing-plans",
          description: "Use when you have a spec or requirements for a multi-step task",
          author: "AxInvest",
          version: "1.0.0",
          source: "builtin",
          sourcePath: "builtin://superpowers-writing-plans",
          enabled: true,
          hasUpdate: false,
          userInvocable: true,
          argumentHint: null,
          whenToUse: null,
          group: "superpowers",
          frontend: null,
        },
        {
          name: "superpowers:test-driven-development",
          description: "Use when implementing any feature or bugfix, before writing implementation code",
          author: "AxInvest",
          version: "1.0.0",
          source: "builtin",
          sourcePath: "builtin://superpowers-tdd",
          enabled: true,
          hasUpdate: false,
          userInvocable: true,
          argumentHint: null,
          whenToUse: null,
          group: "superpowers",
          frontend: null,
        },
      ] as T;

    case "get_skill":
      return {
        info: {
          name: (args as Record<string, unknown>)?.name || "example",
          description: "Example skill",
          source: "axagent",
          sourcePath: "/mock/path",
          enabled: true,
          hasUpdate: false,
          userInvocable: true,
        },
        content: "# Example Skill\n\nThis is a mock skill for browser preview.",
        files: ["SKILL.md"],
        manifest: null,
      } as T;

    case "toggle_skill":
      return undefined as T;

    case "install_skill":
      return ((args as Record<string, unknown>)?.source
        || "installed-skill") as T;

    case "uninstall_skill":
      return undefined as T;

    case "uninstall_skill_group":
      return undefined as T;

    case "open_skills_dir":
      return undefined as T;

    case "open_skill_dir":
      return undefined as T;

    case "search_marketplace":
      return [] as T;

    case "check_skill_updates":
      return [] as T;

    case "get_webdav_sync_status":
      return { status: "disabled", lastSync: null, error: null } as T;

    case "restart_webdav_sync":
      return undefined as T;

    // ── Workflow Templates ────────────────────────────────────────────
    case "seed_preset_templates": {
      const existingTemplates = getStore<Record<string, unknown>[]>(
        "workflow_templates",
        [],
      );
      if (existingTemplates.length > 0) {
        return existingTemplates.length as T;
      }
      const presetTemplates = [
        {
          id: "docs",
          name: "Documentation",
          description: "Generate comprehensive documentation",
          icon: "BookOpen",
          tags: ["docs", "api", "readme"],
          version: 1,
          isPreset: true,
          isEditable: false,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "test-gen",
          name: "Test Generation",
          description: "Generate comprehensive test suites",
          icon: "TestTube",
          tags: ["testing", "tdd", "coverage"],
          version: 1,
          isPreset: true,
          isEditable: false,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "refactor",
          name: "Code Refactor",
          description: "Systematic code refactoring with behavior preservation",
          icon: "GitBranch",
          tags: ["refactor", "clean-code", "patterns"],
          version: 1,
          isPreset: true,
          isEditable: false,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "perf-opt",
          name: "Performance Optimization",
          description: "Identify and optimize performance bottlenecks",
          icon: "Zap",
          tags: ["performance", "optimization", "profiling"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "migration",
          name: "Migration",
          description: "Framework and language migration workflows",
          icon: "Ship",
          tags: ["migration", "upgrade", "compatibility"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "api-design",
          name: "API Design",
          description: "Design and document RESTful APIs",
          icon: "Cloud",
          tags: ["api", "rest", "design"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "env-debug",
          name: "Environment Debug",
          description: "Debug and diagnose environment issues",
          icon: "Bug",
          tags: ["debug", "troubleshoot", "environment"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "feature-impl",
          name: "Feature Implementation",
          description: "Implement new features with AI assistance",
          icon: "Sparkles",
          tags: ["feature", "ai", "implementation"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "knowledge-extract",
          name: "Knowledge Extraction",
          description: "Extract structured knowledge from documents",
          icon: "Brain",
          tags: ["knowledge", "extraction", "nlp"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "knowledge-to-code",
          name: "Knowledge to Code",
          description: "Convert knowledge into executable code",
          icon: "Code",
          tags: ["knowledge", "code", "generation"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "custom-1",
          name: "My Custom Workflow",
          description: "A custom workflow created by user",
          icon: "Star",
          tags: ["custom", "user"],
          version: 1,
          isPreset: false,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-finance-invest",
          name: "金融投资分析",
          description: "财务分析 → 投资建议（CFO 视角）",
          icon: "LineChart",
          tags: ["opc", "finance", "investment"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-finance-stock-research",
          name: "个股投资研究",
          description: "A股个股深度研究：行情→财报→资金流→投资结论（astock-data 工具链）",
          icon: "TrendingUp",
          tags: ["opc", "finance", "stock", "astock"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-ai-research",
          name: "AI 科技研究报告",
          description: "AI 技术调研、原型验证与报告",
          icon: "Robot",
          tags: ["opc", "ai", "research"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-software-dev",
          name: "软件开发流程",
          description: "需求→架构→编码→测试→部署",
          icon: "Code",
          tags: ["opc", "dev", "software"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-accounting",
          name: "会计财务流程",
          description: "发票创建→审批→通知→KPI 记录",
          icon: "Calculator",
          tags: ["opc", "finance", "accounting"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-ecommerce",
          name: "电商运营流程",
          description: "选品→店铺→营销→订单",
          icon: "ShoppingCart",
          tags: ["opc", "ecommerce", "brand"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "workflow-education",
          name: "教育培训流程",
          description: "课程设计→内容制作→学员管理→评估",
          icon: "GraduationCap",
          tags: ["opc", "education", "training"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "prod-startup-mvp",
          name: "Startup MVP 流水线",
          description: "从想法到 MVP 的完整生产流程",
          icon: "Rocket",
          tags: ["opc", "production", "startup", "mvp"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "prod-landing-page",
          name: "落地页营销工作流",
          description: "创建→发布→跟踪落地页效果",
          icon: "FileText",
          tags: ["opc", "production", "landing"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [],
          edges: [],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
        {
          id: "stock-analysis",
          name: "股票分析",
          description: "技术面/基本面/新闻分析",
          icon: "TrendingUp",
          tags: ["stock", "analysis", "A股"],
          version: 1,
          isPreset: true,
          isEditable: true,
          isPublic: false,
          triggerConfig: { trigger_type: "manual", config: {} },
          nodes: [
            {
              id: "trigger_1",
              type: "trigger",
              label: "触发",
              config: { triggerType: "manual" },
              position: { x: 100, y: 100 },
            },
            {
              id: "stock_data_fetch",
              type: "tool",
              label: "获取股票数据",
              config: { toolName: "astock-data" },
              position: { x: 300, y: 100 },
            },
            {
              id: "analysis_llm",
              type: "agent",
              label: "分析",
              config: { agent_id: "stock-analyst" },
              position: { x: 500, y: 100 },
            },
            {
              id: "result_end",
              type: "end",
              label: "结束",
              config: {},
              position: { x: 700, y: 100 },
            },
          ],
          edges: [
            { id: "edge_1", source: "trigger_1", target: "stock_data_fetch", type: "data" },
            { id: "edge_2", source: "stock_data_fetch", target: "analysis_llm", type: "data" },
            { id: "edge_3", source: "analysis_llm", target: "result_end", type: "data" },
          ],
          inputSchema: null,
          outputSchema: null,
          variables: [],
          errorConfig: null,
          createdAt: nowTs(),
          updatedAt: nowTs(),
        },
      ];
      setStore("workflow_templates", presetTemplates);
      return presetTemplates.length as T;
    }
    case "list_workflow_templates": {
      const isPreset = (args as { isPreset?: boolean })?.isPreset;
      const includeSystem = (args as { includeSystem?: boolean })?.includeSystem;
      let templates = getStore<WorkflowTemplate[]>("workflow_templates", []);
      if (isPreset !== undefined) {
        templates = templates.filter((t) => t.isPreset === isPreset);
      }
      // 默认过滤系统模板（认知编排器等）；include_system=true 时返回
      if (!includeSystem) {
        templates = templates.filter((t) => !t.isSystem);
      }
      return templates as T;
    }

    // ── Gateway Links ─────────────────────────────────────────────────
    case "list_gateway_links":
      return getStore("gateway_links", []) as T;

    // ── Workflow Templates ────────────────────────────────────────────
    case "get_workflow_template": {
      const id = (args as { id?: string })?.id;
      const includeSystem = (args as { include_system?: boolean })?.include_system;
      const templates = getStore<WorkflowTemplate[]>("workflow_templates", []);
      const found = templates.find((t) => t.id === id);
      // 系统模板默认不可见；include_system=true 时才可读取
      if (!found || (!includeSystem && found.isSystem)) {
        return null as T;
      }
      return found as T;
    }
    case "create_workflow_template": {
      const input = (args as { input?: CreateWorkflowTemplateInput }).input ?? {};
      const newId = genId();
      const now = nowTs();
      const template: WorkflowTemplate = {
        id: newId,
        name: input.name || "Unnamed Workflow",
        description: input.description || "",
        icon: "Bot",
        tags: input.tags || [],
        version: 1,
        isPreset: false,
        isEditable: true,
        isPublic: false,
        triggerConfig: { type: "manual", config: {} },
        nodes: input.nodes?.length
          ? input.nodes
          : [
            {
              id: genId(),
              type: "trigger",
              label: i18n.t("browserMock.triggerLabel"),
              config: { triggerType: "manual" },
              position: { x: 100, y: 100 },
            },
          ],
        edges: input.edges || [],
        createdAt: now,
        updatedAt: now,
      };
      const templates = getStore<WorkflowTemplate[]>("workflow_templates", []);
      templates.push(template);
      setStore("workflow_templates", templates);
      return newId as T;
    }
    case "update_workflow_template": {
      const updateId = (args as { id?: string }).id;
      const updateInput = (args as { input?: UpdateWorkflowTemplateInput }).input ?? {};
      const templates = getStore<WorkflowTemplate[]>("workflow_templates", []);
      const idx = templates.findIndex((t) => t.id === updateId);
      if (idx >= 0) {
        if (updateInput.name !== undefined) {
          templates[idx].name = updateInput.name;
        }
        if (updateInput.description !== undefined) {
          templates[idx].description = updateInput.description;
        }
        if (updateInput.tags !== undefined) {
          templates[idx].tags = updateInput.tags;
        }
        if (updateInput.nodes !== undefined) {
          templates[idx].nodes = updateInput.nodes;
        }
        if (updateInput.edges !== undefined) {
          templates[idx].edges = updateInput.edges;
        }
        templates[idx].updatedAt = nowTs();
        setStore("workflow_templates", templates);
      }
      return true as T;
    }
    case "delete_workflow_template": {
      const deleteId = (args as { id?: string }).id;
      const templates = getStore<WorkflowTemplate[]>("workflow_templates", []);
      setStore(
        "workflow_templates",
        templates.filter((t) => t.id !== deleteId),
      );
      return undefined as T;
    }

    // Platform / Message Channel commands
    case "get_platform_config": {
      return (getStore<PlatformConfig | null>("platform_config", null) ?? {
        telegramEnabled: false,
        telegramBotToken: null,
        telegramWebhookUrl: null,
        telegramWebhookSecret: null,
        telegramAllowedUsers: null,
        discordEnabled: false,
        discordBotToken: null,
        discordWebhookUrl: null,
        discordAllowedChannels: null,
        slackEnabled: false,
        slackBotToken: null,
        slackSigningSecret: null,
        slackWorkspaceId: null,
        slackAppToken: null,
        whatsappEnabled: false,
        whatsappPhoneNumberId: null,
        whatsappAccessToken: null,
        whatsappBusinessAccountId: null,
        whatsappWebhookVerifyToken: null,
        whatsappApiVersion: null,
        wechatEnabled: false,
        wechatAppId: null,
        wechatAppSecret: null,
        wechatToken: null,
        wechatEncodingAesKey: null,
        wechatOriginalId: null,
        wechatMode: null,
        feishuEnabled: false,
        feishuAppId: null,
        feishuAppSecret: null,
        feishuVerificationToken: null,
        feishuEncryptKey: null,
        qqEnabled: false,
        qqBotAppId: null,
        qqBotToken: null,
        qqBotSecret: null,
        dingtalkEnabled: false,
        dingtalkAppKey: null,
        dingtalkAppSecret: null,
        dingtalkAgentId: null,
        dingtalkRobotCode: null,
        apiServerEnabled: false,
        apiServerPort: 8080,
        autoSyncMessages: false,
        maxHistoryPerSession: 100,
      }) as T;
    }
    case "update_platform_config": {
      const input = args as Partial<PlatformConfig>;
      const existing = getStore<PlatformConfig | null>("platform_config", null)
        ?? ({} as PlatformConfig);
      const merged = { ...existing, ...input };
      setStore("platform_config", merged);
      return undefined as T;
    }
    case "get_platform_statuses": {
      const config = getStore<PlatformConfig | null>("platform_config", null);
      if (!config) {
        return [] as T;
      }
      const keys: { key: keyof PlatformConfig; name: string }[] = [
        { key: "telegramEnabled", name: "Telegram" },
        { key: "discordEnabled", name: "Discord" },
        { key: "slackEnabled", name: "Slack" },
        { key: "whatsappEnabled", name: "WhatsApp" },
        { key: "wechatEnabled", name: "WeChat" },
        { key: "feishuEnabled", name: "Feishu" },
        { key: "qqEnabled", name: "QQ" },
        { key: "dingtalkEnabled", name: "DingTalk" },
      ];
      return keys.map(({ key, name }) => ({
        name,
        enabled: !!config[key],
        connected: false,
        last_activity: null,
        active_sessions: 0,
      })) as T;
    }
    case "reconcile_platforms": {
      return { started: [], stopped: [], errors: [] } as T;
    }
    case "get_active_sessions": {
      return getStore<PlatformSession[]>("platform_sessions", []) as T;
    }
    case "create_platform_session": {
      const input = args as { platform: string; chat_id: string };
      const sessions = getStore<PlatformSession[]>("platform_sessions", []);
      const session: PlatformSession = {
        sessionId: `mock-${input.platform}-${Date.now()}`,
        platform: input.platform,
        userId: input.chat_id,
        username: null,
        isActive: true,
        lastActivity: Date.now(),
      };
      sessions.push(session);
      setStore("platform_sessions", sessions);
      return session as T;
    }
    case "deactivate_platform_session": {
      const input = args as { sessionId: string };
      const sessions = getStore<PlatformSession[]>("platform_sessions", []);
      setStore(
        "platform_sessions",
        sessions.map((s) => s.sessionId === input.sessionId ? { ...s, isActive: false } : s),
      );
      return undefined as T;
    }
    case "send_platform_message": {
      return { ok: true, message_id: `mock-msg-${Date.now()}` } as T;
    }
    case "process_telegram_message":
    case "process_discord_message":
    case "process_platform_message":
      // 后端 process_platform_message 返回 Ok(None)（平台白名单校验 + 日志）
      return null as T;
    case "start_api_server": {
      setStore("api_server_running", true);
      return { port: (args as { port?: number }).port ?? 8080 } as T;
    }
    case "stop_api_server": {
      setStore("api_server_running", false);
      return undefined as T;
    }

    // ── Plugins (OpenClaw) ─────────────────────────────────────────────
    case "plugin_list": {
      const plugins = getStore<Array<Record<string, unknown>>>("plugins", []);
      return plugins as T;
    }
    case "plugin_validate_source": {
      const source = (args?.source as string) || "";
      return {
        name: source.split("/").pop() || source,
        version: "0.0.0",
        description: `Plugin from ${source}`,
        permissions: [],
        defaultEnabled: true,
        hooks: {},
        tools: [],
        mcpServers: [],
        skills: [],
      } as T;
    }
    case "plugin_install": {
      const plugins = getStore<Array<Record<string, unknown>>>("plugins", []);
      const source = (args?.source as string) || "";
      const id = `plugin-${plugins.length + 1}`;
      plugins.push({
        id,
        name: source.split("/").pop() || source,
        version: "0.0.0",
        description: `Plugin from ${source}`,
        kind: "openclaw",
        enabled: true,
        tools: [],
        mcpServers: [],
        skills: [],
      });
      setStore("plugins", plugins);
      return {
        pluginId: id,
        version: "0.0.0",
        installPath: `/mock/plugins/${id}`,
      } as T;
    }
    case "plugin_enable":
    case "plugin_disable": {
      const allPlugins = getStore<Array<Record<string, unknown>>>(
        "plugins",
        [],
      );
      const pluginId = (args?.pluginId as string) || "";
      const idx = allPlugins.findIndex((p) => p.id === pluginId);
      if (idx !== -1) {
        allPlugins[idx] = {
          ...allPlugins[idx],
          enabled: cmd === "plugin_enable",
        };
        setStore("plugins", allPlugins);
      }
      return undefined as T;
    }
    case "plugin_uninstall": {
      let allPlugins = getStore<Array<Record<string, unknown>>>("plugins", []);
      const pluginId = (args?.pluginId as string) || "";
      allPlugins = allPlugins.filter((p) => p.id !== pluginId);
      setStore("plugins", allPlugins);
      return undefined as T;
    }
    case "plugin_update": {
      return {
        pluginId: (args?.pluginId as string) || "",
        version: "0.0.0",
        installPath: "",
      } as T;
    }

    // ── Agent Profiles (mock) ──────────────────────────────────────
    case "list_agent_profiles":
    case "list_agent_roles":
      return [] as T;

    // ── Dashboard Plugins (mock) ────────────────────────────────────
    case "dashboard_list_plugins":
      return [] as T;

    // ── Prompt Templates (mock) ─────────────────────────────────────
    case "list_prompt_templates":
      return [] as T;

    // ── PTY Terminal (mock) ──────────────────────────────────────────
    case "pty_create_session":
      return `pty-mock-${Date.now()}` as T;
    case "pty_write":
    case "pty_resize":
    case "pty_kill_session":
    case "pty_remove_session":
    case "pty_analyze_output":
    case "generate_workflow_from_prompt": {
      return {
        nodes: [
          {
            id: "trigger-1",
            type: "trigger",
            label: i18n.t("browserMock.triggerLabel"),
            config: { triggerType: "manual" },
            position: { x: 100, y: 100 },
          },
          {
            id: "action-1",
            type: "tool",
            label: i18n.t("browserMock.executeLabel"),
            config: { toolName: "mock_tool" },
            position: { x: 300, y: 100 },
          },
        ],
        edges: [
          { id: "e1", source: "trigger-1", target: "action-1" },
        ],
        explanation: i18n.t("browserMock.nlWorkflowExplanation", {
          prompt: (args as { prompt?: string })?.prompt ?? "",
        }),
      } as T;
    }

    // ── DynamicUI Schema CRUD ──────────────────────────────────────────
    case "list_dynamic_ui_schemas": {
      const { category } = (args as { category?: string | null }) ?? {};
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      const filtered = category
        ? schemas.filter((s) => s.category === category)
        : schemas;
      return filtered as T;
    }
    case "get_dynamic_ui_schema": {
      const { id } = args as { id?: string };
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      const schema = schemas.find((s) => s.id === id) ?? null;
      return schema as T;
    }
    case "create_dynamic_ui_schema": {
      const req = (args as { req?: CreateDynamicUISchemaParams }).req;
      if (!req) {
        throw new Error("Missing req parameter");
      }
      const now = new Date().toISOString();
      const schema: DynamicUISchemaRecord = {
        id: genId(),
        title: req.title,
        description: req.description,
        schemaJson: req.schemaJson,
        category: req.category,
        tags: req.tags,
        version: "1.0.0",
        isBuiltin: false,
        createdAt: now,
        updatedAt: now,
      };
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      schemas.push(schema);
      saveMockDynamicUIData("schemas", schemas);
      // 创建初始版本记录
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      versions.push({
        id: Date.now(),
        schemaId: schema.id,
        version: schema.version,
        title: schema.title,
        description: schema.description,
        schemaJson: schema.schemaJson,
        category: schema.category,
        tags: schema.tags,
        changeLog: "initial create",
        createdAt: Date.now(),
      });
      saveMockDynamicUIData("versions", versions);
      return schema as T;
    }
    case "update_dynamic_ui_schema": {
      const { id, req } = args as {
        id?: string;
        req?: UpdateDynamicUISchemaParams;
      };
      if (!id || !req) {
        throw new Error("Missing id or req parameter");
      }
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      const idx = schemas.findIndex((s) => s.id === id);
      if (idx === -1) {
        throw new Error("Schema not found");
      }
      const old = schemas[idx];
      // 版本号：传了用传入值，否则 patch 自增
      const newVersion = req.version ?? bumpPatchVersion(old.version);
      const updated: DynamicUISchemaRecord = {
        ...old,
        title: req.title ?? old.title,
        description: req.description ?? old.description,
        schemaJson: req.schemaJson ?? old.schemaJson,
        category: req.category ?? old.category,
        tags: req.tags ?? old.tags,
        version: newVersion,
        updatedAt: new Date().toISOString(),
      };
      schemas[idx] = updated;
      saveMockDynamicUIData("schemas", schemas);
      // 创建版本记录
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      versions.push({
        id: Date.now(),
        schemaId: updated.id,
        version: updated.version,
        title: updated.title,
        description: updated.description,
        schemaJson: updated.schemaJson,
        category: updated.category,
        tags: updated.tags,
        changeLog: req.changeLog ?? "update",
        createdAt: Date.now(),
      });
      saveMockDynamicUIData("versions", versions);
      return updated as T;
    }
    case "delete_dynamic_ui_schema": {
      const { id } = args as { id?: string };
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      saveMockDynamicUIData(
        "schemas",
        schemas.filter((s) => s.id !== id),
      );
      // 同步清理版本记录
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      saveMockDynamicUIData(
        "versions",
        versions.filter((v) => v.schemaId !== id),
      );
      return undefined as T;
    }

    // ── DynamicUI 表单数据持久化 ──────────────────────────────────────
    case "save_dynamic_ui_form_data": {
      const req = (args as { req?: SaveDynamicUIFormDataParams }).req;
      if (!req) {
        throw new Error("Missing req parameter");
      }
      const instanceKey = req.instanceKey ?? "__default__";
      const records = loadMockDynamicUIData<DynamicUIFormDataRecord[]>("formData", []);
      const idx = records.findIndex(
        (r) => r.schemaId === req.schemaId && r.instanceKey === instanceKey,
      );
      const now = new Date().toISOString();
      const record: DynamicUIFormDataRecord = {
        id: idx !== -1 ? records[idx].id : genId(),
        schemaId: req.schemaId,
        instanceKey: instanceKey,
        formDataJson: req.formDataJson,
        updatedAt: now,
      };
      if (idx !== -1) {
        records[idx] = record;
      } else {
        records.push(record);
      }
      saveMockDynamicUIData("formData", records);
      return record as T;
    }
    case "get_dynamic_ui_form_data": {
      const { schema_id, instance_key } = args as {
        schema_id?: string;
        instance_key?: string | null;
      };
      const instanceKey = instance_key ?? "__default__";
      const records = loadMockDynamicUIData<DynamicUIFormDataRecord[]>("formData", []);
      const record = records.find(
        (r) => r.schemaId === schema_id && r.instanceKey === instanceKey,
      ) ?? null;
      return record as T;
    }
    case "delete_dynamic_ui_form_data": {
      const { schema_id, instance_key } = args as {
        schema_id?: string;
        instance_key?: string | null;
      };
      const instanceKey = instance_key ?? "__default__";
      const records = loadMockDynamicUIData<DynamicUIFormDataRecord[]>("formData", []);
      saveMockDynamicUIData(
        "formData",
        records.filter(
          (r) => !(r.schemaId === schema_id && r.instanceKey === instanceKey),
        ),
      );
      return undefined as T;
    }

    // ── DynamicUI 钉入配置 ────────────────────────────────────────────
    case "list_dynamic_ui_pins": {
      const pins = loadMockDynamicUIData<DynamicUIPinRecord[]>("pins", []);
      return pins as T;
    }
    case "pin_dynamic_ui_schema": {
      const { schema_id, title, group_name, position } = args as {
        schema_id?: string;
        title?: string;
        group_name?: string;
        position?: number | null;
      };
      if (!schema_id) {
        throw new Error("Missing schema_id");
      }
      const pins = loadMockDynamicUIData<DynamicUIPinRecord[]>("pins", []);
      const idx = pins.findIndex((p) => p.schemaId === schema_id);
      const now = new Date().toISOString();
      const pos = position
        ?? (pins.length > 0 ? Math.max(...pins.map((p) => p.position)) + 1 : 0);
      const record: DynamicUIPinRecord = {
        schemaId: schema_id,
        title: title ?? "",
        groupName: group_name ?? "",
        position: pos,
        createdAt: idx !== -1 ? pins[idx].createdAt : now,
        updatedAt: now,
      };
      if (idx !== -1) {
        pins[idx] = record;
      } else {
        pins.push(record);
      }
      saveMockDynamicUIData("pins", pins);
      return record as T;
    }
    case "unpin_dynamic_ui_schema": {
      const { schema_id } = args as { schema_id?: string };
      const pins = loadMockDynamicUIData<DynamicUIPinRecord[]>("pins", []);
      saveMockDynamicUIData(
        "pins",
        pins.filter((p) => p.schemaId !== schema_id),
      );
      return undefined as T;
    }

    // ── DynamicUI 版本管理 ────────────────────────────────────────────
    case "list_dynamic_ui_schema_versions": {
      const { schema_id } = args as { schema_id?: string };
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      const filtered = versions.filter((v) => v.schemaId === schema_id);
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      const schema = schemas.find((s) => s.id === schema_id);
      const response: ListVersionsResponse = {
        versions: filtered,
        currentVersion: schema?.version ?? "",
      };
      return response as T;
    }
    case "get_dynamic_ui_schema_version": {
      const { version_id } = args as { version_id?: number };
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      const version = versions.find((v) => v.id === version_id) ?? null;
      return version as T;
    }
    case "restore_dynamic_ui_schema_version": {
      const { schema_id, version_id } = args as {
        schema_id?: string;
        version_id?: number;
      };
      const versions = loadMockDynamicUIData<DynamicUISchemaVersion[]>("versions", []);
      const version = versions.find(
        (v) => v.id === version_id && v.schemaId === schema_id,
      );
      if (!version) {
        throw new Error("Version not found");
      }
      const schemas = loadMockDynamicUIData<DynamicUISchemaRecord[]>("schemas", []);
      const idx = schemas.findIndex((s) => s.id === schema_id);
      if (idx === -1) {
        throw new Error("Schema not found");
      }
      const restored: DynamicUISchemaRecord = {
        id: schemas[idx].id,
        title: version.title,
        description: version.description,
        schemaJson: version.schemaJson,
        category: version.category,
        tags: version.tags,
        version: version.version,
        isBuiltin: schemas[idx].isBuiltin,
        createdAt: schemas[idx].createdAt,
        updatedAt: new Date().toISOString(),
      };
      schemas[idx] = restored;
      saveMockDynamicUIData("schemas", schemas);
      return restored as T;
    }

    // ── DynamicUI 自然语言生成 ────────────────────────────────────────
    case "edit_dynamic_ui_schema_nl": {
      const { prompt } = args as { existing_schema?: string; prompt?: string };
      const mockSchema = buildMockUISchemaJSON();
      return {
        schema: mockSchema,
        description: `Mock: edited example schema per instruction "${(prompt ?? "").slice(0, 50)}"`,
      } as T;
    }
    case "generate_dynamic_ui_schema_nl": {
      const { prompt } = args as { prompt?: string };
      const mockSchema = buildMockUISchemaJSON();
      return {
        schema: mockSchema,
        title: "Mock Dynamic UI",
        description: `Mock: generated example schema per description "${(prompt ?? "").slice(0, 50)}"`,
      } as T;
    }

    // ── DevTools: Tracer (轨迹追踪) ───────────────────────────────────
    case "tracer_start_span":
      return genId() as T;
    case "tracer_end_span":
    case "tracer_record_error":
    case "tracer_record_span":
    case "tracer_delete_trace":
    case "tracer_submit_feedback":
    case "telemetry_report_error":
      return undefined as T;
    case "tracer_list_traces":
    case "tracer_get_feedback":
    case "tracer_generate_suggestions":
      return [] as T;
    case "tracer_get_trace":
    case "tracer_get_span":
    case "tracer_get_metrics":
      return null as T;
    case "tracer_export_traces":
      // 后端返回 Vec<u8>，浏览器 mock 用空数组占位
      return [] as unknown as T;
    case "tracer_delete_old_traces":
      return 0 as T;
    case "tracer_get_bottlenecks":
      return { bottlenecks: [], summary: {} } as T;

    // ── DevTools: Evaluator (评估器) ──────────────────────────────────
    case "evaluator_list_benchmarks":
    case "evaluator_list_datasets":
      return [] as T;
    case "evaluator_get_benchmark":
    case "evaluator_get_ab_results":
      return null as T;
    case "evaluator_run_benchmark":
      return {
        benchmark_id: "",
        status: "completed",
        results: [],
        started_at: nowTs(),
        completed_at: nowTs(),
      } as T;
    case "evaluator_generate_report":
      return {
        id: genId(),
        benchmark_id: "",
        generated_at: nowTs(),
        summary: {},
        metrics: {},
      } as T;
    case "evaluator_import_dataset":
      return {
        id: genId(),
        name: "Mock Dataset",
        source_path: (args as { path?: string })?.path ?? "",
        size: 0,
        created_at: nowTs(),
      } as T;
    case "evaluator_export_report":
      return "" as T;
    case "evaluator_run_ab_test":
      return {
        test_id: genId(),
        status: "completed",
        variants: {},
        started_at: nowTs(),
        completed_at: nowTs(),
      } as T;

    // ── DevTools: RL Training (强化学习) ──────────────────────────────
    case "start_rl_training":
    case "stop_rl_training":
    case "load_checkpoint":
    case "delete_checkpoint":
      return undefined as T;
    case "get_training_metrics":
      return {
        episode: 0,
        reward: 0,
        loss: 0,
        steps: 0,
        avg_reward: 0,
        epsilon: 0,
        learning_rate: 0,
      } as T;
    case "save_checkpoint":
      return {
        id: genId(),
        episode: 0,
        reward: 0,
        created_at: nowTs(),
        path: "/mock/checkpoint.pt",
      } as T;
    case "list_checkpoints":
      return [] as T;
    case "run_rl_training_step":
      return { step: 0, reward: 0, loss: 0, done: false } as T;

    // ── DevTools: Fine-tune (微调) ────────────────────────────────────
    case "list_datasets":
      return [] as T;
    case "get_dataset":
      return {
        id: "",
        name: "",
        description: "",
        numSamples: 0,
        createdAt: 0,
      } as T;
    case "create_dataset":
      return {
        id: genId(),
        name: (args as { name?: string })?.name ?? "Mock Dataset",
        description: (args as { description?: string })?.description ?? "",
        numSamples: 0,
        createdAt: nowTs(),
      } as T;
    case "add_sample":
    case "delete_dataset":
    case "cancel_training_job":
    case "delete_training_job":
    case "start_training_job":
    case "set_active_model":
      return undefined as T;
    case "list_training_jobs":
      return [] as T;
    case "get_training_job":
      return {
        id: "",
        status: "pending",
        datasetId: "",
        baseModel: "",
        progressPercent: 0,
        currentLoss: 0,
        outputLora: null,
      } as T;
    case "create_training_job":
      return {
        id: genId(),
        datasetId: (args as { datasetId?: string })?.datasetId ?? "",
        baseModel: (args as { baseModel?: string })?.baseModel ?? "",
        status: "pending",
        progressPercent: 0,
        currentLoss: 0,
        outputLora: null,
      } as T;
    case "get_training_stats":
      return {
        totalJobs: 0,
        completedJobs: 0,
        runningJobs: 0,
        failedJobs: 0,
      } as T;
    case "list_base_models":
    case "list_lora_adapters":
      return [] as T;
    case "get_active_model":
      return null as T;

    // ── DevTools: Agent Analytics (智能体分析) ────────────────────────
    case "trajectory_stats":
      return {
        total_trajectories: 0,
        total_steps: 0,
        avg_steps: 0,
        avg_quality: 0,
        success_rate: 0,
      } as T;
    case "trajectory_list":
      return [] as T;
    case "get_trajectory_detail":
      return null as T;
    case "pattern_stats":
      return {
        total_patterns: 0,
        high_value_patterns: 0,
        failure_patterns: 0,
        avg_success_rate: 0,
      } as T;
    case "closed_loop_status":
      return {
        closed_loop_running: false,
        nudge_count: 0,
        pattern_count: 0,
        insight_count: 0,
      } as T;
    case "rl_config":
      return { config: {}, weights: {} } as T;
    case "rl_export_training_data":
      return [] as T;
    case "rl_compute_rewards":
      return {
        trajectory_id: (args as { trajectory_id?: string })?.trajectory_id
          ?? "",
        reward_count: 0,
        total_reward: 0,
        value_count: 0,
        advantage_count: 0,
      } as T;
    case "record_feedback":
      return undefined as T;

    // ── DevTools: Evolution (进化) ────────────────────────────────────
    case "pattern_list":
    case "cross_session_insights":
      return [] as T;
    case "skill_evolution_start":
      return {
        skill_id: (args as { skill_id?: string })?.skill_id ?? "",
        improved: false,
        reason: "Mock: no evolution result generated",
        confidence: 0,
      } as T;
    case "skill_evolution_status":
      return { is_running: false, stats: {} } as T;
    case "adaptation_status":
      return {
        response_style: "",
        content_adjustments: [],
        skill_suggestions: [],
        memory_priorities: [],
      } as T;

    // ── DevTools: Evolution Engine (进化引擎) ─────────────────────────
    case "get_all_engine_status":
      return [] as T;
    case "start_engine":
    case "stop_engine":
      return undefined as T;
    case "update_engine_config":
      return { success: true } as T;
    case "get_engine_logs":
      return [] as T;
    case "run_skill_evolution_generation":
    case "run_text_grad_optimize":
    case "run_dream_consolidation":
    case "run_auto_tool_create":
    case "run_process_reward_analysis":
    case "run_intrinsic_motivation_analysis":
    case "run_coevolution_cycle":
    case "run_sandbox_validate_step":
      return { success: false, reason: "Mock mode not enabled", stats: {} } as T;
    case "get_coevolution_status":
      return { is_running: false, generation: 0, stats: {} } as T;
    case "get_sandbox_policy":
      return { enabled: false, rules: [] } as T;
    case "batch_upsert_entities_and_relations":
      return {
        newEntities: [],
        updatedEntities: [],
        newRelations: [],
        skippedChunks: 0,
        elapsedMs: 0,
      } as T;

    // ── OPC 行业分析决策（行业页「执行分析」）────────────────────────
    // 此前**没有 mock**：会命中下面 default 的 `endsWith("s")` 兜底
    // （"opc_execute_analysis" 以 s 结尾）返回 `[]`，于是面板拿到一个非决策值。
    case "opc_execute_analysis": {
      // 形状逐字对齐 `OpcDomainDecision`（snake_case，无 `rename_all`）。
      // 该命令已在 `SNAKE_CASE_RESPONSE_COMMANDS` 中豁免出口 camel 转换，否则
      // `decision_type` / `risk_level` / `domain_pack_id` 会被改写成 camelCase，
      // 而面板与 `types.ts` 都按后端形态读，两个键就都会变成 `undefined`。
      return {
        domain_pack_id: "content_media",
        decision_type: "performance_review",
        summary: "行业「content_media」分析：发现 1 个风控违规",
        confidence: 0.5,
        kpis: mockContentMediaKpis(),
        recommendations: ["⚠️ min_word_count: word_count 当前 200.0 低于下限 1000.0"],
        risk_level: "medium",
      } as unknown as T;
    }

    case "opc_list_platforms":
      return [...PRESET_MOCK_PLATFORMS] as unknown as T;

    case "opc_get_scan_policy":
      // 与 ScanPolicy::default 对齐（crates/tools/src/tools/scan_policy.rs）
      return {
        concurrency: 4,
        rateLimitPerMin: 60,
        retryMax: 2,
        retryBackoffMs: 500,
        timeoutSecs: 15,
        dedupWindowHours: 168,
        maxLeadsPerScan: 200,
        llmEvalEnabled: true,
        llmEvalMaxLeads: 20,
        llmEvalMinRuleScore: 30,
      } as T;

    case "opc_save_scan_policy":
      return (args as { policy?: unknown }).policy as T;

    case "opc_save_platform": {
      const input = (args ?? {}) as {
        id?: string;
        name?: string;
        platformType?: string;
        enabled?: boolean;
        baseUrl?: string;
        config?: Record<string, unknown>;
      };
      const now = Math.floor(Date.now() / 1000);
      const existingIdx = PRESET_MOCK_PLATFORMS.findIndex((p) => p.id === input.id);
      let platform: MarketPlatform;
      if (existingIdx >= 0) {
        platform = {
          ...PRESET_MOCK_PLATFORMS[existingIdx],
          ...(input.name !== undefined ? { name: input.name } : {}),
          ...(input.platformType !== undefined ? { platformType: input.platformType } : {}),
          ...(input.enabled !== undefined ? { enabled: input.enabled } : {}),
          ...(input.baseUrl !== undefined ? { baseUrl: input.baseUrl } : {}),
          ...(input.config !== undefined ? { config: input.config } : {}),
          updatedAt: now,
        };
        PRESET_MOCK_PLATFORMS[existingIdx] = platform;
      } else {
        platform = buildMockPlatform(
          input.id || `mp-${Date.now()}`,
          input.name || "New Platform",
          input.baseUrl ?? null,
        );
        platform = {
          ...platform,
          platformType: input.platformType || "manual",
          enabled: input.enabled ?? true,
          config: input.config ?? null,
          createdAt: now,
          updatedAt: now,
        };
        PRESET_MOCK_PLATFORMS.push(platform);
      }
      return { ...platform } as unknown as T;
    }

    case "opc_delete_platform": {
      const { id } = args as { id: string };
      const idx = PRESET_MOCK_PLATFORMS.findIndex((p) => p.id === id);
      if (idx >= 0) {
        PRESET_MOCK_PLATFORMS.splice(idx, 1);
      }
      return null as unknown as T;
    }

    case "opc_list_leads": {
      const { minScore, status } = (args ?? {}) as {
        limit?: number;
        minScore?: number;
        status?: string;
      };
      let filtered = typeof minScore === "number"
        ? MOCK_DEMAND_LEADS.filter((l) => l.commercialValueScore >= minScore)
        : [...MOCK_DEMAND_LEADS];
      if (status) {
        filtered = filtered.filter((l) => l.status === status);
      }
      return filtered.sort((a, b) => b.commercialValueScore - a.commercialValueScore) as unknown as T;
    }

    case "opc_discover_and_evaluate_leads": {
      const { query } = (args ?? {}) as { query?: string };
      // 浏览器模式没有真实扫描器：按关键词对预置线索做一次「伪命中」，
      // 只为打通 UI 链路，结果不代表真实扫描。
      const kw = (query ?? "").trim().toLowerCase();
      const hits = kw
        ? MOCK_DEMAND_LEADS.filter((l) => (l.title + l.description + l.platform).toLowerCase().includes(kw))
        : MOCK_DEMAND_LEADS;
      const matched = hits.length > 0 ? hits : MOCK_DEMAND_LEADS.slice(0, 1);
      const highValue = matched.filter((l) => l.commercialValueScore >= 60);
      return {
        totalScanned: matched.length,
        totalEvaluated: matched.length,
        totalSaved: 0,
        highValueCount: highValue.length,
        leads: highValue,
      } as unknown as T;
    }

    case "opc_update_lead_status": {
      const { leadId, status } = (args ?? {}) as { leadId?: string; status?: string };
      const lead = MOCK_DEMAND_LEADS.find((l) => l.id === leadId);
      if (!lead) {
        throw new Error(`需求线索不存在: ${leadId}`);
      }
      const legal: Record<string, string[]> = {
        new: ["evaluated", "contacted", "lost"],
        evaluated: ["contacted", "lost"],
        contacted: ["won", "lost"],
      };
      if (lead.status !== status && !(legal[lead.status] ?? []).includes(status ?? "")) {
        throw new Error(`非法状态迁移: ${lead.status} → ${status}`);
      }
      lead.status = status ?? lead.status;
      lead.updatedAt = Math.floor(Date.now() / 1000);
      return { ...lead } as unknown as T;
    }

    case "opc_convert_lead_to_workflow": {
      const { leadId } = (args ?? {}) as { leadId?: string };
      const lead = MOCK_DEMAND_LEADS.find((l) => l.id === leadId);
      if (!lead) {
        throw new Error(`需求线索不存在: ${leadId}`);
      }
      if (lead.linkedWorkflowId) {
        throw new Error(`线索已转化，工作流 ID: ${lead.linkedWorkflowId}`);
      }
      // 浏览器模式没有 workflow_templates 表：返回最小模板结构（与真实契约同形）
      const template = {
        id: `demand:lead:${lead.id}`,
        name: `需求实现: ${lead.title.slice(0, 40)}`,
        description: lead.description,
        icon: "target",
        tags: ["opc_demand", "demand_implement"],
        version: 1,
        isPreset: false,
        isEditable: true,
        isPublic: false,
        triggerConfig: { triggerType: "manual", config: {} },
        nodes: [],
        edges: [],
        variables: [],
        toolDefs: [],
        clusterId: "demand_implement",
        routePath: `/automation/demand/${lead.id}`,
      };
      lead.linkedWorkflowId = template.id;
      lead.updatedAt = Math.floor(Date.now() / 1000);
      return template as unknown as T;
    }

    case "opc_run_lead_workflow": {
      const { leadId } = (args ?? {}) as { leadId?: string };
      const lead = MOCK_DEMAND_LEADS.find((l) => l.id === leadId);
      if (!lead) {
        throw new Error(`需求线索不存在: ${leadId}`);
      }
      if (!lead.linkedWorkflowId) {
        throw new Error("线索尚未转化为工作流，请先转化");
      }
      lead.implementedAt = Math.floor(Date.now() / 1000);
      lead.updatedAt = lead.implementedAt;
      return `mock-exec-${lead.id}-${Date.now()}` as unknown as T;
    }

    // ── OPC 手动补录线索（P1-4）──────────────────────────────────────
    case "opc_create_lead": {
      const { input } = (args ?? {}) as { input?: Record<string, unknown> };
      const title = (typeof input?.title === "string" ? input.title : "").trim();
      const description = (typeof input?.description === "string" ? input.description : "").trim();
      if (!title || !description) {
        throw new Error("title 与 description 不能为空");
      }
      const now = Math.floor(Date.now() / 1000);
      const text = `${title} ${description}`.toLowerCase();
      // 简化评分：痛点关键词命中数 × 权重，保证与扫描管线「有分」的观感一致
      const painHits = ["难", "痛点", "慢", "费", "麻烦", "need", "frustrating", "difficult", "lack", "missing"]
        .filter((k) => text.includes(k)).length;
      const painScore = Math.min(100, 40 + painHits * 12);
      const budgetMin = typeof input?.budgetMin === "number" ? input.budgetMin : null;
      const budgetMax = typeof input?.budgetMax === "number" ? input.budgetMax : null;
      // 有预算信号 → 预算分高；无预算 → 中性
      const budgetScore = budgetMin !== null || budgetMax !== null ? 75 : 40;
      const commercialValueScore = Math.round(painScore * 0.4 + budgetScore * 0.3 + 50 * 0.3);
      const row: DemandLead = {
        id: `lead-manual-${now}`,
        platform: "manual",
        title,
        description,
        budgetMin,
        budgetMax,
        budgetCurrency: typeof input?.budgetCurrency === "string" && input.budgetCurrency
          ? input.budgetCurrency
          : "CNY",
        contactName: typeof input?.contactName === "string" && input.contactName
          ? input.contactName
          : null,
        contactEmail: typeof input?.contactEmail === "string" && input.contactEmail
          ? input.contactEmail
          : null,
        contactPhone: typeof input?.contactPhone === "string" && input.contactPhone
          ? input.contactPhone
          : null,
        sourceUrl: typeof input?.sourceUrl === "string" && input.sourceUrl
          ? input.sourceUrl
          : null,
        status: "new",
        confidence: budgetMin !== null || budgetMax !== null ? 0.7 : 0.4,
        painScore,
        marketGapScore: 50,
        commercialValueScore,
        demandType: "custom_development",
        linkedWorkflowId: null,
        implementedAt: null,
        createdAt: now,
        updatedAt: now,
      };
      MOCK_DEMAND_LEADS.push(row);
      return { ...row } as unknown as T;
    }

    // ── OPC 需求订阅（v133 定时扫描）────────────────────────────────
    case "opc_list_subscriptions":
      return [...MOCK_DEMAND_SUBSCRIPTIONS] as unknown as T;

    case "opc_save_subscription": {
      const { input } = (args ?? {}) as { input?: Record<string, unknown> };
      const now = Math.floor(Date.now() / 1000);
      const id = (input?.id as string | undefined) ?? "";
      const existing = MOCK_DEMAND_SUBSCRIPTIONS.find((s) => s.id === id);
      if (existing) {
        if (typeof input?.keyword === "string") {
          existing.keyword = input.keyword;
        }
        if (typeof input?.enabled === "boolean") {
          existing.enabled = input.enabled;
        }
        if (typeof input?.intervalHours === "number") {
          existing.intervalHours = input.intervalHours;
        }
        if (typeof input?.minScore === "number") {
          existing.minScore = input.minScore;
        }
        if (Array.isArray(input?.platforms)) {
          existing.platforms = input.platforms as string[];
        }
        existing.updatedAt = now;
        return { ...existing } as unknown as T;
      }
      const keyword = (input?.keyword as string | undefined)?.trim() ?? "";
      if (!keyword) {
        throw new Error("订阅关键词不能为空");
      }
      if (MOCK_DEMAND_SUBSCRIPTIONS.some((s) => s.keyword === keyword)) {
        throw new Error(`订阅词已存在: ${keyword}`);
      }
      const row: DemandSubscription = {
        id: `sub-${Date.now()}`,
        keyword,
        enabled: input?.enabled === false ? false : true,
        intervalHours: typeof input?.intervalHours === "number" ? input.intervalHours : 6,
        minScore: typeof input?.minScore === "number" ? input.minScore : 60,
        platforms: Array.isArray(input?.platforms) ? (input.platforms as string[]) : [],
        lastScannedAt: null,
        lastHitCount: 0,
        createdAt: now,
        updatedAt: now,
      };
      MOCK_DEMAND_SUBSCRIPTIONS.push(row);
      return { ...row } as unknown as T;
    }

    case "opc_delete_subscription": {
      const { id } = (args ?? {}) as { id?: string };
      const before = MOCK_DEMAND_SUBSCRIPTIONS.length;
      MOCK_DEMAND_SUBSCRIPTIONS = MOCK_DEMAND_SUBSCRIPTIONS.filter((s) => s.id !== id);
      if (MOCK_DEMAND_SUBSCRIPTIONS.length === before) {
        throw new Error(`订阅不存在: ${id}`);
      }
      return undefined as unknown as T;
    }

    case "opc_run_subscription_scan": {
      const { onlyDue } = (args ?? {}) as { onlyDue?: boolean };
      const now = Math.floor(Date.now() / 1000);
      const due = onlyDue === false
        ? MOCK_DEMAND_SUBSCRIPTIONS.filter((s) => s.enabled)
        : MOCK_DEMAND_SUBSCRIPTIONS.filter((s) =>
          s.enabled
          && (s.lastScannedAt === null
            || now - s.lastScannedAt >= s.intervalHours * 3600)
        );
      const outcomes = due.map((s) => {
        const hits = MOCK_DEMAND_LEADS.filter((l) => l.commercialValueScore >= s.minScore).slice(0, 2);
        s.lastScannedAt = now;
        s.lastHitCount = hits.length;
        s.updatedAt = now;
        return {
          subscriptionId: s.id,
          keyword: s.keyword,
          ok: true,
          error: null,
          hits,
        };
      });
      return {
        scannedSubscriptions: due.length,
        totalSaved: outcomes.length,
        totalRefreshed: 0,
        highValueHits: outcomes.reduce((n, o) => n + o.hits.length, 0),
        outcomes,
      } as unknown as T;
    }

    case "opc_ensure_demand_scan_job": {
      const { cronExpression } = (args ?? {}) as { cronExpression?: string };
      const cron = cronExpression ?? "0 */6 * * *";
      // 5 字段校验（与后端 validate_cron_expression 同规则）
      if (cron.trim().split(/\s+/).length !== 5) {
        throw new Error(`非法的 cron 表达式: ${cron}`);
      }
      return {
        id: "mock-opc-demand-scan",
        name: "OPC 需求订阅扫描",
        description: "按订阅词表扫描需求平台，命中推送门槛的线索走 delivery 推送",
        schedule: cron,
        enabled: true,
        status: "active",
        taskType: "opc_demand_scan",
        recurring: true,
        workflowId: null,
        createdAt: Math.floor(Date.now() / 1000),
        lastRunAt: null,
        nextRunAt: null,
        runCount: 0,
      } as unknown as T;
    }

    case "opc_match_lead_capabilities": {
      // 能力匹配 mock：与后端 opc_demand_capability 同口径（required 映射 + 门槛判定），
      // 检索侧用固定假候选模拟（浏览器模式无 RAG）
      const { leadId, topK } = (args ?? {}) as { leadId?: string; topK?: number };
      const lead = MOCK_DEMAND_LEADS.find((l) => l.id === leadId);
      if (!lead) { throw new Error(`线索不存在: ${leadId}`); }
      const limit = Math.min(Math.max(topK ?? 8, 1), 20);

      // 需求类型 → 必需能力域（与后端 required_domains_for 保持一致）
      const REQUIRED: Record<string, string[]> = {
        tool_software: ["general", "automation"],
        content_creation: ["content_creation", "ai_media"],
        design: ["content_creation", "ai_media"],
        development: ["devops", "general"],
        operations: ["automation", "devops"],
        marketing: ["content_creation", "communication"],
        education: ["content_creation"],
        enterprise_service: ["automation", "data_analysis"],
        outsourcing: ["general", "automation"],
        consulting: ["data_analysis", "general"],
      };
      const required = REQUIRED[lead.demandType] ?? [];

      // 固定假候选：development 类给「部分覆盖」演示，其余给 ready 演示
      const matches = lead.demandType === "development"
        ? [
          {
            capabilityId: "mock-cap-devops-ci",
            name: "CI/CD 流水线工作流",
            kind: "workflow",
            domain: "devops",
            retrievalScore: 0.71,
            summary: "多平台 API 对接与定时同步任务编排",
          },
        ].slice(0, limit)
        : [
          {
            capabilityId: "mock-cap-content-agent",
            name: "内容聚类摘要 Agent",
            kind: "agent",
            domain: "content_creation",
            retrievalScore: 0.78,
            summary: "按兴趣关键词自动聚类并生成摘要周报",
          },
          {
            capabilityId: "mock-cap-ai-media",
            name: "AI 媒体生成工具链",
            kind: "toolchain",
            domain: "ai_media",
            retrievalScore: 0.66,
            summary: "多模态内容生成与配图",
          },
        ].slice(0, limit);

      const bestScore = matches.length > 0 ? Math.max(...matches.map((m) => m.retrievalScore)) : 0;
      const covered = matches.map((m) => m.domain);
      const missingDomains = required.filter((d) => !covered.includes(d));
      const verdict = bestScore >= 0.65 && missingDomains.length === 0
        ? "ready"
        : bestScore >= 0.4 || (missingDomains.length === 0 && bestScore > 0)
        ? "partial"
        : "missing";

      return {
        leadId: lead.id,
        verdict,
        bestScore,
        matches,
        requiredDomains: required,
        missingDomains,
        gapHint: missingDomains.length > 0
          ? `需求类型 ${lead.demandType} 需要以下能力域，当前能力库未覆盖：${missingDomains.join("、")}`
          : null,
      } as unknown as T;
    }

    case "opc_list_invoices": {
      const { status } = (args ?? {}) as { status?: string };
      return (status ? MOCK_INVOICES.filter((i) => i.status === status) : [...MOCK_INVOICES]) as unknown as T;
    }

    case "opc_create_invoice_from_lead": {
      const { leadId } = (args ?? {}) as { leadId?: string };
      const lead = MOCK_DEMAND_LEADS.find((l) => l.id === leadId);
      if (!lead) { throw new Error(`线索不存在: ${leadId}`); }
      // 与后端同规则：仅 won 线索可开票；已有发票幂等返回
      if (lead.status !== "won") {
        throw new Error(`仅 won 线索可开票，当前状态: ${lead.status}（线索 ${leadId}）`);
      }
      const existing = MOCK_INVOICES.find((i) => i.leadId === leadId);
      if (existing) { return existing as unknown as T; }
      const now = Math.floor(Date.now() / 1000);
      const inv: DeliveryInvoice = {
        id: `inv-${leadId}-${now}`,
        leadId: lead.id,
        linkedWorkflowId: lead.linkedWorkflowId,
        title: lead.title,
        amount: lead.budgetMax ?? lead.budgetMin ?? 0,
        currency: lead.budgetCurrency || "CNY",
        status: "draft",
        issuedAt: null,
        paidAt: null,
        notes: null,
        createdAt: now,
        updatedAt: now,
      };
      MOCK_INVOICES.push(inv);
      return inv as unknown as T;
    }

    case "opc_update_invoice_status": {
      const { invoiceId, status } = (args ?? {}) as {
        invoiceId?: string;
        status?: "draft" | "sent" | "paid";
      };
      const inv = MOCK_INVOICES.find((i) => i.id === invoiceId);
      if (!inv) { throw new Error(`发票不存在: ${invoiceId}`); }
      // 与后端同规则：draft → sent → paid 单向，同状态幂等
      const legal = inv.status === status
        || (inv.status === "draft" && status === "sent")
        || (inv.status === "sent" && status === "paid");
      if (!legal) {
        throw new Error(`非法发票状态迁移: ${inv.status} → ${status}（合法路径 draft → sent → paid）`);
      }
      const now = Math.floor(Date.now() / 1000);
      if (status === "sent") { inv.issuedAt = now; }
      if (status === "paid") {
        inv.issuedAt = inv.issuedAt ?? now;
        inv.paidAt = now;
      }
      inv.status = status ?? inv.status;
      inv.updatedAt = now;
      return inv as unknown as T;
    }

    case "opc_delete_invoice": {
      const { invoiceId } = (args ?? {}) as { invoiceId?: string };
      const idx = MOCK_INVOICES.findIndex((i) => i.id === invoiceId);
      if (idx >= 0) { MOCK_INVOICES.splice(idx, 1); }
      return { ok: true } as unknown as T;
    }

    case "opc_get_delivery_summary": {
      const won = MOCK_DEMAND_LEADS.filter((l) => l.status === "won").length;
      const active = MOCK_DEMAND_LEADS.filter((l) => l.status !== "lost").length;
      const byCurrency = new Map<string, { paidTotal: number; issuedTotal: number }>();
      let paidCount = 0;
      for (const inv of MOCK_INVOICES) {
        const acc = byCurrency.get(inv.currency) ?? { paidTotal: 0, issuedTotal: 0 };
        if (inv.status === "paid") {
          acc.paidTotal += inv.amount;
          acc.issuedTotal += inv.amount;
          paidCount += 1;
        } else if (inv.status === "sent") {
          acc.issuedTotal += inv.amount;
        }
        byCurrency.set(inv.currency, acc);
      }
      return {
        wonLeads: won,
        activeLeads: active,
        invoiceCount: MOCK_INVOICES.length,
        paidCount,
        revenues: [...byCurrency.entries()].map(([currency, v]) => ({ currency, ...v })),
        conversionRate: active === 0 ? 0 : won / active,
      } as unknown as T;
    }

    case "kb_connect_vault":
      return { success: true, vault_id: (args as { vault_id?: string })?.vault_id ?? "" } as T;
    case "kb_disconnect_vault":
      return { success: true } as T;
    case "workflow_execute":
      return { execution_id: "mock-exec", status: "completed", outputs: {} } as T;
    case "workflow_cancel":
      return { success: true } as T;
    case "capability_register_passport": {
      const passport = (args as { request?: { passport?: CapabilityPassportDto } })
        ?.request?.passport;
      if (!passport) {
        return {
          capability_id: "",
          success: false,
          vector_dimensions: 0,
          indexed_at_ms: 0,
          error: "missing passport",
        } as T;
      }
      const passports = readCapabilityPassports();
      const idx = passports.findIndex((p) => p.capabilityId === passport.capabilityId);
      if (idx >= 0) {
        passports[idx] = passport;
      } else {
        passports.push(passport);
      }
      writeCapabilityPassports(passports);
      return indexResultFor(passport, true) as T;
    }
    case "capability_register_batch": {
      const batch = (args as { passports?: CapabilityPassportDto[] })?.passports ?? [];
      const passports = readCapabilityPassports();
      for (const passport of batch) {
        const idx = passports.findIndex((p) => p.capabilityId === passport.capabilityId);
        if (idx >= 0) {
          passports[idx] = passport;
        } else {
          passports.push(passport);
        }
      }
      writeCapabilityPassports(passports);
      return batch.map((p) => indexResultFor(p, true)) as T;
    }
    case "capability_remove_passport": {
      const capabilityId = (args as { capabilityId?: string })?.capabilityId ?? "";
      const passports = readCapabilityPassports();
      writeCapabilityPassports(
        passports.filter((p) => p.capabilityId !== capabilityId),
      );
      return { success: true } as T;
    }
    case "capability_list_passports":
      return readCapabilityPassports() as T;
    case "capability_get_stats":
      return capabilityStatsFrom(readCapabilityPassports()) as T;
    case "capability_discover": {
      const userInput = (args as { request?: { userInput?: string } })?.request
        ?.userInput ?? "";
      return mockDiscover(userInput) as T;
    }
    case "capability_registry_dump":
      // 浏览器模式 mock 能力注册表检视（缺陷 #6：前端插件页消费该命令）
      return [
        {
          id: "agent.loop",
          version: "1.0",
          contract: "axagent_harness::AgentTurnRunner",
          description: "Agent 主循环接缝",
          origin: "builtin",
          pluginId: null,
        },
        {
          id: "model.provider.openai",
          version: "1.0",
          contract: "axagent_harness::ProviderAdapter",
          description: "内置 LLM 提供商适配器：openai",
          origin: "builtin",
          pluginId: null,
        },
        {
          id: "session.log.invariant",
          version: "1.0",
          contract: "axagent_harness::SessionLogInvariant",
          description: "会话日志不变量接缝",
          origin: "builtin",
          pluginId: null,
        },
      ] as T;

    case "get_app_config":
      return {} as T;

    // ── Onboarding (引导检测) ───────────────────────────────────────
    case "detect_ollama_availability":
      return { available: false, models: [], error: null } as T;
    case "detect_api_keys":
      return [] as T;

    // ── LLM Wiki (知识库) ───────────────────────────────────────────
    // 有 fixture 时让 wiki 列表/图谱走真实数据（见 loadWikiGraphFixture）——
    // 否则 `llm_wiki_list` 返回空列表会让图页面直接判定「无可用 wiki」，
    // 图谱渲染层在浏览器模式下完全无法调试。
    case "llm_wiki_list": {
      const fx = await loadWikiGraphFixture();
      return (fx ? [fx.wiki] : []) as T;
    }
    case "get_wiki_graph_cached": {
      const fx = await loadWikiGraphFixture();
      const wikiId = String(args?.wiki_id ?? args?.wikiId ?? "");
      if (fx && (!wikiId || fx.wiki.id === wikiId)) {
        return fx.graph as T;
      }
      return { nodes: [], edges: [] } as T;
    }
    case "wiki_graph_communities_cached": {
      const fx = await loadWikiGraphFixture();
      return (fx ? fx.communitiesResult : { communities: {} }) as T;
    }
    case "wiki_notes_list": {
      const vaultId = String(args?.vault_id ?? "");
      return seedWikiNotes(vaultId) as unknown as T;
    }
    case "wiki_notes_get": {
      const id = String(args?.id ?? "");
      const note = getWikiNotes().find((n) => n.id === id);
      if (!note) { throw new Error(`Note ${id} not found`); }
      return note as unknown as T;
    }
    case "wiki_notes_get_by_path": {
      const vaultId = String(args?.vault_id ?? "");
      const filePath = String(args?.file_path ?? "");
      const note = getWikiNotes().find(
        (n) => n.vaultId === vaultId && n.filePath === filePath,
      );
      if (!note) { throw new Error(`Note ${filePath} not found`); }
      return note as unknown as T;
    }
    case "wiki_notes_search": {
      const vaultId = String(args?.vault_id ?? "");
      const query = String(args?.query ?? "").toLowerCase();
      const results: NoteSearchResult[] = seedWikiNotes(vaultId)
        .filter(
          (n) =>
            !query
            || n.title.toLowerCase().includes(query)
            || n.content.toLowerCase().includes(query),
        )
        .map((n) => ({
          note: n,
          snippet: n.content.slice(0, 120),
          score: 1,
        }));
      return results as unknown as T;
    }
    case "wiki_notes_create": {
      const input = (args?.input ?? {}) as Record<string, unknown>;
      const title = String(input.title ?? "未命名");
      const ts = nowTs();
      const note: Note = {
        id: genId(),
        vaultId: String(input.vault_id ?? ""),
        title,
        filePath: String(input.file_path ?? `${title}.md`),
        content: String(input.content ?? ""),
        contentHash: "",
        author: String(input.author ?? "user"),
        pageType: input.page_type !== undefined ? String(input.page_type) : undefined,
        sourceRefs: Array.isArray(input.source_refs)
          ? (input.source_refs as string[])
          : undefined,
        userEdited: true,
        createdAt: ts,
        updatedAt: ts,
        isDeleted: false,
      };
      setWikiNotes([...getWikiNotes(), note]);
      return note as unknown as T;
    }
    case "wiki_notes_update": {
      const id = String(args?.id ?? "");
      const input = (args?.input ?? {}) as Record<string, unknown>;
      const notes = getWikiNotes();
      const idx = notes.findIndex((n) => n.id === id);
      if (idx < 0) { throw new Error(`Note ${id} not found`); }
      const updated: Note = {
        ...notes[idx],
        title: input.title !== undefined ? String(input.title) : notes[idx].title,
        content: input.content !== undefined ? String(input.content) : notes[idx].content,
        pageType: input.page_type !== undefined
          ? String(input.page_type)
          : notes[idx].pageType,
        relatedPages: Array.isArray(input.related_pages)
          ? (input.related_pages as string[])
          : notes[idx].relatedPages,
        userEdited: true,
        userEditedAt: nowTs(),
        updatedAt: nowTs(),
      };
      const next = [...notes];
      next[idx] = updated;
      setWikiNotes(next);
      return updated as unknown as T;
    }
    case "wiki_notes_delete": {
      const id = String(args?.id ?? "");
      setWikiNotes(getWikiNotes().filter((n) => n.id !== id));
      return undefined as T;
    }
    case "wiki_note_create_from_template":
    case "wiki_create_daily_note": {
      const vaultId = String(args?.vault_id ?? "");
      const isDaily = cmd === "wiki_create_daily_note";
      const title = isDaily
        ? new Date().toISOString().slice(0, 10)
        : String(args?.title ?? "新笔记");
      const note = mockWikiNote(
        vaultId,
        title,
        isDaily ? `# ${title}\n\n` : "# 新笔记\n\n",
        [],
      );
      setWikiNotes([...getWikiNotes(), note]);
      return note as unknown as T;
    }
    case "wiki_template_create": {
      const input = (args?.input ?? {}) as Record<string, unknown>;
      const ts = nowTs();
      const tpl: WikiTemplate = {
        id: genId(),
        wikiId: String(input.wiki_id ?? ""),
        name: String(input.name ?? "模板"),
        description: input.description !== undefined ? String(input.description) : undefined,
        content: String(input.content ?? ""),
        pageType: input.page_type !== undefined ? String(input.page_type) : undefined,
        isBuiltin: Boolean(input.is_builtin),
        createdAt: ts,
        updatedAt: ts,
      };
      return tpl as unknown as T;
    }

    // ── Prompt Cache (提示缓存) ────────────────────────────────────
    case "get_prompt_cache_state":
      return {
        cacheValid: true,
        hasPendingChanges: false,
        tokensSaved: 0,
        cacheHits: 0,
      } as T;

    // ── Tool Count (工具计数) ───────────────────────────────────────
    case "get_tool_count":
      return 0 as T;

    // ── Skill Stats (技能执行统计) ─────────────────────────────────
    case "get_skill_execution_stats":
      return [] as T;

    // ── Background Tasks (后台任务) ────────────────────────────────
    // 显式声明而非依赖 default 分支的 `endsWith("s")` 启发式：那个判断是巧合，
    // 一旦命令改名或启发式调整，浏览器模式会静默变成 `undefined`，
    // 而 TaskPanel 会把它当成「没有任务」。显式 case 是一份契约。
    case "list_background_tasks":
    case "list_task_events":
      return [] as unknown as T;

    // ── 股票管道（stock_pipeline）──────────────────────────────────────
    // 浏览器模式没有真实管道执行。必须显式声明，否则会落到 default 分支的
    // `get_` → `{}` 兜底：`get_pipeline_history` 的消费端按数组使用
    // （`pipelineRuns.filter/.length`），收到 `{}` 会直接抛
    // `TypeError: pipelineRuns.filter is not a function`，
    // 令「投资中心 → 管道」整个 tab 被 ErrorBoundary 兜成「页面错误」。
    case "get_pipeline_history":
      return [] as unknown as T;
    case "get_pipeline_run_detail":
      return null as unknown as T;
    case "run_stock_pipeline":
      return null as unknown as T;

    // 2026-09-19（D1）：`list_stock_analyses` 显式声明，理由有二 ——
    //   ① **消除误导性告警**：不声明会落到 default 并打出
    //      `[BrowserMock] Unhandled command: list_stock_analyses`，读起来像「该命令没实现」，
    //      而它的兜底行为（`list_` 前缀 → `[]`）其实**恰好正确**。审计 D1 时就被这条
    //      warn 误导，一度判定该命令「返回 undefined」。
    //   ② **让语义显式**：浏览器模式**确实没有**历史决策数据，返回空数组是**如实声明**，
    //      而不是「未处理所以给个默认值」。将来补真实 mock 数据时，落点就在此。
    case "list_stock_analyses":
      return [] as unknown as T;

    // 2026-09-19（D1 配套）：`get_stock_analysis` 必须显式声明，否则会落到 default
    // 分支的 `get_` → `{}` 兜底，而 `{}` **不含 `id`** —— 新增的 IPC 契约校验
    // （`src/lib/ipc-schemas.ts` 的 `get_stock_analysis: z.object({ id: z.string() })`）
    // 会据此判违约并抛 `IpcSchemaError`，把原先「静默无数据」变成「抛错」。
    //
    // 语义：浏览器模式没有任何可复算的历史决策 ⇒ 本处**只保证形状合法**（过契约下限），
    // **不伪造内容** —— 除 `id` 外所有字段缺席，消费端仍按空数据处理（与改动前一致）。
    // 将来若在 mock 层补上真实历史数据，须同时补内容，否则会造出「有 id 但没内容」的
    // 半截对象，比返回 `{}` 更难排查。
    //
    // 注：`list_stock_analyses` 无需显式声明 —— default 分支的 `list_` 前缀规则
    // 已返回 `[]`，与 `z.array(...)` 契约相符。
    case "get_stock_analysis":
      return {
        id: String((args as { analysis_id?: unknown } | undefined)?.analysis_id ?? ""),
      } as unknown as T;

    // 浏览器模式没有真实 FTS5 子系统。返回 `available: false` 而不是编造计数：
    // 若落到 default 分支的 `get_*` → `{}`，会渲染成「未挂载」但丢失原因；
    // 若返回假的非零计数，则是在 mock 层伪造「索引健康」这一结论。
    case "get_memory_fts_health":
      return {
        available: false,
        unavailableReason: "浏览器模式（localStorage mock）无 FTS5 子系统",
        tablesExist: false,
        sourceRows: 0,
        indexedRows: 0,
        laggingRows: 0,
        needsRebuild: false,
        trajectoriesCount: 0,
        skillsCount: 0,
        messagesCount: 0,
      } as unknown as T;

    // ── 能力域覆盖层（P2）─────────────────────────────────────────────
    // 必须**显式**声明这两条：`update_capability_domain` 落到 default 分支会返回
    // `undefined`（它不以 `list_`/`get_` 开头、也不以 `s` 结尾），而面板的消费端
    // 是 `entries.map(e => e.id === updated.id ? …)` ⇒ 直接抛
    // `TypeError: Cannot read properties of undefined`，把整个设置页
    // 兜成 ErrorBoundary 的「页面错误」。这是 mock 层最容易漏的一类坑：
    // 「命令没接线」在 mock 里的症状不是「没反应」，而是**消费端崩**。
    //
    // ⚠ 已知偏差（不在本次范围内）：上面的 `mockCognitiveRoute` 用的
    //   `domainRules` 是另一份**手抄**域表，且不读本覆盖层 —— 即浏览器模式下
    //   「停用某域」不会改变 L1 路由 mock 的结果。该副本已被门禁登记为报告项，
    //   属既有边界，此处不顺手改（改它等于顺手改判据的对象）。
    case "list_capability_domain_registry":
      return mockCapabilityDomainRegistry() as unknown as T;
    case "update_capability_domain": {
      const request = (args as {
        request?: { domain?: string; enabled?: boolean; extra_aliases?: string[] };
      }).request ?? {};
      const domain = request.domain ?? "";
      const rows = mockCapabilityDomainRegistry();
      const row = rows.find((r) => r.id === domain);
      // 错误必须带**码**（JSON 塞进 Error.message）——`parseBackendError` 明确支持
      // 这一形态，故浏览器模式与桌面模式走同一套 `error.${code}` 翻译。
      // 只抛 `new Error("某中文文案")` 会让浏览器模式丢失本地化能力。
      if (!row) {
        throw new Error(JSON.stringify({
          code: "CAPABILITY_DOMAIN_UNKNOWN",
          category: "validation",
          detail: domain,
        }));
      }
      if (request.enabled === false && !row.toggleable) {
        throw new Error(JSON.stringify({
          code: "CAPABILITY_DOMAIN_NOT_TOGGLEABLE",
          category: "validation",
          detail: row.toggle_block_reason ?? "",
        }));
      }
      const prev = MOCK_DOMAIN_OVERRIDES[domain];
      // 局部更新语义与后端一致：未提供的字段保持原值；写一次即产生覆盖行。
      MOCK_DOMAIN_OVERRIDES[domain] = {
        enabled: request.enabled ?? prev?.enabled ?? true,
        extra_aliases: request.extra_aliases ?? prev?.extra_aliases ?? [],
      };
      const updated = mockCapabilityDomainRegistry().find((r) => r.id === domain);
      return updated as unknown as T;
    }

    default: {
      console.warn(`[BrowserMock] Unhandled command: ${cmd}`, args);
      // SAFE: browser mock fallback for unhandled commands — returns empty placeholder matching generic T
      if (cmd.startsWith("list_") || cmd.endsWith("_list") || cmd.includes("_list_") || cmd.endsWith("s")) {
        return [] as unknown as T;
      }
      if (cmd.startsWith("get_")) {
        return {} as unknown as T;
      }
      return undefined as T;
    }
  }
}
