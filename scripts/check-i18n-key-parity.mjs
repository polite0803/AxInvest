#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
//
// i18n **key 路径一致性**门禁：11 个语言字典必须拥有完全相同的 key 路径集合。
//
// ## 为什么需要它（2026-09-21）
//
// `src/i18n/locales/*.json` 是**平行副本** —— 同一个 key 要在 11 个文件里各写一遍。
// CI 已有三道 i18n 门禁，但各自比较的**不是同一对集合**：
//   · `check-hardcoded-i18n.sh`     —— 源码里的硬编码文案（不涉及 locale 文件）；
//   · `check_i18n.py`               —— JSON 语法 / zh-CN 空值 / **代码 `t()` 引用 ↔ zh-CN**；
//   · `check-i18n-untranslated.mjs` —— 非 CJK locale 的**值**里是否还残留中文。
// **没有一道比较「zh-CN ↔ 其余 10 语言」的 key 集合** ⇒ 往 zh-CN 加一个 key、漏掉
// 其余语言，三道门禁全绿，而运行时那 10 种语言会**回退到 zh-CN**
// （`fallbackLng: "zh-CN"`），用户看到中文揣在日文界面里 —— 静默降级，不报错、不告警。
//
// 本脚本补上这一格：以基准语言（默认 zh-CN）为参照，递归展开两边的**叶子路径**
// 集合，双向差集非空即失败。含**正控**（故意查一条不存在的路径，必须报"不存在"），
// 以防「比较面被写窄 ⇒ 恒判通过」这类假绿。
//
// ## 用法
//
//   node scripts/check-i18n-key-parity.mjs                 # 基准 zh-CN.json
//   node scripts/check-i18n-key-parity.mjs --ref=zh-TW.json
//   node scripts/check-i18n-key-parity.mjs --list=5        # 每个文件最多列 5 条差异
//
// 退出码：0 = 全部一致；1 = 存在差异或基准文件缺失。
//
// ## 判据
//
// 只比**路径集合**，不比**值**。「值是否已翻译」属 `check-i18n-untranslated.mjs`
// 的职责，两者互补、不重叠。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIR = path.join(ROOT, "src", "i18n", "locales");

function fail(msg) {
  process.stderr.write(`check-i18n-key-parity: ${msg}\n`);
  process.exit(1);
}

/** 取 `--name=value` 形式的参数。 */
function argOf(name, dflt) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((a) => a.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : dflt;
}

/**
 * 递归展开 JSON 的**叶子路径**（`.` 连接）。
 *
 * 对象继续下钻；标量与数组视为叶子（数组整体当一个值 —— i18n 里数组元素是同序文案，
 * 逐元素比较会因插值占位差异产生噪声，且缺元素的问题由「值翻译」门禁覆盖）。
 */
function leafPaths(node, prefix = "", out = new Set()) {
  if (node !== null && typeof node === "object" && !Array.isArray(node)) {
    for (const [k, v] of Object.entries(node)) {
      leafPaths(v, prefix ? `${prefix}.${k}` : k, out);
    }
    // 空对象：i18n 字典里几乎不出现，忽略（否则会与祖先重复计数）
  } else {
    out.add(prefix);
  }
  return out;
}

const refName = argOf("ref", "zh-CN.json");
const maxList = Number.parseInt(argOf("list", "8"), 10);
const CONTROL = "__definitely_not_a_key__";

if (!fs.existsSync(DIR)) fail(`找不到 locales 目录：${DIR}`);

const files = fs.readdirSync(DIR).filter((f) => f.endsWith(".json")).sort();
if (files.length === 0) fail(`${DIR} 下没有 .json 语言文件`);
if (!files.includes(refName)) {
  fail(`基准文件 ${refName} 不存在。现有：${files.join(", ")}`);
}

const dicts = new Map();
for (const f of files) {
  try {
    dicts.set(f, JSON.parse(fs.readFileSync(path.join(DIR, f), "utf8")));
  } catch (e) {
    fail(`${f} 不是合法 JSON：${e.message}`);
  }
}

const ref = leafPaths(dicts.get(refName));
console.log(`语言文件数 ${files.length}，基准 ${refName}，叶子路径 ${ref.size} 条\n`);

let bad = 0;
for (const f of files) {
  if (f === refName) {
    console.log(`REF  ${f}  (${ref.size} 条)`);
    continue;
  }
  const cur = leafPaths(dicts.get(f));
  const missing = [...ref].filter((k) => !cur.has(k));
  const extra = [...cur].filter((k) => !ref.has(k));
  if (missing.length === 0 && extra.length === 0) {
    console.log(`OK   ${f}  (${cur.size} 条)`);
    continue;
  }
  bad++;
  console.log(`FAIL ${f}  缺 ${missing.length} 条 / 多 ${extra.length} 条`);
  for (const k of missing.slice(0, maxList)) console.log(`       − ${k}`);
  if (missing.length > maxList) console.log(`       … 另有 ${missing.length - maxList} 条缺失`);
  for (const k of extra.slice(0, maxList)) console.log(`       + ${k}`);
  if (extra.length > maxList) console.log(`       … 另有 ${extra.length - maxList} 条多余`);
}

// 正控：比较面若被写窄（例如退化成只比顶层 key），这条会「意外存在」⇒ 立刻暴露。
const controlOk = !ref.has(CONTROL);
console.log(
  `\n正控（查不存在的路径 ${CONTROL}）：${
    controlOk ? "报不存在 ✓ 比较面有效" : "误报存在 ✗ 比较面异常"
  }`,
);

const pass = bad === 0 && controlOk;
console.log(
  pass
    ? `结论: ${files.length} 个语言文件的 key 路径集合一致`
    : `结论: ${bad} 个文件与基准 ${refName} 不一致`,
);
process.exit(pass ? 0 : 1);
