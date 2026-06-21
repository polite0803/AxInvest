import re

setup_path = r'd:\OneManager\AxInvest\src-tauri\src\commands\stock_analysis_setup.rs'
content = open(setup_path, 'r', encoding='utf-8').read()

# 找 system_prompt: format!(...) 多行字符串
# 模式: system_prompt: format!( <whitespace>* "<string-literal-with-escapes>" <whitespace>* )
pattern = re.compile(r'system_prompt:\s*format!\(\s*"((?:[^"\\]|\\.)*?)"\s*,?\s*\)', re.DOTALL)
matches = pattern.findall(content)
print(f"找到 {len(matches)} 个 system_prompt: format! 字符串")
for i, m in enumerate(matches):
    # 找 {{...}} literal
    slot_pattern = re.findall(r'\{\{([^{}]*)\}\}', m)
    empty_slots = [s for s in slot_pattern if s.strip() == '']
    # 也找所有 slot path
    if slot_pattern or empty_slots:
        print(f"  [{i}]  slot path: {slot_pattern[:10]}")
        if empty_slots:
            print(f"      *** 包含空路径 Slot! ***")
            print(f"      字符串前 200 字符: {m[:200]}")
