import re
import os
import glob

def compile_prompt(text):
    """模拟 rt-workflow/prompt_template.rs 的 compile_prompt 状态机"""
    segments = []
    slot_buffer = ''
    current_text = ''
    var_refs = []
    state = 'Normal'
    i = 0
    while i < len(text):
        ch = text[i]
        if state == 'Normal':
            if ch == '{':
                state = 'SawOpen'
            else:
                current_text += ch
        elif state == 'SawOpen':
            if ch == '{':
                if current_text:
                    segments.append(('Static', current_text))
                    current_text = ''
                slot_buffer = ''
                state = 'InSlot'
            else:
                current_text += '{' + ch
                state = 'Normal'
        elif state == 'InSlot':
            if ch == '}':
                state = 'SawClose'
            else:
                slot_buffer += ch
        elif state == 'SawClose':
            if ch == '}':
                path = slot_buffer.strip()
                var_refs.append(path)
                segments.append(('Slot', path))
                slot_buffer = ''
                state = 'Normal'
            else:
                slot_buffer += '}' + ch
                state = 'InSlot'
        i += 1
    # EOF 处理
    if state == 'SawOpen':
        current_text += '{'
    elif state in ('InSlot', 'SawClose'):
        current_text += '{{' + slot_buffer
        if state == 'SawClose':
            current_text += '}'
    if current_text:
        segments.append(('Static', current_text))
    return segments, var_refs

def parse_expert_md(content):
    """模拟 parse_expert_md 函数"""
    if not content.startswith('---'):
        return content
    rest = content[3:]
    end = rest.find('\n---')
    if end < 0:
        return content
    return rest[end+4:].strip()

md_dir = r'd:\OneManager\AxInvest\src-tauri\agency_experts\stock-analysis'
print("=== 检查 .md 文件 ===")
for f in sorted(glob.glob(os.path.join(md_dir, '*.md'))):
    name = os.path.basename(f)
    content = open(f, 'r', encoding='utf-8').read()
    body = parse_expert_md(content)
    segments, var_refs = compile_prompt(body)
    empty_slots = [v for v in var_refs if v == '']
    if empty_slots:
        print(f"*** {name} 发现空路径 Slot! 全部 Slot: {var_refs}")
    else:
        print(f"OK {name}: {len(var_refs)} 个 Slot - {var_refs[:5]}")

print()
print("=== 检查 stock_analysis_setup.rs 的所有 inline system_prompt 字符串 ===")
setup_path = r'd:\OneManager\AxInvest\src-tauri\src\commands\stock_analysis_setup.rs'
setup_content = open(setup_path, 'r', encoding='utf-8').read()

# 提取所有 system_prompt: format!(...) 块
# 简化：找 system_prompt: format!(\n   "..."\n  ) 模式
matches = re.findall(r'system_prompt:\s*format!\(\s*"((?:[^"\\]|\\.)*)"', setup_content)
print(f"找到 {len(matches)} 个 system_prompt: format! 字符串")
for i, m in enumerate(matches):
    # Rust 字符串字面量：{{ -> {, }} -> }, {varname} 是 format 占位符
    # 实际编译后字符串：{{X}} 形式保留
    compiled_str = m.replace('{{', '{').replace('}}', '}')
    # 但我们要看是否有 {{X}} literal —— 这些 format 字符串是 "..."，里面 {{ }} 会变成 { }
    # 实际上要查的是编译后字符串里有没有 {{X}} 形式
    # 重新看：format!("{{stock_code}}") 编译后是 {stock_code} (单大括号)
    # format!("{stock_code}") 是 format 占位符
    # 所以 format! 字符串里 {{ 是字面量 {，{{{{stock_code}}}} 是字面量 {{stock_code}}
    # 让我们用更准确的方法：识别 format! 字符串里的 {{...}} 双大括号 literal

    # 找 {{...}} 形式
    slot_pattern = re.findall(r'\{\{([^{}]*)\}\}', m)
    empty_slots = [s for s in slot_pattern if s.strip() == '']
    if empty_slots:
        print(f"  [{i}]  发现空路径 Slot! 全部: {slot_pattern}")
        print(f"      字符串: {m[:200]}")

print()
print("=== 检查所有 .rs 文件的 system_prompt 字符串字面量 ===")
# 找 "..." 字符串里含 {{...}} 的
rs_files = glob.glob(r'd:\OneManager\AxInvest\src-tauri\src\**\*.rs', recursive=True)
rs_files += glob.glob(r'd:\OneManager\AxInvest\src-tauri\crates\**\*.rs', recursive=True)
for f in rs_files:
    try:
        content = open(f, 'r', encoding='utf-8').read()
    except:
        continue
    # 找 system_prompt 字段赋值: 字符串含 {{X}} 形式
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if 'system_prompt' in line and '{{' in line and '}}' in line:
            # 提取 {{...}} 内容
            slots = re.findall(r'\{\{([^{}]*)\}\}', line)
            empty_slots = [s for s in slots if s.strip() == '']
            if empty_slots:
                print(f"  {f}:{i+1} 发现空路径: {line.strip()[:200]}")
