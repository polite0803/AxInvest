/**
 * 修复所有 locale JSON 文件中 stockAnalysis 下的扁平 key → 嵌套结构。
 *
 * 问题：stockAnalysis 对象内，"progress.fetchingData" 等 key 以 JSON 扁平属性名存在，
 * 但同时有空的 "progress": {}。i18next 用 "." 分隔嵌套解析时走入空对象，找不到翻译。
 *
 * 修复：将 stockAnalysis 内所有包含 "." 的扁平 key 转为真正的嵌套对象结构。
 * 例如: "progress.fetchingData" → progress: { fetchingData: ... }
 *
 * 处理顺序：
 *   1. 收集 stockAnalysis 下所有 "." 扁平 key，按第一段分组
 *   2. 对于每个组，先检查是否已有同名的嵌套对象
 *   3. 如果有，合并到嵌套对象中
 *   4. 如果没有，创建嵌套对象
 *   5. 删除扁平 key
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.resolve(__dirname, "../src/i18n/locales");
const files = fs.readdirSync(localesDir).filter(f => f.endsWith(".json"));

/** 递归地将扁平 key 注入到嵌套对象中 */
function setNested(obj, keyPath, value) {
  const parts = keyPath.split(".");
  let current = obj;
  for (let i = 0; i < parts.length - 1; i++) {
    if (!current[parts[i]] || typeof current[parts[i]] !== "object") {
      current[parts[i]] = {};
    }
    current = current[parts[i]];
  }
  current[parts[parts.length - 1]] = value;
}

console.log(`找到 ${files.length} 个 locale 文件\n`);

for (const file of files) {
  const filePath = path.join(localesDir, file);
  const original = fs.readFileSync(filePath, "utf-8");

  let data;
  try {
    data = JSON.parse(original);
  } catch (e) {
    console.log(`  ❌ ${file}: JSON 解析失败: ${e.message}`);
    continue;
  }

  const sa = data.stockAnalysis;
  if (!sa || typeof sa !== "object") {
    console.log(`  ⚠️ ${file}: 无 stockAnalysis 对象，跳过`);
    continue;
  }

  // 1. 收集所有包含 "." 的扁平 key
  const dotKeys = Object.keys(sa).filter(k => k.includes("."));
  if (dotKeys.length === 0) {
    console.log(`  ${file}: 无扁平 key 需要转换`);
    continue;
  }

  // 2. 按第一段分组
  const groups = {};
  for (const key of dotKeys) {
    const prefix = key.split(".")[0];
    if (!groups[prefix]) { groups[prefix] = []; }
    groups[prefix].push(key);
  }

  console.log(`  ${file}: ${dotKeys.length} 个扁平 key (${Object.keys(groups).length} 组)`);

  // 3. 处理每组
  let moved = 0;
  for (const [prefix, keys] of Object.entries(groups)) {
    // 检查是否有同名嵌套对象
    const existing = sa[prefix];
    const existingIsObj = existing !== undefined && existing !== null && typeof existing === "object"
      && !Array.isArray(existing);

    if (!existingIsObj) {
      // 新建对象
      sa[prefix] = {};
    }

    // 注入扁平 key 值到嵌套对象
    for (const flatKey of keys) {
      const innerKey = flatKey.slice(prefix.length + 1); // 去掉 "prefix."
      if (sa[flatKey] !== undefined) {
        // 只有当嵌套对象中没有同路径 key 时才注入
        if (existingIsObj) {
          // 检查嵌套路径是否已存在
          const parts = innerKey.split(".");
          let cursor = sa[prefix];
          let exists = true;
          for (const p of parts) {
            if (cursor === undefined || cursor === null || !(p in cursor)) {
              exists = false;
              break;
            }
            cursor = cursor[p];
          }
          if (!exists) {
            setNested(sa[prefix], innerKey, sa[flatKey]);
            moved++;
          }
        } else {
          setNested(sa[prefix], innerKey, sa[flatKey]);
          moved++;
        }
      }
    }
  }

  // 4. 删除所有扁平 key
  for (const key of dotKeys) {
    delete sa[key];
  }

  // 5. 序列化回 JSON
  // 保留 2 空格缩进和尾随换行
  const serialized = JSON.stringify(data, null, 2) + "\n";

  // 6. 验证
  try {
    JSON.parse(serialized);
    fs.writeFileSync(filePath, serialized, "utf-8");
    console.log(`    ✅ ${moved} 个值迁入嵌套对象，${dotKeys.length} 个扁平 key 已删除 — JSON 有效`);
  } catch (e) {
    console.log(`    ❌ 序列化后 JSON 无效: ${e.message} — 未写入`);
  }
}

console.log("\n全部处理完成！");
