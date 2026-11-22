// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 浏览器模式（`npm run dev` / Playwright，无 Tauri 后端）下的 Bot IPC mock。
 *
 * ## 为什么单独一个文件
 *
 * `browserMock.ts` 已近 9k 行；Bot 的 9 个命令（`docs/bot-integration/IPC.md` §1）
 * 与既有会话命令共享 localStorage 数据面，但**判定规则**独立（校验、SOUL 镜像、
 * 记忆预算）。集中在本文件，方便门禁与单测直接锁定，也避免与前端正在改的
 * `ChatSidebar` / `botStore` 抢同一个大文件的编辑区。
 *
 * ## 这一层修的是什么缺陷（2026-11-22）
 *
 * Web 模式下会话对象**不写 `botId`**，而 P1 的「按 Bot 过滤线程」读取该字段 ⇒
 * 过滤结果恒空，页面看着像「Bot 没有线程」。本文件与 `browserMock.ts` 的
 * `create_conversation` / `update_conversation` 双侧保证：只要传了 `botId`，
 * 落库（localStorage）就带 `botId`，`list_bot_threads` 与前端过滤都能命中。
 *
 * ## 与真实后端的差异（刻意保留）
 *
 * - 工作目录只做**形态**校验（绝对路径），不访问文件系统（浏览器没有 fs）。
 * - SOUL 镜像用 localStorage 的 `axagent_bot_soul_mirrors` 模拟 `~/.axagent/bots/<id>/SOUL.md`；
 *   `soul_drift` 的判据与 ADR-003 一致：DB 与镜像的 hash 不等 ⇒ drift。
 * - 记忆预算固定 200 行 / 24 KB（与 `ProjectMemory` 同值）。
 */

/** 调度哨兵：`handleBotCommand` 不处理的命令原样返回它，调用方继续走原 switch。 */
export const BOT_UNHANDLED: unique symbol = Symbol("BOT_UNHANDLED");

/** P1 的 9 个 Bot 命令（IPC.md §1），顺序与文档一致。 */
export const BOT_COMMANDS = [
  "list_bots",
  "get_bot",
  "create_bot",
  "update_bot",
  "delete_bot",
  "list_bot_threads",
  "read_bot_soul",
  "write_bot_soul",
  "read_bot_memory",
] as const;

/**
 * P2-a 例程命令（IPC-P2 §2）：2 条 bot 域命令 + 4 条复用的通用 scheduled_task
 * 命令（仅拦截 `task_type === "bot_routine"` 的行，普通定时任务继续走 browserMock
 * 原有分支）。T5' 冻结契约的字面名，不得改名。
 */
export const BOT_ROUTINE_COMMANDS = [
  "create_bot_scheduled_task",
  "list_bot_scheduled_tasks",
  "update_scheduled_task",
  "delete_scheduled_task",
  "pause_scheduled_task",
  "resume_scheduled_task",
] as const;

/** 与后端 `BOT_ROUTINE_TASK_TYPE` 同字面量（scheduled_task.rs）。 */
export const BOT_ROUTINE_TASK_TYPE = "bot_routine" as const;

export const APPROVAL_MODES = ["ask", "edits", "auto", "full", "custom"] as const;

const BOTS_KEY = "bots";
const SOUL_MIRRORS_KEY = "bot_soul_mirrors";
const ROUTINES_KEY = "bot_routines";
const MEMORY_BUDGET = { maxLines: 200, maxBytes: 24 * 1024 } as const;
const SOUL_MAX_BYTES = 64 * 1024;

/**
 * 例程 DTO（浏览器 mock 形态）：键与后端 `ScheduledTaskDto`（camelCase 序列化）
 * 及前端 `ScheduledTask` 类型对齐。真实后端的 next_run_at/last_run_at 来自
 * `CronJobStore` 档期计算；浏览器无调度器，由 mock 用 cron 最小间隔估算，仅保
 * 「非空且可被 `new Date()` 解析」的形态（面板只做展示）。
 */
interface MockRoutine {
  id: string;
  name: string;
  description: string;
  taskType: string;
  cronExpression: string | null;
  intervalSeconds: null;
  nextRunAt: string | null;
  lastRunAt: string | null;
  lastResult: {
    success: boolean;
    output: string | null;
    error: string | null;
    durationMs: number;
    skipped: boolean;
  } | null;
  status: "active" | "paused" | "disabled";
  config: {
    timeoutSeconds: number;
    retryOnFailure: boolean;
    maxRetries: number;
    retryDelaySeconds: number;
    notificationEnabled: boolean;
    runOnStartup: boolean;
  };
  createdAt: string;
  updatedAt: string;
  workflowId: null;
  runningSince: null;
  runningTrigger: null;
  lastTrigger: null;
  botId: string;
  timezone: string | null;
}

interface MockBot {
  id: string;
  name: string;
  title: string | null;
  description: string | null;
  soul: string;
  soulHash: string;
  soulDrift: boolean;
  icon: string;
  color: string | null;
  section: string;
  modelId: string | null;
  providerId: string | null;
  effort: string | null;
  workdir: string | null;
  approvalMode: string;
  agentProfileId: string | null;
  isEnabled: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

interface MockMemoryTopic {
  relativePath: string;
  category: string;
  title: string;
  updatedAt: number;
}

// ── localStorage（与 browserMock 同前缀，键名保持平行）─────────────────

function readStore<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`axagent_${key}`);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeStore(key: string, value: unknown): void {
  try {
    localStorage.setItem(`axagent_${key}`, JSON.stringify(value));
  } catch (e) {
    console.warn(`Failed to write localStorage key: axagent_${key}`, e);
  }
}

// ── 键形兼容（handleCommand 入口会把参数递归 camel→snake）───────────────

function snakeOf(camel: string): string {
  return camel.replace(/[A-Z]/g, (l) => `_${l.toLowerCase()}`);
}

function hasField(obj: unknown, camelKey: string): boolean {
  if (obj === null || typeof obj !== "object") {
    return false;
  }
  const rec = obj as Record<string, unknown>;
  return (
    Object.prototype.hasOwnProperty.call(rec, camelKey)
    || Object.prototype.hasOwnProperty.call(rec, snakeOf(camelKey))
  );
}

function field<T>(obj: unknown, camelKey: string): T | undefined {
  if (obj === null || typeof obj !== "object") {
    return undefined;
  }
  const rec = obj as Record<string, unknown>;
  const value = rec[camelKey] ?? rec[snakeOf(camelKey)];
  return value === undefined ? undefined : (value as T);
}

/** 会话归属键：兼容 camelCase（mock 新写）与 snake_case（旧 fixture / 后端 DTO）。 */
export function conversationBotId(conv: unknown): string | null {
  return field<string | null>(conv, "botId") ?? null;
}

// ── 错误（与后端 `ErrorResponse::err(code)` 同形态）──────────────────────

function botError(code: string): Error {
  return new Error(JSON.stringify({ code, category: "general" }));
}

// ── hash（只用于 drift 判定；不是密码学用途）────────────────────────────

function stableHash(content: string): string {
  let h = 5381;
  for (let i = 0; i < content.length; i += 1) {
    h = ((h << 5) + h + content.charCodeAt(i)) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

function byteLength(content: string): number {
  return new TextEncoder().encode(content).length;
}

// ── Bot 存取 ────────────────────────────────────────────────────────────

function newBotId(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c && typeof c.randomUUID === "function") {
    return c.randomUUID();
  }
  return `bot-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function loadBots(): MockBot[] {
  return readStore<MockBot[]>(BOTS_KEY, []);
}

function saveBots(bots: MockBot[]): void {
  writeStore(BOTS_KEY, bots);
}

function findBot(botId: string): MockBot {
  const bot = loadBots().find((b) => b.id === botId);
  if (!bot) {
    throw botError("BOT_NOT_FOUND");
  }
  return bot;
}

function loadSoulMirrors(): Record<string, string> {
  return readStore<Record<string, string>>(SOUL_MIRRORS_KEY, {});
}

function writeSoulMirror(botId: string, content: string): void {
  const mirrors = loadSoulMirrors();
  mirrors[botId] = content;
  writeStore(SOUL_MIRRORS_KEY, mirrors);
}

function soulView(bot: MockBot) {
  const mirror = loadSoulMirrors()[bot.id];
  const dbHash = bot.soulHash || stableHash(bot.soul);
  const drift = mirror !== undefined && stableHash(mirror) !== dbHash;
  return { content: bot.soul, hash: dbHash, drift };
}

// ── 例程（P2-a，IPC-P2 §2）──────────────────────────────────────────────

function loadRoutines(): MockRoutine[] {
  return readStore<MockRoutine[]>(ROUTINES_KEY, []);
}

function saveRoutines(rows: MockRoutine[]): void {
  writeStore(ROUTINES_KEY, rows);
}

function findRoutine(taskId: string): { rows: MockRoutine[]; idx: number; row: MockRoutine } {
  const rows = loadRoutines();
  const idx = rows.findIndex((r) => r.id === taskId);
  if (idx === -1) {
    // 与后端 update/execute 对未知 id 一致：TASK_NOT_FOUND（pause/resume/delete 静默幂等）。
    throw botError("TASK_NOT_FOUND");
  }
  return { rows, idx, row: rows[idx] };
}

/** cron 第 1 段（分钟）→ 下一个整分时刻（仅保展示形态；真实档期在 CronJobStore）。 */
function nextRunEstimate(cron: string): string | null {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) {
    return null;
  }
  const minute = Number.parseInt(parts[0], 10);
  const d = new Date();
  d.setSeconds(0, 0);
  if (Number.isFinite(minute) && minute >= 0 && minute <= 59) {
    if (d.getMinutes() >= minute) {
      d.setDate(d.getDate() + 1);
    }
    d.setMinutes(minute);
  } else {
    d.setMinutes(d.getMinutes() + 1);
  }
  return d.toISOString();
}

/**
 * IANA 时区校验（GATES-P2 §2 A3 的 JS 侧口径）：`Intl.DateTimeFormat` 可解析即
 * 放行；空 / 缺省 = 宿主 Local（null），与后端 `validate_timezone` 语义一致。
 */
function isValidTimeZoneJs(tz: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz }).format(new Date());
    return true;
  } catch {
    return false;
  }
}

function validateRoutineInput(input: { name?: unknown; prompt?: unknown; cron?: unknown; timezone?: unknown }): void {
  const name = typeof input.name === "string" ? input.name.trim() : "";
  if (!name || Array.from(name).length > 64) {
    throw botError("BOT_NAME_INVALID");
  }
  // 与后端 validate_bot_routine_prompt 同规则：非空（trim 后）。
  if (typeof input.prompt !== "string" || input.prompt.trim().length === 0) {
    throw botError("COMMON_INVALID_INPUT");
  }
  const cron = typeof input.cron === "string" ? input.cron.trim() : "";
  if (cron.split(/\s+/).length !== 5) {
    // 与后端同码：包住 validate_cron_detail 的文案细节。
    throw botError("BOT_SCHEDULE_CRON_INVALID");
  }
  const tz = typeof input.timezone === "string" ? input.timezone.trim() : "";
  if (tz && !isValidTimeZoneJs(tz)) {
    throw botError("BOT_SCHEDULE_TIMEZONE_INVALID");
  }
}

// ── 校验 ────────────────────────────────────────────────────────────────

function normalizeCreateInput(input: Record<string, unknown>): Partial<MockBot> {
  const name = field<string>(input, "name")?.trim() ?? "";
  if (!name || Array.from(name).length > 64) {
    throw botError("BOT_NAME_INVALID");
  }
  const title = field<string | null>(input, "title") ?? null;
  const description = field<string | null>(input, "description") ?? null;
  if ((title && title.length > 120) || (description && description.length > 500)) {
    throw botError("BOT_FIELD_TOO_LONG");
  }
  const soul = field<string>(input, "soul") ?? "";
  if (byteLength(soul) > SOUL_MAX_BYTES) {
    throw botError("BOT_SOUL_TOO_LARGE");
  }
  const approvalMode = field<string>(input, "approvalMode") ?? "full";
  if (!APPROVAL_MODES.includes(approvalMode as (typeof APPROVAL_MODES)[number])) {
    throw botError("BOT_APPROVAL_MODE_INVALID");
  }
  const modelId = field<string | null>(input, "modelId") ?? null;
  const providerId = field<string | null>(input, "providerId") ?? null;
  if ((modelId === null) !== (providerId === null)) {
    throw botError("BOT_MODEL_PAIR_INVALID");
  }
  const workdir = field<string | null>(input, "workdir") ?? null;
  if (workdir !== null && !/^(?:[A-Za-z]:[\\/]|\/|~[\\/])/.test(workdir)) {
    throw botError("BOT_WORKDIR_INVALID");
  }
  const icon = field<string>(input, "icon") ?? "🤖";
  if (Array.from(icon).length > 8) {
    throw botError("BOT_ICON_INVALID");
  }
  return {
    name,
    title,
    description,
    soul,
    icon,
    color: field<string | null>(input, "color") ?? null,
    section: field<string>(input, "section") ?? "default",
    modelId,
    providerId,
    effort: field<string | null>(input, "effort") ?? null,
    workdir,
    approvalMode,
    agentProfileId: field<string | null>(input, "agentProfileId") ?? null,
  };
}

// ── 记忆 ────────────────────────────────────────────────────────────────

function memoryKey(botId: string): string {
  return `bot_memory_${botId}`;
}

function memoryView(botId: string) {
  const stored = readStore<{ indexContent?: string; topics?: MockMemoryTopic[] }>(
    memoryKey(botId),
    {},
  );
  const indexContent = stored.indexContent ?? "";
  return {
    indexContent,
    lineCount: indexContent ? indexContent.split(/\r?\n/).length : 0,
    byteCount: byteLength(indexContent),
    budget: { ...MEMORY_BUDGET },
    topics: stored.topics ?? [],
  };
}

// ── 命令实现 ────────────────────────────────────────────────────────────

export async function handleBotCommand(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<unknown | typeof BOT_UNHANDLED> {
  if (
    !(BOT_COMMANDS as readonly string[]).includes(cmd)
    && !(BOT_ROUTINE_COMMANDS as readonly string[]).includes(cmd)
  ) {
    return BOT_UNHANDLED;
  }
  const a = args ?? {};

  switch (cmd) {
    case "list_bots": {
      const includeDisabled = field<boolean>(a, "includeDisabled") === true;
      return loadBots()
        .filter((b) => includeDisabled || b.isEnabled)
        .sort((x, y) => x.sortOrder - y.sortOrder || x.createdAt - y.createdAt);
    }
    case "get_bot": {
      return findBot(field<string>(a, "botId") ?? "");
    }
    case "create_bot": {
      const input = field<Record<string, unknown>>(a, "input") ?? {};
      const normalized = normalizeCreateInput(input);
      const now = Date.now();
      const bot: MockBot = {
        id: field<string>(input, "id") ?? newBotId(),
        name: normalized.name as string,
        title: normalized.title ?? null,
        description: normalized.description ?? null,
        soul: normalized.soul ?? "",
        soulHash: stableHash(normalized.soul ?? ""),
        soulDrift: false,
        icon: normalized.icon ?? "🤖",
        color: normalized.color ?? null,
        section: normalized.section ?? "default",
        modelId: normalized.modelId ?? null,
        providerId: normalized.providerId ?? null,
        effort: normalized.effort ?? null,
        workdir: normalized.workdir ?? null,
        approvalMode: normalized.approvalMode ?? "full",
        agentProfileId: normalized.agentProfileId ?? null,
        isEnabled: true,
        sortOrder: loadBots().length,
        createdAt: now,
        updatedAt: now,
      };
      const bots = loadBots();
      bots.push(bot);
      saveBots(bots);
      writeSoulMirror(bot.id, bot.soul);
      return bot;
    }
    case "update_bot": {
      const bots = loadBots();
      const idx = bots.findIndex((b) => b.id === (field<string>(a, "botId") ?? ""));
      if (idx === -1) {
        throw botError("BOT_NOT_FOUND");
      }
      const bot = bots[idx];
      const input = field<Record<string, unknown>>(a, "input") ?? {};
      // `Option<Option<T>>` 语义：键缺失 = 不改；键存在且为 null = 显式置空。
      const nullable = [
        "title",
        "description",
        "color",
        "modelId",
        "providerId",
        "effort",
        "workdir",
        "agentProfileId",
      ] as const;
      for (const key of nullable) {
        if (hasField(input, key)) {
          (bot as unknown as Record<string, unknown>)[key] = field<unknown>(input, key) ?? null;
        }
      }
      const name = field<string>(input, "name");
      if (name !== undefined) {
        const trimmed = name.trim();
        if (!trimmed || Array.from(trimmed).length > 64) {
          throw botError("BOT_NAME_INVALID");
        }
        bot.name = trimmed;
      }
      const title = field<string | null>(input, "title");
      const description = field<string | null>(input, "description");
      if ((title && title.length > 120) || (description && description.length > 500)) {
        throw botError("BOT_FIELD_TOO_LONG");
      }
      const soul = field<string>(input, "soul");
      if (soul !== undefined) {
        if (byteLength(soul) > SOUL_MAX_BYTES) {
          throw botError("BOT_SOUL_TOO_LARGE");
        }
        bot.soul = soul;
        bot.soulHash = stableHash(soul);
        writeSoulMirror(bot.id, soul);
      }
      const icon = field<string>(input, "icon");
      if (icon !== undefined) {
        if (Array.from(icon).length > 8) {
          throw botError("BOT_ICON_INVALID");
        }
        bot.icon = icon;
      }
      const section = field<string>(input, "section");
      if (section !== undefined) {
        bot.section = section;
      }
      const approvalMode = field<string>(input, "approvalMode");
      if (approvalMode !== undefined) {
        if (!APPROVAL_MODES.includes(approvalMode as (typeof APPROVAL_MODES)[number])) {
          throw botError("BOT_APPROVAL_MODE_INVALID");
        }
        bot.approvalMode = approvalMode;
      }
      const isEnabled = field<boolean>(input, "isEnabled");
      if (isEnabled !== undefined) {
        bot.isEnabled = isEnabled;
      }
      const sortOrder = field<number>(input, "sortOrder");
      if (sortOrder !== undefined) {
        bot.sortOrder = sortOrder;
      }
      if ((bot.modelId === null) !== (bot.providerId === null)) {
        throw botError("BOT_MODEL_PAIR_INVALID");
      }
      if (bot.workdir !== null && !/^(?:[A-Za-z]:[\\/]|\/|~[\\/])/.test(bot.workdir)) {
        throw botError("BOT_WORKDIR_INVALID");
      }
      bot.updatedAt = Date.now();
      bots[idx] = bot;
      saveBots(bots);
      return bot;
    }
    case "delete_bot": {
      const botId = field<string>(a, "botId") ?? "";
      const bots = loadBots();
      const idx = bots.findIndex((b) => b.id === botId);
      if (idx === -1) {
        throw botError("BOT_NOT_FOUND");
      }
      bots.splice(idx, 1);
      saveBots(bots);
      // ADR-009：线程保留、只解绑；记忆目录与 SOUL 镜像都保留。
      const convs = readStore<Record<string, unknown>[]>("conversations", []);
      let detached = 0;
      for (const conv of convs) {
        if (conversationBotId(conv) === botId) {
          conv.botId = null;
          conv.bot_id = null;
          detached += 1;
        }
      }
      writeStore("conversations", convs);
      // P2-a（GAP G-10 / T2'）：级联删除该 Bot 的例程（非 Bot 例程不受影响）。
      const routines = loadRoutines().filter((r) => r.botId !== botId);
      const removedRoutines = loadRoutines().length - routines.length;
      saveRoutines(routines);
      return { detachedThreads: detached, removedRoutines, memoryPath: `~/.axagent/bots/${botId}` };
    }
    case "list_bot_threads": {
      const botId = field<string>(a, "botId") ?? "";
      const includeArchived = field<boolean>(a, "includeArchived") === true;
      return readStore<Record<string, unknown>[]>("conversations", []).filter((conv) => {
        if (conversationBotId(conv) !== botId) {
          return false;
        }
        const archived = field<boolean>(conv, "isArchived") === true;
        return includeArchived || !archived;
      });
    }
    case "read_bot_soul": {
      return soulView(findBot(field<string>(a, "botId") ?? ""));
    }
    case "write_bot_soul": {
      const botId = field<string>(a, "botId") ?? "";
      const content = field<string>(a, "content") ?? "";
      if (byteLength(content) > SOUL_MAX_BYTES) {
        throw botError("BOT_SOUL_TOO_LARGE");
      }
      const bots = loadBots();
      const idx = bots.findIndex((b) => b.id === botId);
      if (idx === -1) {
        throw botError("BOT_NOT_FOUND");
      }
      // 先写镜像、后写 DB：镜像失败则不落 DB（与 IPC.md §2 的写入顺序一致）。
      writeSoulMirror(botId, content);
      bots[idx].soul = content;
      bots[idx].soulHash = stableHash(content);
      bots[idx].updatedAt = Date.now();
      saveBots(bots);
      return soulView(bots[idx]);
    }
    case "read_bot_memory": {
      return memoryView(findBot(field<string>(a, "botId") ?? "").id);
    }

    // ── P2-a 例程（IPC-P2 §2；通用 scheduled_task 命令只拦截 bot_routine 行）──

    case "create_bot_scheduled_task": {
      const botId = field<string>(a, "botId") ?? "";
      const name = field<string>(a, "name");
      const prompt = field<string>(a, "prompt");
      const cron = field<string>(a, "cronExpression");
      const timezone = field<string | null>(a, "timezone") ?? null;
      validateRoutineInput({ name, prompt, cron, timezone });
      findBot(botId); // BOT_NOT_FOUND 校验（与后端 ensure_bot_exists 同语义）
      const now = new Date().toISOString();
      const row: MockRoutine = {
        id: `bot-routine-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
        name: (name as string).trim(),
        description: prompt as string,
        taskType: BOT_ROUTINE_TASK_TYPE,
        cronExpression: (cron as string).trim(),
        intervalSeconds: null,
        nextRunAt: nextRunEstimate(cron as string),
        lastRunAt: null,
        lastResult: null,
        status: "active",
        config: {
          timeoutSeconds: 300,
          retryOnFailure: false,
          maxRetries: 0,
          retryDelaySeconds: 30,
          notificationEnabled: true,
          runOnStartup: false,
        },
        createdAt: now,
        updatedAt: now,
        workflowId: null,
        runningSince: null,
        runningTrigger: null,
        lastTrigger: null,
        botId,
        timezone: timezone && (timezone as string).trim() ? (timezone as string).trim() : null,
      };
      const rows = loadRoutines();
      rows.push(row);
      saveRoutines(rows);
      return row;
    }

    case "list_bot_scheduled_tasks": {
      const botId = field<string>(a, "botId") ?? "";
      findBot(botId); // IPC-P2 §2.2：Bot 不存在 → BOT_NOT_FOUND
      return loadRoutines()
        .filter((r) => r.botId === botId)
        .sort((x, y) => x.createdAt.localeCompare(y.createdAt));
    }

    case "update_scheduled_task": {
      const taskId = field<string>(a, "taskId") ?? "";
      const { rows, idx, row } = findRoutine(taskId);
      if (row.taskType !== BOT_ROUTINE_TASK_TYPE) {
        return BOT_UNHANDLED; // 普通定时任务：走 browserMock 原分支
      }
      const task = field<Record<string, unknown>>(a, "task") ?? {};
      const name = field<string>(task, "name");
      const prompt = field<string>(task, "description");
      const cron = field<string>(task, "cronExpression");
      validateRoutineInput({
        name: name ?? row.name,
        prompt: prompt ?? row.description,
        cron: cron ?? row.cronExpression ?? "",
        timezone: field<string | null>(a, "timezone") ?? row.timezone,
      });
      if (name !== undefined) {
        row.name = name.trim();
      }
      if (prompt !== undefined) {
        row.description = prompt;
      }
      if (cron !== undefined) {
        row.cronExpression = cron.trim();
        row.nextRunAt = nextRunEstimate(cron);
      }
      // IPC-P2 §2.5：顶层 timezone — undefined=不改；null=清除回落宿主 Local。
      if (hasField(a, "timezone")) {
        const tz = field<string | null>(a, "timezone");
        row.timezone = tz && tz.trim() ? tz.trim() : null;
      }
      row.updatedAt = new Date().toISOString();
      rows[idx] = row;
      saveRoutines(rows);
      return row;
    }

    case "delete_scheduled_task": {
      const taskId = field<string>(a, "taskId") ?? "";
      const { rows, idx, row } = findRoutine(taskId);
      if (row.taskType !== BOT_ROUTINE_TASK_TYPE) {
        return BOT_UNHANDLED;
      }
      rows.splice(idx, 1);
      saveRoutines(rows);
      return undefined;
    }

    case "pause_scheduled_task":
    case "resume_scheduled_task": {
      const taskId = field<string>(a, "taskId") ?? "";
      const { rows, idx, row } = findRoutine(taskId);
      if (row.taskType !== BOT_ROUTINE_TASK_TYPE) {
        return BOT_UNHANDLED;
      }
      row.status = cmd === "pause_scheduled_task" ? "paused" : "active";
      row.updatedAt = new Date().toISOString();
      rows[idx] = row;
      saveRoutines(rows);
      return undefined;
    }

    // delete_bot 级联删例程（GAP G-10 / T2'）：见上方 delete_bot case 内联实现。

    default:
      return BOT_UNHANDLED;
  }
}
