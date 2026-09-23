// 解码 psql 导出的 base64 大字段 → pretty JSON（psql 的 -A -F tab 输出会把长 JSON 行破坏，
// 且 Read 工具对 >2000 字符的行会截断 ⇒ 走 base64 中转是唯一无损路径）。
import fs from "node:fs";

const src = process.argv[2];
const dst = process.argv[3];
const raw = fs.readFileSync(src, "utf8").split(/\r?\n/);
// 首行是列名（b / llm），其余行拼起来才是 base64 体
const body = raw.slice(1).join("").trim();
const text = Buffer.from(body, "base64").toString("utf8");
let out = text;
try {
  out = JSON.stringify(JSON.parse(text), null, 2);
} catch {
  // 非 JSON（或 JSON 片段）就原样落盘
}
fs.writeFileSync(dst, out, "utf8");
console.log("bytes=", out.length);
