import os, sys
fp = r"C:\Users\polit\Downloads\A股反思复盘.json"
print("size:", os.path.getsize(fp))
with open(fp, "rb") as f:
    raw = f.read()
print("first 400 bytes (repr):")
print(repr(raw[:400]))
print("---")
print("last 200 bytes (repr):")
print(repr(raw[-200:]))
