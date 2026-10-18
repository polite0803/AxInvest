// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流侧「模型级请求延时」接线门。
//!
//! ## 为什么需要它
//!
//! 设置页那格（`settings.requestDelayMs`，文案「每次调用 API 前等待的毫秒数」）此前在工作流路径
//! **零消费点** —— 面板改了不报错、不掉日志，只是分析链照旧打 429。接线做在 5 个 executor 的
//! **10 个 `LlmCallConfig` 构造点**上，而这类改动最常见的复发形态是「后来新增的第 11 个构造点
//! 忘了带字段」—— 编译器不拦（`..Default::default()` 合法），运行时也不响。
//!
//! ## 与 harness 那两条判据的分工
//!
//! `crates/harness/src/llm_executor.rs` 的 `request_delay_tests` 钉**消费端真在睡**；
//! 本门钉**调用端把值送到了**。摘掉任一边都只红对应那一条。

use std::fs;
use std::path::{Path, PathBuf};

/// 是否为 `LlmCallConfig` 的**构造点**。
///
/// 三排除：`use` 行、`&LlmCallConfig` 这类类型标注、以及注释行 —— 注释里出现
/// `LlmCallConfig::default()` 的字样**不是**代码，把它算进扫描面会让本门对
/// 「文档改写」误报，进而被当成噪声关掉。
fn is_construction_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("use ") || t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
        return false;
    }
    t.contains("LlmCallConfig::default()") || t.contains("LlmCallConfig {")
}

/// 扫一份源码：返回 `(构造点总数, 漏掉 request_delay_ms 的行号列表)`（行号 1 基）。
///
/// 结构体字面量从声明行向后扫到闭合 `};` 再判 ⇒ rustfmt 怎么折行都不影响结论。
/// 「总数」与「违规」由同一次遍历给出，避免两处各写一遍「什么算构造点」。
fn scan_sites(text: &str) -> (usize, Vec<usize>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut total = 0;
    let mut missing = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_construction_line(line) {
            continue;
        }
        total += 1;
        if line.trim_start().contains("LlmCallConfig::default()") {
            missing.push(i + 1);
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let mut j = i + 1;
        while j < lines.len() {
            let l = lines[j];
            if l.trim() == "};" && (l.len() - l.trim_start().len()) <= indent + 4 {
                break;
            }
            j += 1;
        }
        let block = lines[i..j.max(i) + 1].join("\n");
        if !block.contains("request_delay_ms") {
            missing.push(i + 1);
        }
    }
    (total, missing)
}

fn executor_sources(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("读 {} 失败: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|d| d.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rs"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_workflow_llm_config_site_carries_the_model_delay() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/work_engine/executors");
    let mut checked = 0usize;
    let mut violators: Vec<String> = Vec::new();
    for path in executor_sources(&dir) {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("读 {:?} 失败: {e}", path));
        let (total, missing) = scan_sites(&text);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        checked += total;
        violators.extend(missing.into_iter().map(|ln| format!("{name}:{ln}")));
    }
    // 防空集恒真：现况是 10 个构造点（agent_executor 6 + condition/classifier/switch/llm 各 1）。
    assert!(
        checked >= 10,
        "扫描面异常：只数到 {checked} 个 LlmCallConfig 构造点，先确认 executor 目录与写法没变"
    );
    assert!(
        violators.is_empty(),
        "这些构造点没带 request_delay_ms（面板那格在这条路径上又成了零消费点）：{violators:?}"
    );
}

/// 探测器自证：假样本必须被报出来，正确写法必须放行。
/// 缺这一条时，上面那条断言在「探测器什么都不返回」的坏写法下也会恒绿。
#[test]
fn detector_spots_a_site_that_omits_the_field() {
    let bare = "fn f() {\n    let cfg = LlmCallConfig::default();\n}\n";
    assert_eq!(scan_sites(bare), (1, vec![2]), "裸 default() 必须被点名");

    let literal_without = concat!(
        "fn f() {\n",
        "    let llm_config = LlmCallConfig {\n",
        "        max_context_tokens: None,\n",
        "        ..Default::default()\n",
        "    };\n",
        "}\n",
    );
    assert_eq!(scan_sites(literal_without), (1, vec![2]), "字面量漏字段必须被点名");

    let literal_with = concat!(
        "fn f() {\n",
        "    let llm_config = LlmCallConfig {\n",
        "        request_delay_ms: request_delay_ms_for_model(&prov, &model),\n",
        "        ..Default::default()\n",
        "    };\n",
        "}\n",
    );
    assert_eq!(scan_sites(literal_with), (1, Vec::new()), "带了字段的写法不该被误报");

    // 非构造点：`use` 行与类型标注既不计入总数，也不报违规。
    let not_a_site =
        "use axagent_harness::{LlmCallConfig, execute_llm};\nfn g(cfg: &LlmCallConfig) {}\n";
    assert_eq!(scan_sites(not_a_site), (0, Vec::new()), "非构造点不得算进扫描面");

    // 负控五：注释里出现同样字样 ⇒ 不是代码，不得计入、更不得报违规。
    let comment_only = "/// 改前写法是 `LlmCallConfig::default()`。\nfn h() {}\n";
    assert_eq!(scan_sites(comment_only), (0, Vec::new()), "注释不得被当成构造点");
}
