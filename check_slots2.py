import os
import re

def parse_expert_md(content):
    if not content.startswith('---'):
        return content
    rest = content[3:]
    end = rest.find('\n---')
    if end < 0:
        return content
    return rest[end+4:].strip()

def compile_prompt(text):
    """模拟 rt-workflow/prompt_template.rs 的 compile_prompt 状态机"""
    segments = []
    slot_buffer = ''
    current_text = ''
    var_refs = []
    state = 'Normal'
    chars = list(text)
    i = 0
    while i < len(chars):
        ch = chars[i]
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
    if state == 'SawOpen':
        current_text += '{'
    elif state in ('InSlot', 'SawClose'):
        current_text += '{{' + slot_buffer
        if state == 'SawClose':
            current_text += '}'
    if current_text:
        segments.append(('Static', current_text))
    return segments, var_refs

md_dir = r'd:\OneManager\AxInvest\src-tauri\agency_experts\stock-analysis'
print("=== 10 个 analyst .md body 的所有 Slot ===")
for name in ['market-analyst', 'sentiment-analyst', 'news-analyst', 'fundamentals-analyst',
             'policy-analyst', 'hot-money-tracker', 'lockup-watcher', 'research-analyst',
             'sector-analyst', 'catalyst-analyst']:
    f = os.path.join(md_dir, name + '.md')
    content = open(f, 'r', encoding='utf-8').read()
    body = parse_expert_md(content)
    segments, var_refs = compile_prompt(body)
    empty_slots = [v for v in var_refs if v == '']
    print(f"{name}:")
    print(f"  Slot path 列表: {var_refs}")
    if empty_slots:
        print(f"  *** 发现 {len(empty_slots)} 个空路径 Slot! ***")
