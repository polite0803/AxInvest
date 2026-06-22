#!/usr/bin/env python3
"""
Fix antd message deprecation: replace static `message` with App.useApp() hook.
Only modifies files where a matching React component is found.
Files where injection is not possible are left untouched.
"""

import re, os

SKIP = {"__tests__", "node_modules", ".git"}
COMP_PAT = re.compile(
    r"export\s+(default\s+)?(?:"
    r"function\s+\w+\s*\([^)]*\)\s*\{"
    r"|const\s+\w+(?::\s*[^{]+)?\s*=\s*\((?:[^)]*)\)\s*=>\s*\{"
    r")"
)

def fix(path):
    with open(path, encoding="utf-8") as f:
        content = f.read()

    # Find antd import with message
    m = re.search(
        r"^(import\s*\{)([^}]*\bmessage\b[^}]*)(\}\s*from\s+[\"']antd[\"']\s*;?\s*)$",
        content, re.MULTILINE,
    )
    if not m:
        return False

    # Must have a component function
    cm = COMP_PAT.search(content)
    if not cm:
        return False  # Skip files without matching component

    pre = content[max(0, cm.start() - 3):cm.start()]
    if pre.endswith("("):
        return False  # Skip IIFEs

    # Inject useApp() hook
    brace_end = cm.end()
    line_start = content.rfind("\n", 0, cm.start()) + 1
    indent = content[line_start:cm.start()]
    injection = f"\n{indent}  const {{ message }} = App.useApp();"
    content = content[:brace_end] + injection + content[brace_end:]

    # Replace message with App in import (re-search after injection)
    m2 = re.search(
        r"^(import\s*\{)([^}]*\bmessage\b[^}]*)(\}\s*from\s+[\"']antd[\"']\s*;?\s*)$",
        content, re.MULTILINE,
    )
    if m2:
        items = [x.strip() for x in m2.group(2).split(",") if x.strip()]
        new_items = [x for x in items if x != "message"]
        if "App" not in new_items:
            new_items.append("App")
        new_import = f"{m2.group(1)} {', '.join(new_items)} {m2.group(3)}"
        content = content[:m2.start()] + new_import + content[m2.end():]

    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    return True


def main():
    src = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src")
    count = 0
    for root, dirs, fnames in os.walk(src):
        dirs[:] = [d for d in dirs if d not in SKIP]
        for f in fnames:
            if not f.endswith(".tsx"):
                continue
            path = os.path.join(root, f)
            try:
                if fix(path):
                    print(f"  {os.path.relpath(path, src)}")
                    count += 1
            except Exception as e:
                print(f"  ERROR {path}: {e}", file=__import__("sys").stderr)
    print(f"\nFixed: {count}")


if __name__ == "__main__":
    main()
