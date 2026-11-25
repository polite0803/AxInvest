#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// Bot P1/P2-a i18n **必需 key 集合**门禁（2026-11-22 建，2026-11-23 登记 P2-a 29 键）。
//
// ## 补的是哪一格
//
// `check-i18n-key-parity.mjs` 只比较「11 个语言的 key 集合是否彼此相等」。
// 如果某个 Bot key **在 11 个语言里一起漏掉**，集合仍然相等 ⇒ 全绿；运行时
// `t("bots.save")` 回退成字面 key 或 zh-CN，用户界面出现 `bots.save`。
// 本脚本把 `PLAN.md` T3 的 key 表变成**必需集合**：少任何一个都红。
//
// ## 判据
//
// 1. `BOT_I18N_KEYS` 全部存在于 11 个 locale（点路径展开后是非空字符串）。
// 2. `bots.deleteConfirm` 的 zh-CN 文案必须含「线程与记忆会保留」（ADR-009：
//    删除 Bot 不删线程与记忆，确认框必须讲清楚）。
// 3. locale 文件数必须是 11（少一个语言文件也算红）。
//
// ## 为什么带 selftest
//
// 「检查器把比较面写窄 ⇒ 恒判通过」是本仓库反复出现的假绿形态。selftest 在
// 临时目录里造 11 份完整字典，再逐项破坏，要求每一处都变红；正控要求 0 错。
//
// 用法：
//   node scripts/check-bot-i18n-keys.mjs
//   node scripts/check-bot-i18n-keys.mjs --dir=/tmp/locales   # 对副本验证
//   node scripts/check-bot-i18n-keys.mjs --selftest
//
// 退出码：0 = 全部齐备；1 = 缺键 / 空值 / 文案不合规 / 自检失败。

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_DIR = path.join(ROOT, "src", "i18n", "locales");
const EXPECTED_LOCALES = 11;

/**
 * `PLAN.md` T3「i18n（11 语言）」表的完整展开（含 `nav.bots`）。
 *
 * ⚠ 计数说明：PLAN 表头写「共 42 个 key」，但逐行相加是 46（1+7+14+9+6+9）。
 * 门禁以**逐行列出的 key** 为准（漏掉文档里写出的任何一个都是缺陷）；数量差异
 * 已作为文档问题上报，不在本脚本里猜测「哪 4 个不算」。
 */
export const BOT_I18N_KEYS = [
  // 导航（1）
  "nav.bots",
  // 页面（7）
  "bots.title",
  "bots.newBot",
  "bots.createTitle",
  "bots.editTitle",
  "bots.empty",
  "bots.emptyDescription",
  "bots.createFirst",
  // 表单（14）
  "bots.name",
  "bots.namePlaceholder",
  "bots.nameRequired",
  "bots.role",
  "bots.rolePlaceholder",
  "bots.soul",
  "bots.soulPlaceholder",
  "bots.model",
  "bots.modelPlaceholder",
  "bots.workdir",
  "bots.workdirPlaceholder",
  "bots.avatar",
  "bots.approvalMode",
  "bots.approvalModeP2Hint",
  // 动作（9）
  "bots.save",
  "bots.saved",
  "bots.saveFailed",
  "bots.createSuccess",
  "bots.createFailed",
  "bots.delete",
  "bots.deleteConfirm",
  "bots.deleteSuccess",
  "bots.deleteFailed",
  // 线程（6）
  "bots.allBots",
  "bots.unassigned",
  "bots.threads",
  "bots.newThread",
  "bots.threadsEmpty",
  "bots.threadsEmptyDescription",
  // 记忆 / SOUL（9）
  "bots.soulTitle",
  "bots.soulEmpty",
  "bots.soulLoadFailed",
  "bots.memory",
  "bots.memoryTitle",
  "bots.memoryEmpty",
  "bots.memoryLoadFailed",
  "bots.memoryBudget",
  "bots.memoryBudgetHint",
  // ── P2-a T4' 新增（2026-11-23，权威清单 docs/bot-integration/P2/I18N-KEYS-P2.md §1）──
  // 例程区（18）
  "bots.tabRoutines",
  "bots.routineAdd",
  "bots.routineName",
  "bots.routineNameRequired",
  "bots.routinePrompt",
  "bots.routinePromptRequired",
  "bots.routineCron",
  "bots.routineCronHint",
  "bots.routineCronInvalid",
  "bots.routineTimezone",
  "bots.routineTimezoneLocal",
  "bots.routineTimezoneInvalid",
  "bots.routineEmpty",
  "bots.routineEmptyDescription",
  "bots.routineNextRun",
  "bots.routineLastRun",
  "bots.routinePause",
  "bots.routineResume",
  "bots.routineDelete",
  "bots.routineDeleteConfirm",
  "bots.routineSaveFailed",
  "bots.routineBadge",
  // 通知（4）
  "notification.filterByBot",
  "notification.allBots",
  "notification.viewThread",
  "notification.unreadCount",
  // 审批生效级别（2）+ 角标（1）
  "bots.approvalEffective",
  "bots.approvalEffectiveHint",
  "bots.unreadBadge",
  // ── P2-b T5'' 新增（2026-11-23，权威清单 docs/bot-integration/P2/I18N-KEYS-P2B.md §2）──
  // 部门（13，含 ADR-P2B-1 区分文案 hint）
  "bot.department.title",
  "bot.department.untitled",
  "bot.department.create",
  "bot.department.edit",
  "bot.department.name",
  "bot.department.nameRequired",
  "bot.department.description",
  "bot.department.chief",
  "bot.department.chiefPlaceholder",
  "bot.department.chiefEmpty",
  "bot.department.addMember",
  "bot.department.removeMember",
  "bot.department.hint",
  // 委派（14）
  "bot.assignment.title",
  "bot.assignment.create",
  "bot.assignment.assignee",
  "bot.assignment.task",
  "bot.assignment.taskRequired",
  "bot.assignment.modeDispatch",
  "bot.assignment.send",
  "bot.assignment.cancel",
  "bot.assignment.statusQueued",
  "bot.assignment.statusRunning",
  "bot.assignment.statusCompleted",
  "bot.assignment.statusFailed",
  "bot.assignment.statusCancelled",
  "bot.assignment.empty",
  // ── P3 T6''' 新增（2026-11-23，权威清单 docs/bot-integration/P3/I18N-KEYS-P3.md §2）──
  "bot.workflow.title",
  "bot.workflow.new",
  "bot.workflow.edit",
  "bot.workflow.empty",
  "bot.workflow.emptyDescription",
  "bot.workflow.createFirst",
  "bot.workflow.name",
  "bot.workflow.nameRequired",
  "bot.workflow.description",
  "bot.workflow.ownerScope",
  "bot.workflow.ownerInstance",
  "bot.workflow.ownerDepartment",
  "bot.workflow.department",
  "bot.workflow.departmentPlaceholder",
  "bot.workflow.steps",
  "bot.workflow.addStep",
  "bot.workflow.removeStep",
  "bot.workflow.stepKey",
  "bot.workflow.assignee",
  "bot.workflow.prompt",
  "bot.workflow.dependsOn",
  "bot.workflow.timeoutSecs",
  "bot.workflow.onFailure",
  "bot.workflow.onFailureAbort",
  "bot.workflow.onFailureContinue",
  "bot.workflow.maxConcurrency",
  "bot.workflow.totalTimeoutSecs",
  "bot.workflow.save",
  "bot.workflowRun.title",
  "bot.workflowRun.run",
  "bot.workflowRun.empty",
  "bot.workflowRun.statusQueued",
  "bot.workflowRun.statusRunning",
  "bot.workflowRun.statusWaiting",
  "bot.workflowRun.statusCompleted",
  "bot.workflowRun.statusFailed",
  "bot.workflowRun.statusCancelled",
  "bot.workflowRun.progress",
  "bot.workflowRun.outputs",
  "bot.workflowRun.skipped",
  "bot.workflowRun.cancel",
  "bot.workflowRun.viewThread",
  "bot.workflowSchedule.title",
  "bot.workflowSchedule.create",
  "bot.workflowSchedule.cron",
  "bot.workflowSchedule.timezone",
  "bot.workflowSchedule.hint",
  "bot.assignment.depth",
  "bot.assignment.chain",
  "bot.quota.title",
  "bot.quota.subtitle",
  "bot.quota.inheritHint",
  "bot.quota.maxConcurrency",
  "bot.quota.maxConcurrencyHint",
  "bot.quota.totalTimeout",
  "bot.quota.totalTimeoutHint",
  "bot.quota.maxSteps",
  "bot.quota.maxStepsHint",
  "bot.quota.perDepth",
  "bot.quota.perDepthHint",
  "bot.quota.stepTimeout",
  "bot.quota.stepTimeoutHint",
  "bot.quota.maxRuns",
  "bot.quota.maxRunsHint",
  "bot.quota.save",
  "bot.quota.reset",
  "bot.quota.resetConfirm",
  "bot.quota.invalidRange",
  "bot.quota.saved",
  "bot.input.pendingTitle",
  "bot.input.pendingBadge",
  "bot.input.question",
  "bot.input.placeholder",
  "bot.input.submit",
  "bot.input.cancel",
  "bot.input.cancelConfirm",
  "bot.input.expiresIn",
  "bot.input.expired",
  "bot.input.expiredHint",
  "bot.input.answered",
  "bot.input.cancelled",
  "bot.input.answerFailed",
  "bot.input.empty",
  // ── P5 T7'''' 新增（2026-11-25，权威清单 docs/bot-integration/P5/I18N-KEYS-P5.md §2）──
  "bot.usage.title",
  "bot.usage.subtitle",
  "bot.usage.estimated",
  "bot.usage.estimatedHint",
  "bot.usage.pricingSource",
  "bot.usage.inputTokens",
  "bot.usage.outputTokens",
  "bot.usage.cacheCreationTokens",
  "bot.usage.cacheReadTokens",
  "bot.usage.totalTokens",
  "bot.usage.totalCost",
  "bot.usage.quotaTitle",
  "bot.usage.quotaHint",
  "bot.usage.breached",
  "bot.usage.window",
  "bot.usage.empty",
  "bot.usage.refresh",
  "bot.skills.title",
  "bot.skills.subtitle",
  "bot.skills.globalSource",
  "bot.skills.botSource",
  "bot.skills.shadowedGlobal",
  "bot.skills.missing",
  "bot.skills.enabled",
  "bot.skills.disabled",
  "bot.skills.empty",
  "bot.skills.importPreset",
  "bot.skills.presetName",
  "bot.skills.presetImported",
  "bot.skills.presetSkipped",
  "bot.skills.presetInvalid",
  "bot.skills.whitelist",
];

/** ADR-009：删除确认必须说明线程与记忆会保留。 */
const DELETE_CONFIRM_PHRASE = "线程与记忆会保留";

function leafValue(json, dottedKey) {
  let node = json;
  for (const part of dottedKey.split(".")) {
    if (node === null || typeof node !== "object" || !(part in node)) {
      return { present: false, value: undefined };
    }
    node = node[part];
  }
  return { present: true, value: node };
}

/** 纯函数：对一组 locale 文件内容做判定。 */
export function auditLocales(files) {
  const errors = [];
  if (files.length !== EXPECTED_LOCALES) {
    errors.push(`locale 文件数是 ${files.length}，期望 ${EXPECTED_LOCALES}`);
  }
  for (const { name, json } of files) {
    for (const key of BOT_I18N_KEYS) {
      const { present, value } = leafValue(json, key);
      if (!present) {
        errors.push(`${name}: 缺 key \`${key}\``);
      } else if (typeof value !== "string" || value.trim() === "") {
        errors.push(`${name}: \`${key}\` 必须是非空字符串（实得 ${JSON.stringify(value)}）`);
      }
    }
  }
  const zh = files.find((f) => f.name === "zh-CN.json") ?? files.find((f) => f.name === "zh-CN");
  if (zh) {
    const confirm = leafValue(zh.json, "bots.deleteConfirm").value;
    if (typeof confirm === "string" && confirm && !confirm.includes(DELETE_CONFIRM_PHRASE)) {
      errors.push(`zh-CN: bots.deleteConfirm 必须含「${DELETE_CONFIRM_PHRASE}」（ADR-009）`);
    }
  }
  return errors;
}

function readLocales(dir) {
  const files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort();
  return files.map((name) => {
    let json = {};
    try {
      json = JSON.parse(fs.readFileSync(path.join(dir, name), "utf8"));
    } catch (e) {
      json = {};
      json.__parseError = String(e);
    }
    return { name, json };
  });
}

function buildLocales() {
  const names = ["zh-CN.json", "zh-TW.json", "en-US.json", "ja.json", "ko.json", "de.json", "fr.json", "es.json", "ru.json", "ar.json", "hi.json"];
  return names.map((name) => {
    const json = {};
    for (const key of BOT_I18N_KEYS) {
      const parts = key.split(".");
      let node = json;
      for (let i = 0; i < parts.length - 1; i += 1) {
        node[parts[i]] ??= {};
        node = node[parts[i]];
      }
      node[parts[parts.length - 1]] = name === "zh-CN.json" && key === "bots.deleteConfirm"
        ? `删除这个机器人？${DELETE_CONFIRM_PHRASE}`
        : `${name}:${key}`;
    }
    return { name, json };
  });
}

function runSelftest() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "bot-i18n-selftest-"));
  const cases = [];
  const write = (files) => {
    for (const f of fs.readdirSync(tmp)) {
      fs.rmSync(path.join(tmp, f));
    }
    for (const { name, json } of files) {
      fs.writeFileSync(path.join(tmp, name), JSON.stringify(json, null, 2));
    }
  };
  const check = (label, files, pattern) => {
    write(files);
    const errors = auditLocales(files);
    const pass = pattern === null ? errors.length === 0 : errors.some((e) => pattern.test(e));
    cases.push({ label, pass, detail: pass ? "" : JSON.stringify(errors.slice(0, 5)) });
  };

  check("正控：11 语言完整 ⇒ 0 错", buildLocales(), null);

  {
    const files = buildLocales();
    delete files.find((f) => f.name === "ko.json").json.bots.save;
    check("ko 缺 bots.save", files, /ko\.json: 缺 key `bots\.save`/);
  }
  {
    const files = buildLocales();
    files.find((f) => f.name === "en-US.json").json.bots.memoryBudget = "  ";
    check("en-US 的 bots.memoryBudget 为空", files, /en-US\.json: `bots\.memoryBudget` 必须是非空字符串/);
  }
  {
    const files = buildLocales();
    delete files.find((f) => f.name === "fr.json").json.bots;
    check("fr 整个 bots 段缺失", files, /fr\.json: 缺 key `bots\.title`/);
  }
  {
    const files = buildLocales();
    delete files.find((f) => f.name === "de.json").json.nav.bots;
    check("de 缺 nav.bots", files, /de\.json: 缺 key `nav\.bots`/);
  }
  {
    const files = buildLocales();
    files.find((f) => f.name === "zh-CN.json").json.bots.deleteConfirm = "确定删除？";
    check("zh-CN 删除确认丢了保留语义", files, /bots\.deleteConfirm 必须含「线程与记忆会保留」/);
  }
  {
    const files = buildLocales().filter((f) => f.name !== "hi.json");
    check("少一个语言文件", files, /locale 文件数是 10，期望 11/);
  }

  fs.rmSync(tmp, { recursive: true, force: true });

  const failed = cases.filter((c) => !c.pass);
  console.log("=== check-bot-i18n-keys 自检（逐项破坏必须变红）===");
  for (const c of cases) {
    console.log(`  ${c.pass ? "✓" : "✗"} ${c.label}${c.pass ? "" : `\n      ${c.detail}`}`);
  }
  console.log(`\n自检：${cases.length - failed.length} 通过 / ${failed.length} 失败`);
  return failed.length === 0;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--selftest")) {
    process.exit(runSelftest() ? 0 : 1);
  }
  const dirArg = argv.find((a) => a.startsWith("--dir="));
  const dir = dirArg ? dirArg.slice("--dir=".length) : DEFAULT_DIR;
  if (!fs.existsSync(dir)) {
    console.error(`check-bot-i18n-keys: 找不到 locales 目录：${dir}`);
    process.exit(1);
  }
  const files = readLocales(dir);
  const errors = auditLocales(files);
  console.log("=== Bot P1 i18n 必需 key ===");
  console.log(`目录: ${dir}`);
  console.log(`语言: ${files.length}  必需 key: ${BOT_I18N_KEYS.length}`);
  if (errors.length) {
    console.log(`\n缺失/不合规 (${errors.length}):`);
    errors.slice(0, 60).forEach((e) => console.log(`  ${e}`));
    if (errors.length > 60) {
      console.log(`  ... 另有 ${errors.length - 60} 条`);
    }
    console.log("\n结果: FAIL");
    process.exit(1);
  }
  console.log("结果: PASS（11 语言 × 全部必需 key 齐备）");
}

main();
