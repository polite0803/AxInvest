import os, sys

ROOT = sys.argv[1] if len(sys.argv) > 1 else r"D:\OneManager\AxInvest\src-tauri"
SKIP_FILE = {"register_commands.rs"}

# ---- KEEP protected substrings (+ placeholders) ----
PROTECT = [
    ("config/opc/domain_packs", "__KEEP_PATH_DP__"),
    ("config/opc/industries", "__KEEP_PATH_LEGACY__"),
    ('"opc_domain_packs"', "__KEEP_TABLE_Q1__"),
    ("'opc_domain_packs'", "__KEEP_TABLE_Q2__"),
    ("domain_pack_id", "__KEEP_FIELD_ID__"),
    ("domain_pack_count", "__KEEP_FIELD_CNT__"),
    ('"domain_pack_id"', "__KEEP_FIELD_ID_Q__"),
]

REPLACEMENTS = [
    ("DomainPack", "CapabilityPack"),
    ("DOMAIN_PACKS_DIR", "CAPABILITY_PACKS_DIR"),
    ("LEGACY_DOMAIN_PACKS_DIR", "LEGACY_CAPABILITY_PACKS_DIR"),
    ("domain_pack", "capability_pack"),
]

def transform_text(t):
    for token, ph in PROTECT:
        t = t.replace(token, ph)
    for src, dst in REPLACEMENTS:
        t = t.replace(src, dst)
    for token, ph in PROTECT:
        t = t.replace(ph, token)
    return t

# ---- Step A: content replacement on .rs files ----
count = 0
for dirpath, dirnames, filenames in os.walk(ROOT):
    if "target" in dirnames:
        dirnames.remove("target")
    for fn in filenames:
        if not fn.endswith(".rs"):
            continue
        if fn in SKIP_FILE:
            continue
        p = os.path.join(dirpath, fn)
        with open(p, "r", encoding="utf-8") as f:
            orig = f.read()
        new = transform_text(orig)
        if new != orig:
            with open(p, "w", encoding="utf-8") as f:
                f.write(new)
            count += 1
print("content-replaced files:", count)

# ---- Step B: rename files whose basename contains 'domain_pack' ----
renamed_files = []
for dirpath, dirnames, filenames in os.walk(ROOT):
    if "target" in dirnames:
        dirnames.remove("target")
    for fn in filenames:
        if "domain_pack" in fn:
            old = os.path.join(dirpath, fn)
            new = os.path.join(dirpath, fn.replace("domain_pack", "capability_pack"))
            if old != new and os.path.exists(old) and not os.path.exists(new):
                os.rename(old, new)
                renamed_files.append((old, new))
print("renamed files:", len(renamed_files))

# ---- Step C: rename dirs deepest-first ----
dirs = []
for dirpath, dirnames, filenames in os.walk(ROOT):
    if "target" in dirnames:
        dirnames.remove("target")
    for d in dirnames:
        if "domain_pack" in d:
            dirs.append(os.path.join(dirpath, d))
dirs.sort(key=lambda x: x.count(os.sep), reverse=True)
renamed_dirs = 0
for old in dirs:
    new = os.path.join(old, "..") + os.sep + os.path.basename(old).replace("domain_pack", "capability_pack")
    new = os.path.normpath(new)
    if old != new and os.path.exists(old) and not os.path.exists(new):
        os.rename(old, new)
        renamed_dirs += 1
        print("RENAMED DIR", old, "->", new)
print("renamed dirs:", renamed_dirs)