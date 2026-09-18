// sim-verify.rhai —— 「仿真验证」**节点体**（图上位于决策之后，默认不在 DAG 执行链上）
//
// ## ⚠️ 本脚本在默认路径下**不会被执行**（务必先读这段）
//
// 模板里的 `sim-verify` 节点是 `enabled: false` **且不连任何边**的「图示节点」：
//   · `enabled=false` —— DAG 调度器跳过它；
//   · 无边 —— 既不会被 `compute_ready_nodes`（入度 0）当成就绪节点立刻跑一次，
//     也不会因留下出边而把下游 `store-result` 卡成永久 Pending。
//   （两条必须成对出现，理由见 `seed_stock_analysis.rs` 节点定义处注释。）
//
// 因此本文件有**两条**互斥的执行路径，二者调用同一个 Rust 实现：
//
//   路径甲（**默认、唯一实际跑的**）：决策落库后由后端挂钩触发 ——
//     `stock_workflow/core.rs` → `sim_hook::spawn_simulation_after_decision`
//     → `commands::market_sim::run_mc_preset(...)`。
//     `rerun_decision`（重跑决策）走同一挂钩，避免决策改了而仿真还是旧的。
//     选它是因为它**在持久化之后**，不占用工作流执行时长。
//
//   路径乙（**仅当有人手动把该节点 `enabled` 打开时**）：DAG 的 code_executor
//     执行本脚本 → 宿主函数 `sim_run_mc` → 同一个 `run_mc_preset`。
//     保留它是为了让节点在编辑器里自带可读、可单独试跑的定义，
//     **不是**默认链路的一环。改动仿真参数时，两条路径的入参必须同步。
//
// ## 位置（图上）
//   … → quality-gate（最后一个会改决策的节点）→ decision-explainer
//     → store-result（持久化）
//     并列：**sim-verify（本节点，enabled=false，与 store-result 同排显示）**
//
// ## 语义边界（这正是它「不阻滞决策」的机制）
//   只读上游、只产出补充信息，不产出 action / positionPct / confidence 等任何
//   决策字段；全图**没有任何节点消费它的输出**，结果仅注入快照供前端展示
//   ⇒「在决策之后」由数据依赖方向保证，不是靠约定。
//
// ## 它回答什么
//   「这套仓位在最坏情形下会怎样」，**不是**「它会涨还是会跌」。
//   ⚠️ 仿真内核没有 order 注入入口（不含「你的这一单」的下单量），因此**不得**
//   把本结果解读为本笔交易的冲击成本 —— 详见 `PLAN-market-sim-value.md`。
//
// ## 实现要点
//   仿真循环在 Rust 宿主函数 `sim_run_mc` 内完成（注册于
//   `stock_workflow/rhai_pm.rs::register_pm_functions`）：共享 Rhai 引擎设了
//   `set_max_operations(200_000)`，把「场景 × 路径 × 事件」三级循环写在脚本里
//   必然超限。宿主函数内部是 Rust 循环，对 Rhai 只计 1 次操作。

// V57 兼容：`present(x)` 是 code_executor 会扫描的**约定函数名**。
// 脚本中出现的 `present(v)`，若 v 未被 input_mapping 注入，执行器会自动补 unit，
// 避免 Rhai 在求值参数阶段就抛 `Variable not found`（函数体根本进不去）。
fn present(x) {
    if x == () {
        return false;
    }
    if type_of(x) == "string" {
        return x != "";
    }
    return true;
}

// 安全取字段：JSON 解析出的 map 缺字段时返回 ()，不抛错。
// 不用 `m.key` 属性访问 —— 缺字段时的行为依 Rhai 版本而异，显式判断更稳。
fn field(m, key) {
    if m == () {
        return ();
    }
    if !(key in m) {
        return ();
    }
    m[key]
}

// ⚠️ 错误码值在此是**副本**，权威源是 `commands/error_code.rs::stock_sim`。
// Rhai 读不到 Rust 常量（同 `domain_ontology` 的三力权重：`.rhai` 只能靠模板变量
// 注入，注入不了就得留一份 fallback）。本脚本默认不执行（`enabled=false`），
// 为它专门造一套注入机制不划算，故保留副本；**改动 stock_sim 域码值时此处必须同步**。
// 码值形态受 `scripts/check-errorcode-alignment.mjs` 约束：全大写下划线、至少两段，
// 且必须在 11 语言顶层 `error` 段有翻译。**禁止**改回自由文本 —— 非中文界面无法翻译。
if !present(stock_code) || !present(current_price) {
    return #{
        "simOk": false,
        "code": "STOCK_SIM_INPUT_MISSING",
        "category": "validation",
    };
}

// current_price 单位是元（与 t-scoring 的 currentPrice 口径一致），
// 宿主函数内部转「分」。
let raw = sim_run_mc(stock_code, current_price, "stress");

if !present(raw) {
    return #{
        "simOk": false,
        "code": "STOCK_SIM_EXEC_FAILED",
        "category": "unrecoverable",
    };
}

let r = json_parse(raw);

if r == () {
    return #{
        "simOk": false,
        "code": "STOCK_SIM_RESULT_PARSE_FAILED",
        "category": "unrecoverable",
    };
}

// 宿主函数失败时返回 `ErrorResponse` 形状（含 code，不抛异常），此处原样透传：
// 与默认路径（`sim_hook.rs`）的失败体**完全同构**，前端只需一套代码消费。
// 上一版在此把宿主的 `error` 自由文本改写成自造的小写 reason，那正是
// 「第二套错误码体系」的来源 —— 不要再走回去。
let host_code = field(r, "code");

if present(host_code) {
    return #{
        "simOk": false,
        "code": host_code,
        "category": field(r, "category"),
        "detail": field(r, "detail"),
    };
}

let scenarios = [];
let raw_scenarios = field(r, "scenarioResults");

if raw_scenarios != () {
    for item in raw_scenarios {
        scenarios.push(#{
            "scenario": field(item, "scenario"),
            "label": field(item, "label"),
            "paths": field(item, "paths"),
            "priceChangePct": field(item, "priceChangePct"),
        });
    }
}

// consistencyScore = null 表示「不可判定」（各场景涨跌幅均值趋零 ⇒ 变异系数数学上无定义）。
// 必须原样把 null 传给前端，**不可**用 0 兜底 —— 0 会被前端读成「高度一致」，方向相反。
return #{
    "simOk": true,
    "stockCode": field(r, "stockCode"),
    "referencePrice": field(r, "referencePrice"),
    "totalPaths": field(r, "totalPaths"),
    "survivalRate": field(r, "survivalRate"),
    "consistencyScore": field(r, "consistencyScore"),
    "bestScenario": field(r, "bestScenario"),
    "worstScenario": field(r, "worstScenario"),
    "scenarioResults": scenarios,
};
