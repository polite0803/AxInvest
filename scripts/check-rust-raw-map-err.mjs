#!/usr/bin/env node
// 后端错误码 i18n 守卫（Phase 4.1）
//
// 禁止 src-tauri/src/commands 下出现「裸」map_err(|x| x.to_string())。
// 后端命令错误必须携带错误码（ErrorResponse）返回，供前端按 error.{CODE} 翻译，
// 否则该错误在切语言后会退回原始英文串、破坏 i18n。
//
// 正确写法（按优先级）：
//   1) 复用已有错误码 + 技术详情（推荐，粒度最细）：
//      .map_err(|e| crate::commands::error::ErrorResponse::err_with_detail(
//          crate::commands::error_code::<业务域>::<CODE>, format!("中文上下文: {e}")))?
//   2) 兜底退化（错误码恒为 COMMON_INTERNAL，仅在你确实没有合适业务码时用）：
//      .map_err(|e| crate::commands::error::CommandError::from_error(
//          e, crate::commands::error::ErrorCategory::Unrecoverable))?
//
// 优先「复用」而不是新增码：新增错误码必须同步 11 语言 error 段，
// 否则 check-errorcode-alignment.mjs 会红。改完这两个脚本都要跑。
//
// 见 AGENTS.md「后端错误码 i18n 规范（强制）」。

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const target = join(root, "src-tauri", "src", "commands");

// 精确匹配「身份式」unwrap：map_err(|IDENT| IDENT.to_string())
const RE = /map_err\(\|(\w+)\|\s*\1\.to_string\(\)\)/;

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) { walk(p, out); }
    else if (/\.rs$/.test(name)) { out.push(p); }
  }
}

const files = [];
walk(target, files);

let violations = 0;
for (const f of files) {
  const rel = f.replace(root + "/", "");
  const lines = readFileSync(f, "utf8").split("\n");
  lines.forEach((line, i) => {
    if (RE.test(line)) {
      violations++;
      console.error(`::error file=${rel},line=${i + 1}::裸 map_err(|x| x.to_string()) 禁止：后端错误必须带错误码返回`);
      console.error(`   ${rel}:${i + 1}: ${line.trim()}`);
    }
  });
}

if (violations > 0) {
  console.error(`\n发现 ${violations} 处裸 map_err(|x| x.to_string())。`);
  console.error(
    "改法（推荐，复用已有错误码）：",
  );
  console.error(
    "  .map_err(|e| crate::commands::error::ErrorResponse::err_with_detail(",
  );
  console.error("      crate::commands::error_code::common::INVALID_INPUT, format!(\"上下文: {e}\")))?",
  );
  console.error(
    "兜底（无合适业务码时，错误码退化为 COMMON_INTERNAL）：",
  );
  console.error(
    "  .map_err(|e| crate::commands::error::CommandError::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable))?",
  );
  console.error(
    "注意：新增错误码须同步 11 语言 error 段，否则 check-errorcode-alignment.mjs 会失败。",
  );
  process.exit(1);
}
console.log("OK: src-tauri/src/commands 未发现裸 map_err(|x| x.to_string())");
