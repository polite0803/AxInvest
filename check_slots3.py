import os

def parse_expert_md(content):
    if not content.startswith('---'):
        return content
    rest = content[3:]
    end = rest.find('\n---')
    if end < 0:
        return content
    return rest[end+4:].strip()

# 复用 Rust 状态机逻辑
def compile_prompt(text):
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
print("=== 全部 30 个 .md 文件 Slot 检查 ===")
all_files = sorted(os.listdir(md_dir))
for name in all_files:
    if not name.endswith('.md'):
        continue
    f = os.path.join(md_dir, name)
    content = open(f, 'r', encoding='utf-8').read()
    body = parse_expert_md(content)
    segments, var_refs = compile_prompt(body)
    empty_slots = [v for v in var_refs if v == '']
    if empty_slots:
        print(f"*** {name} 发现 {len(empty_slots)} 个空路径 Slot! 完整: {var_refs}")
        # 找出空路径 Slot 在 body 里的位置
        idx = 0
        for v in var_refs:
            if v == '':
                # 找 body 里的 {{}} 位置
                pos = body.find('{{}}', idx)
                if pos >= 0:
                    print(f"    位置: {pos} 上下文: ...{body[max(0,pos-20):pos]}[{{}}]{body[pos+4:pos+24]}...")
                idx = pos + 4
