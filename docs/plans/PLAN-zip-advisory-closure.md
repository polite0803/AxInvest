# PLAN：cargo audit 的 zip 三族红收口

触发：CI `Run cargo audit` 红 —— `error: 9 vulnerabilities found!` + 14 条 allowed unmaintained。
本仓 audit 的既有豁免先例见 `.github/workflows/ci.yml:814-832`（rkyv / glib 两条，各带影响范围与退役条件）。

## 一、现网读数（不是推断）

9 条红全部来自 `zip` 一个 crate 的三个并存主版本，每条 ×3 个判据：

| zip 版本 | 唯一/全部来源 | 上游最新版对 zip 的要求 | 本地能否升 |
|---|---|---|---|
| 4.6.1 | `tauri-plugin-updater` 2.11.0（`init/plugins.rs:27` 注册） | 最新 2.14.0 仍 `^4` | ✖ 上游没跟 |
| 7.2.0 | `rust_xlsxwriter` 0.96.0（`crates/tools`） | **0.100.0 已 `^9`** | ✔ 今天可清 |
| 8.6.0 | 自家 5 处 `zip = "8"` + `calamine` 0.36.1 + `candle-core` 0.11.0 + `docx-rs` 0.4.22 | calamine **0.37.0 已 `^9`**；candle-core 最新 0.11.0 `^8.6`；docx-rs 最新 0.4.22 `^8.6` | 部分：自家与 calamine 可升，candle/docx-rs 锁死 |

三条判据（RUSTSEC-2026-0367 SUID/SGID、0368 符号链接逃逸、0369 经符号链接套权限）**全部只打在 `ZipArchive::extract()` / `ZipStreamReader::extract()` 这两个「解包到目录」接口上**。

自家代码的 zip 用法实测（`grep -rn "\.extract(\|ZipArchive\|ZipWriter"`）：
写侧 `ZipWriter`（webdav 备份 `storage/src/webdav.rs:513`、OPC 域包 `analysis-engine/src/opc/capability_pack.rs:650`、文档导出 `tools/src/tools/document.rs:2578`）；
读侧一律 `ZipArchive::new` + `by_name` 只读（`document-parser/src/lib.rs:462/580/731/878`、`storage/src/webdav.rs:582`）。
⇒ **`extract()` 在自家代码零调用点**。真正会调 extract 的只有 updater 插件内部。

发版矩阵（`.github/workflows/release.yml:110-123`）含 macOS x86/arm、Windows x86/arm、Linux x86_64 ⇒ 判据里的 Unix 权限/符号链接路径对本 app 的 macOS/Linux 产物是真实相关的，不能用「只发 Windows」搪塞。

## 二、本轮已得的两条裁定（2026-10-19）

- **裁-1 updater：真正接入**（不摘插件）。
- **裁-2 zip 8.6.0：替掉 `docx-rs` / `candle`**（不换 crate 不算收口，不接受仅靠豁免）。

**必须先记下两条后果**：

1. 裁-1 使 zip 4.6.1 三条**不可能**被本地动作消掉（上游最新仍 `^4`）⇒ audit 只能靠「带证据豁免」或「保持红」二者之一收尾。也就是说：**即便裁-2 全部做完，CI 这一格仍然不会绿。**
2. 裁-1 同时把 `extract()` 从「配置层关着的死路径」变成网络喂入的活路径：现状是 `plugins.updater.active=false`、`pubkey` 空、`endpoints` 空、`createUpdaterArtifacts=false`。真接入需要密钥/托管/前端流程一起做，风险等级随之上升（Tauri 的缓解是先 minisign 验签再解包，因此利用前提是签发私钥或发布管线被攻破）。

## 三、阶段切分

### 阶段 A —— 今天可做，零能力变更（不含重写）

1. `crates/tools/Cargo.toml:37`：`rust_xlsxwriter 0.96 → 0.100` ⇒ **7.2.0 三条直接消失**（唯一来源就是它）。
2. 5 处 `zip = "8" → "9"`（`src-tauri/Cargo.toml:248`、`analysis-engine:29`、`document-parser:19`、`storage:26`、`tools:50`）：不减 audit 行数（8.6.0 仍被 candle/docx-rs 留着），但把**我们自己**的副本移出漏洞主版本，也是 C/D 完成后能彻底清零的前置。
3. `crates/document-parser/Cargo.toml:10`：`calamine 0.36 → 0.37`（已 `^9`）。
4. 验证：`cargo check -p axagent -p axagent-tools -p axagent-storage -p axagent-document-parser -p axagent-analysis-engine --all-targets`；`cargo audit` 本地读数从 9 → 6（预期）；`cargo clippy -p <逐个> --all-targets -- -D warnings`。zip 8→9 属主版本变更，`SimpleFileOptions` / `ZipWriter` 签名可能变，改动集中在上述 4 个写侧点。

⚠ 阶段 A 的确切数字要在跑完后回填（本 PLAN 不预填读数）。

### 阶段 B —— updater 真正接入（发版管线工程）

要做的事：`tauri signer generate-key` 出 pubkey 入 `tauri.conf.json:55` 的 `plugins.updater.pubkey`；`endpoints` 指向托管的静态清单；`bundle.createUpdaterArtifacts: true`；前端「检查更新 → 下载 → 安装 → 重启」流程（含 11 语言文案与失败态）；能力清单 `src-tauri/capabilities/default.json:52` 已有 `updater:default`，无需新增。
前置问题（**待裁 Z-4**）：清单托管在哪（GitHub Releases / 自建静态站），以及 Linux/macOS/Windows 三套产物格式各用什么（AppImage / dmg / nsis-msi），这决定解包路径是否真的走到 `extract()`。

### 阶段 C —— 替掉 `docx-rs`

实测代价比小：
- **写侧**：`tools/src/tools/document.rs` 共 4627 行，但 `docx_rs` 只出现 21 次、11 种符号（`Docx`/`Paragraph`/`Run`/`RunChild`/`DocumentChild`/`LineSpacing`/`LineSpacingType` 等）。替法两条（**待裁 Z-3**）：① 自研 OOXML 生成 —— 复用仓内已在跑的 `zip::ZipWriter` 打包形态（`capability_pack.rs:650` 同构先例），把 document.xml/styles.xml 直接写出来；② 换 crate（生态里同类只有 docx-wasm/js 绑定等，形态不合）。
- **读侧**：`agent/src/ingest_pipeline.rs:802` 的 `read_docx`（4 种符号）**不必另造**——`document-parser/src/lib.rs:462/580/731/878` 已有基于 `zip + by_name` 的 OOXML 只读解析，属既有实现，应扩它而非重定义（AGENTS.md 禁区 12）。
- 回归面：`.docx` 导出的样式/表格/列表编号/页码/行距、`.docx` 摄取管道。要有逐特征对照样本，不能只验「能打开」。

### 阶段 D —— 替掉 `candle`

- 体量：`search/src/inference.rs` 798 行 + `search/src/lora_trainer.rs` 361 行；candle 符号面只有 7 处（`candle_core::{Result, Error::Msg, Device::Cpu}`、`candle_core::quantized::gguf_file::Content::read`、`candle_transformers::models::quantized_llama::ModelWeights(+::from_gguf)`）。
- 接线面（消费方）：`search/src/rag.rs:2486` 经 `Arc<dyn InferenceEngine>` 取 `global_engine()`、`search/src/model_downloader.rs`、`agent/src/fine_tune/candle_trainer.rs`、`src/commands/fine_tune.rs:14`、`src/commands/local_model.rs`。
- ⇒ 好消息：**已有 `InferenceEngine` trait 作接缝**，换实现不必动调用方。
- 难点是 **LoRA 本地训练**（`lora_trainer.rs` + `candle_trainer.rs`）：candle 在这里承担的是自动微分与权重读写，候选（ort/ONNX、tract、burn）都不等价。**待裁 Z-2**：本地 LoRA 训练必须保留吗？若不保留，D 只剩「GGUF 推理」，代价骤降；若必须保留，D 实际等于自研或换一套训练栈。

### 阶段 E —— 收口与防回归

1. C+D 完成且 A 的自家 zip=9 生效后，锁文件应无 zip 4/7/8 ⇒ audit 只剩 updater 的 4.6.1 三条 ⇒ **待裁 Z-1**：这三条走带证据豁免（照 rkyv/glib 先例，写清影响范围/缓解/退役条件），还是接受 audit 长期红。
2. 新门（建议名 `scripts/check-zip-advisory-expiry.mjs`）：判据不是「zip 版本 ≥9」，而是**逐 vendor 现推**——查 `tauri-plugin-updater`/`docx-rs`/`candle-core`/`calamine`/`rust_xlsxwriter` 的最新版本要求的 zip 主版本，任一已升到修复版而其 ignore 仍在 ⇒ 红（防止豁免条目变成永久免检）。这与本仓「豁免带日期且每次打印、失效即红」的既有做法同源。

## 四、待裁（按编号点完即整批开工）

- **Z-1** updater 的 zip 4.6.1 三条：带证据豁免 / 保持红。（不选就没有「CI 绿」这个结果，见 §二.1）
- **Z-2** 本地 LoRA 训练是否必须保留：决定阶段 D 的候选集与代价量级。
- **Z-3** docx 写侧：自研 OOXML 生成（复用 `ZipWriter` 先例）还是换 crate。
- **Z-4** updater 清单托管位置与三平台产物格式：决定 `extract()` 是否真的进入活路径。
- **Z-5** 阶段 A 现在是否开工（分类器已把改 manifest 拦下，需要你显式点头）。

## 五、阶段 A 执行状态（2026-10-19）

**已落地并取到读数**（未提交前不写「完成」）：

- manifest：`zip` 五处 → `{ version = "9", default-features = false, features = ["deflate"] }`；
  `calamine 0.36 → 0.37`；`rust_xlsxwriter 0.96 → 0.100`。
  **与原计划的偏离**：原写 `zip = "9"`。实测 plain "9" 会带上 zip 9 的默认 feature，把
  `zstd-sys` 从 2.0.16 顶到 2.1.2 ⇒ 必须重新编译并**链接** C 源码 ⇒ 本机缺 Windows SDK 的
  `kernel32.lib` 时直接炸。改成只开 `deflate`（我们全部用法就是 Deflate 写 + 只读解包）后
  `zstd-sys` 停在已缓存的 2.0.16，链接面归零 —— 顺带把没在用的压缩/加密后端移出依赖图。
- 读数：`zip 7.2.0` 已从 `Cargo.lock` 消失（cargo 自报 `Updating zip v7.2.0 -> v9.0.5`）
  ⇒ audit 9 条 → **预期 6 条**（4.6.1 三条 + 8.6.0 三条）。

**迁移面与原判断相反**：PLAN 猜「写侧 `SimpleFileOptions`/`ZipWriter` 可能变」——实际写侧
**零改动**（`SimpleFileOptions` 在 zip 9 只是 `FileOptions<'static,'static,()>` 的别名）。
全部破坏在**读侧三个方法改返回类型**：`ZipFile::name()` → `ZipResult<Cow<str>>`、
`ZipArchive::file_names()` → 逐条 `ZipResult<Cow<str>>`、`ZipFile::mangled_name()` → `ZipResult<PathBuf>`。
共 9 处，分布在 7 个文件；其中三处是安全判据（webdav 还原、OPC 域包解包、技能安装的
符号链接 + `safe_zip_path`），一律**失败即中止**，不许用空名顶替 —— 空名会直接通过 zip-slip 检查。

**连带修**：`knowledge.rs` 因这次编辑整体后移 6 行，`harness/knowledge_graph.rs` 有 6 处按
`文件:行` 引用它。行号门只硬拦了落在 `continue;` 上的那一条，其余五条「位置有效、语义已漂」
是门查不出的 ⇒ 全部按内容重定位（2913 / 2928 / 2944）。该文件自陈「行号引用已漂移六次」，
正解是改引字面量（`"in_industry"`）而不是行号 —— 未做，记在此处。

**验证口径（本轮实测，写下来免得下次再猜）**：
`__TAURI_WORKSPACE__=true cargo check -p axagent --all-targets` 可绕过 winres/RC
（`build.rs:41` 的分支跳过 `tauri_build::build()`），代价是不再由 tauri-build 发 `cfg(desktop)`，
本仓受影响仅 `init/plugins.rs`、`lib.rs`、`register_commands.rs`、`tray.rs` 四个文件。
**clippy 与 `cargo test` 绕不过**：两者都要链接产物（clippy 连每个 crate 的 build script 都要重链）
⇒ 这两格只能由 CI 取读数。

## 六、2026-10-20 裁定：六条全部带证据豁免（Z-1 与 8.6 临时豁免）

用户按选项拍板：**六条 zip 漏洞全部走带证据豁免**，CI audit 立即转绿；C/D 仍按原计划另排。

- 上游核实（2026-10-20）：`tauri-plugin-updater` 2.14.0 仍 `^4`，3.0.0-alpha.3 只配
  `tauri ^3.0.0-alpha` 且仍 `^4`；`docx-rs` 0.4.22 与 `candle-core` 0.11.0 均为最新版，
  仍 `^8.6.0` ⇒ 没有可升级的兼容版本。
- 可达性核实：三条判据只打在 `ZipArchive::extract()` / `ZipStreamReader::extract()`。
  `candle-core` 与 `docx-rs` 源码只有 `ZipArchive::new` 只读与 `ZipWriter` 写入，**零
  `extract()` 调用**；`tauri-plugin-updater` 的唯一调用点是 Windows 更新解包，而当前
  updater 未启用（active=false、pubkey/endpoints 空、createUpdaterArtifacts=false），
  启用时 Tauri 先验 minisign 再解包。
- 落地：`.github/workflows/ci.yml` 的 cargo audit 增加
  `--ignore RUSTSEC-2026-0367 --ignore RUSTSEC-2026-0368 --ignore RUSTSEC-2026-0369`，
  注释写明影响范围、缓解与退役条件。
- 读数：本地按 CI 同参数复跑 `cargo audit` **EXIT=0**，`warning: 14 allowed warnings`
  （全部 unmaintained），0 vulnerabilities。
- ⚠ 本裁定**临时推翻裁-2 的「8.6 不接受仅靠豁免」**（用户明确选择「六条都带证据豁免」），
  但 C/D 的替换计划不废止；退役条件：任一来源升到 zip>=9 或 C/D 完成替换后删除对应 ignore。
