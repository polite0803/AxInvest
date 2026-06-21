fn main() {
    // 模拟 v15 修复版 inline system_prompt 源码(line 1135-1137 in commit 28e93fd78)
    // 关键:这是 Rust format! 字符串字面量,{{stock_code}} 2个 { 编译后是 {stock_code} 单大括号
    let title = "技术面分析:K线形态、MACD/RSI、支撑阻力位";
    let v15_inline = format!(
        "你的任务: {title}\n目标股票代码: {{stock_code}}，股票名称: {{stock_name}}\n\n重要原则：\n1. 如果上游数据节点返回为空，请主动调用可用工具获取补充数据。\n2. 如果确实无法获取某些数据，基于你已知的公开信息和通用分析框架给出尽可能有价值的分析，不要只列 data_gaps。\n3. 始终针对目标股票给出明确的观点（看多/看空/中性）和论据，不要输出空结果。\n4. 调用任何需要 stock_code 参数的工具时，必须始终传递 stock_code={{stock_code}}。\n5. 分析输出中严禁出现'工具调用失败'、'在当前环境中不可用'、'上游数据获取为空'、'数据缺失'、'无法获取'、'does not exist'、'error'等负面措辞。工具返回空数组[]或空对象{{}}是正常情况（表示该数据源暂无记录），请直接基于已有信息给出分析结论。\n6. 如果你是研报分析师，目标是从券商研报、一致预期EPS、机构调研等维度给出观点。如果这些数据源返回空，说明该股票暂无机构覆盖，你可以基于公司基本面、行业地位、新闻公告等公开信息给出独立分析，不要强调'无券商研报'。",
    );

    println!("=== v15 修复版编译后字符串 ===");
    println!("字符串长度: {} 字符", v15_inline.len());
    println!();

    println!("--- {{ 和 }} 出现次数 ---");
    let open_d = v15_inline.matches("{{").count();
    let close_d = v15_inline.matches("}}").count();
    println!("'{{' 出现次数: {}", open_d);
    println!("'}}' 出现次数: {}", close_d);
    println!();

    // 模拟 compile_prompt 状态机,扫描所有 {{X}} Slot
    println!("--- 模拟 compile_prompt 状态机 ---");
    let chars: Vec<char> = v15_inline.chars().collect();
    let mut state = "Normal";
    let mut slot_buffer = String::new();
    let mut slot_count = 0;
    let mut empty_slot_count = 0;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match state {
            "Normal" => {
                if ch == '{' {
                    state = "SawOpen";
                }
            },
            "SawOpen" => {
                if ch == '{' {
                    slot_buffer.clear();
                    state = "InSlot";
                } else {
                    state = "Normal";
                }
            },
            "InSlot" => {
                if ch == '}' {
                    state = "SawClose";
                } else {
                    slot_buffer.push(ch);
                }
            },
            "SawClose" => {
                if ch == '}' {
                    let path = slot_buffer.trim().to_string();
                    slot_count += 1;
                    if path.is_empty() {
                        empty_slot_count += 1;
                        println!("*** 发现空路径 Slot! ***");
                    } else {
                        println!("Slot path: '{}'", path);
                    }
                    state = "Normal";
                } else {
                    slot_buffer.push('}');
                    slot_buffer.push(ch);
                    state = "InSlot";
                }
            },
            _ => {},
        }
        i += 1;
    }
    println!();
    println!("共发现 {} 个 {{X}} Slot, 其中 {} 个空路径", slot_count, empty_slot_count);
}
