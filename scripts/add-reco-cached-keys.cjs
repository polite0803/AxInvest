// 给 11 个 i18n locale 文件的 stockAnalysis.recommendation 节点加 3 个新 key
//   - cachedBadge: "缓存" / "Cached" etc.
//   - cachedAt:    "缓存于 {{time}}" / "Cached at {{time}}" etc.
//   - emptyNoCache: "暂无缓存结果,请点击刷新获取最新推荐" / "No cached results..." etc.
//
// 严格要求:只插入新 key 行,绝不动其它行的顺序/格式。

const fs = require("fs");
const path = require("path");

const LOCALES_DIR = path.join(__dirname, "..", "src", "i18n", "locales");

const NEW_KEYS = {
  "zh-CN": {
    cachedBadge: "缓存",
    cachedAt: "缓存于 {{time}}",
    emptyNoCache: "暂无缓存结果，请点击刷新获取最新推荐",
  },
  "zh-TW": {
    cachedBadge: "快取",
    cachedAt: "快取於 {{time}}",
    emptyNoCache: "暫無快取結果，請點擊刷新獲取最新推薦",
  },
  "en-US": {
    cachedBadge: "Cached",
    cachedAt: "Cached at {{time}}",
    emptyNoCache: "No cached results. Click Refresh to get the latest recommendations.",
  },
  "de": {
    cachedBadge: "Zwischengespeichert",
    cachedAt: "Zwischengespeichert um {{time}}",
    emptyNoCache:
      "Keine zwischengespeicherten Ergebnisse. Klicken Sie auf Aktualisieren, um die neuesten Empfehlungen zu erhalten.",
  },
  "es": {
    cachedBadge: "En caché",
    cachedAt: "En caché a las {{time}}",
    emptyNoCache: "No hay resultados en caché. Haz clic en Actualizar para obtener las últimas recomendaciones.",
  },
  "fr": {
    cachedBadge: "En cache",
    cachedAt: "En cache à {{time}}",
    emptyNoCache: "Aucun résultat en cache. Cliquez sur Actualiser pour obtenir les dernières recommandations.",
  },
  "ja": {
    cachedBadge: "キャッシュ",
    cachedAt: "{{time}} にキャッシュ",
    emptyNoCache: "キャッシュされた結果はありません。最新のおすすめを取得するには「更新」をクリックしてください。",
  },
  "ko": {
    cachedBadge: "캐시",
    cachedAt: "{{time}}에 캐시됨",
    emptyNoCache: "캐시된 결과가 없습니다. 새로 고침을 클릭하여 최신 추천을 받으세요.",
  },
  "ru": {
    cachedBadge: "Кэш",
    cachedAt: "Кэшировано в {{time}}",
    emptyNoCache: "Нет кэшированных результатов. Нажмите «Обновить», чтобы получить последние рекомендации.",
  },
  "hi": {
    cachedBadge: "कैश्ड",
    cachedAt: "{{time}} पर कैश्ड",
    emptyNoCache: "कोई कैश्ड परिणाम नहीं। नवीनतम सिफारिशें प्राप्त करने के लिए रीफ़्रेश पर क्लिक करें।",
  },
  "ar": {
    cachedBadge: "مخزّن",
    cachedAt: "مخزّن في {{time}}",
    emptyNoCache: "لا توجد نتائج مخزنة. انقر فوق تحديث للحصول على أحدث التوصيات.",
  },
};

/**
 * 找到 `stockAnalysis.recommendation` 对象(子对象模式)
 * 算法:先定位 `"stockAnalysis": {`,再在它的 brace 内找 `"recommendation": {`
 */
function findRecommendationObject(text) {
  // 1) 找到 "stockAnalysis": { 的位置
  const saRe = /"stockAnalysis"\s*:\s*\{/g;
  const saM = saRe.exec(text);
  if (!saM) { return null; }
  const saStart = saM.index + saM[0].length;
  // 2) 找到 stockAnalysis 对象结束
  let depth = 1;
  let i = saStart;
  let inString = false;
  let escape = false;
  while (i < text.length && depth > 0) {
    const ch = text[i];
    if (escape) {
      escape = false;
      i++;
      continue;
    }
    if (ch === "\\") {
      escape = true;
      i++;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      i++;
      continue;
    }
    if (inString) {
      i++;
      continue;
    }
    if (ch === "{") { depth++; }
    else if (ch === "}") { depth--; }
    i++;
  }
  if (depth !== 0) { return null; }
  const saEnd = i - 1; // 指向 stockAnalysis 的 }
  const saInner = text.slice(saStart, saEnd);

  // 3) 在 stockAnalysis 内找 "recommendation": {
  const recoRe = /"recommendation"\s*:\s*\{/g;
  const recoM = recoRe.exec(saInner);
  if (!recoM) { return null; }
  const recoStart = saStart + recoM.index + recoM[0].length; // 指向 reco {
  // 4) 配对 reco } (确保在 stockAnalysis 范围内)
  let rDepth = 1;
  let j = recoStart;
  let rinString = false;
  let rescape = false;
  while (j < text.length && rDepth > 0) {
    const ch = text[j];
    if (rescape) {
      rescape = false;
      j++;
      continue;
    }
    if (ch === "\\") {
      rescape = true;
      j++;
      continue;
    }
    if (ch === '"') {
      rinString = !rinString;
      j++;
      continue;
    }
    if (rinString) {
      j++;
      continue;
    }
    if (ch === "{") { rDepth++; }
    else if (ch === "}") { rDepth--; }
    j++;
  }
  if (rDepth !== 0) { return null; }
  const recoEnd = j - 1;
  return { start: recoStart, end: recoEnd, braceStart: saStart + recoM.index };
}

/**
 * 找到 recommendation 对象内 key 列表的最后一行,在它后面插入新行
 * 算法:扫描 recommendation 对象,记录 "key": 的所有行尾(行号),按行号排序,
 * 找到每个新 key 应该插入的"前一行的行尾"
 *
 * 关键:对每行计算 brace 深度,只把"与 { 平级的"(depth 0)的 key 视为 recommendation
 * 自己的直接子键,跳过嵌套对象/数组里的 key。
 */
function insertKeysIntoRecommendationObject(text, objRange, keys) {
  const inner = text.slice(objRange.start, objRange.end);
  // 把 inner 按行 split
  const lines = inner.split("\n");
  // 缩进 = 4 + 2 = 6 空格(看样本)
  const indent = "      ";

  // 跟踪每行的 brace 深度(在 inner 内部的累计深度)
  // 第一行的初始深度 = 0(recommendation 的 `{` 已经在 objRange.start 之前)
  // 但实际上 inner 第一行通常是 "\n  " (recommendation 后的换行),所以 depth 仍为 0
  // 见到 `{` depth++,见到 `}` depth--
  const lineDepths = [];
  let runningDepth = 0;
  for (let idx = 0; idx < lines.length; idx++) {
    let d = 0;
    let inS = false;
    let es = false;
    const line = lines[idx];
    for (let c = 0; c < line.length; c++) {
      const ch = line[c];
      if (es) {
        es = false;
        continue;
      }
      if (ch === "\\") {
        es = true;
        continue;
      }
      if (ch === '"') {
        inS = !inS;
        continue;
      }
      if (inS) { continue; }
      if (ch === "{") { d++; }
      else if (ch === "}") { d--; }
    }
    runningDepth += d;
    lineDepths.push(runningDepth);
  }

  // 找到所有 depth==0 的"直接子 key"行
  const directKeyLineIdx = new Map(); // key → 行索引
  for (let idx = 0; idx < lines.length; idx++) {
    if (lineDepths[idx] !== 0) { continue; // 跳过嵌套对象内的行
     }
    const m = lines[idx].match(/^(\s+)"([^"]+)"\s*:/);
    if (m) {
      directKeyLineIdx.set(m[2], idx);
    }
  }
  // 找到 `}` 单独一行的位置(depth 降为 -1 的位置,即 recommendation 自己的右括号)
  let closingBraceIdx = -1;
  for (let idx = 0; idx < lineDepths.length; idx++) {
    if (lineDepths[idx] === -1) {
      closingBraceIdx = idx;
      break;
    }
  }

  // 检查已存在
  for (const k of Object.keys(keys)) {
    if (directKeyLineIdx.has(k)) {
      console.log(`  skip existing: ${k}`);
      delete keys[k];
    }
  }
  if (Object.keys(keys).length === 0) { return { text, inserted: false }; }

  // 按字母序排序新 key
  const newEntries = Object.entries(keys).sort(([a], [b]) => a.localeCompare(b));
  // 按字母降序遍历,这样从后往前插入不会影响前面 key 的位置
  newEntries.sort(([a], [b]) => b.localeCompare(a));

  // 计算每个新 key 的插入"锚点行索引"
  // 算法:在直接子 key 中找第一个 ">" k 的 key 行,新行要插到该行**之前**
  //   (即:它是字母序第一个比新 key 大的,所以新 key 应放在它之前)
  //   若找不到(新 key 比所有现有 key 都大),则放在 closing } 之前
  const insertions = newEntries.map(([k, v]) => {
    let anchorIdx = -1;
    const sortedKeys = Array.from(directKeyLineIdx.entries()).sort((a, b) => a[0].localeCompare(b[0]));
    for (const [existingK, idx] of sortedKeys) {
      if (existingK.localeCompare(k) > 0) {
        anchorIdx = idx;
        break;
      }
    }
    // isInsertBefore: 新行要插到 anchor 之前;否则插到 closing } 之前
    const isInsertBefore = anchorIdx !== -1;
    if (!isInsertBefore) {
      anchorIdx = closingBraceIdx;
    }
    return { k, v, anchorIdx, isInsertBefore };
  });

  // 按 anchorIdx 降序处理,避免前面插入改变后续行号
  insertions.sort((a, b) => b.anchorIdx - a.anchorIdx);

  for (const { k, v, anchorIdx, isInsertBefore } of insertions) {
    const anchorLine = lines[anchorIdx];
    // 锚点行:recommendation 自己的 `}` → 插入到它之前,新行无逗号
    // 锚点行:直接子 key 行:
    //   - isInsertBefore: 新行插到 anchor 之前,新行带逗号;且 anchor 行是它之后的第一行,
    //     永远有逗号(或被新行以逗号收尾),不需要动 anchor
    //   - 否则:新行插到 anchor 之后,新行带逗号;若 anchor 原本无逗号,需补逗号
    const isClosingBrace = lineDepths[anchorIdx] === -1;

    if (isInsertBefore) {
      // 新行插到 anchor 之前
      if (isClosingBrace) {
        // closing } 之前插入,新行无逗号
        const newLine = `${indent}"${k}": "${escapeForJson(v)}"`;
        lines.splice(anchorIdx, 0, newLine);
      } else {
        // key 行之前插入,新行带逗号
        const newLine = `${indent}"${k}": "${escapeForJson(v)}",`;
        lines.splice(anchorIdx, 0, newLine);
      }
    } else {
      // 新行插到 anchor 之后(适用于:新 key 比所有现有 key 都大,anchor 是 closing })
      // 此时 anchor 是 closing },新行带逗号(后面跟着 closing })
      const newLine = `${indent}"${k}": "${escapeForJson(v)}",`;
      lines.splice(anchorIdx, 0, newLine);
    }
  }

  const newInner = lines.join("\n");
  const newText = text.slice(0, objRange.start) + newInner + text.slice(objRange.end);
  return { text: newText, inserted: true };
}

function escapeForJson(s) {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

const errors = [];
for (const [locale, keys] of Object.entries(NEW_KEYS)) {
  const file = path.join(LOCALES_DIR, `${locale}.json`);
  if (!fs.existsSync(file)) {
    errors.push(`file missing: ${file}`);
    continue;
  }
  const text = fs.readFileSync(file, "utf8");
  const objRange = findRecommendationObject(text);
  if (!objRange) {
    errors.push(`${locale}: stockAnalysis.recommendation not found`);
    continue;
  }
  const result = insertKeysIntoRecommendationObject(text, objRange, { ...keys });
  if (result.inserted) {
    fs.writeFileSync(file, result.text, "utf8");
    console.log(`[${locale}] updated`);
  } else {
    console.log(`[${locale}] no change`);
  }
}

if (errors.length) {
  console.error("ERRORS:", errors);
  process.exit(1);
}
console.log("done");
