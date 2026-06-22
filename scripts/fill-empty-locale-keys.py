#!/usr/bin/env python3
"""补齐 10 个非 zh-CN 语言文件中 35 个空字符串 key 的翻译。

批量翻译策略：
- zh-TW: 用字词级映射做简→繁转换
- ja: 用基础日语翻译表
- en-US: 英文翻译
- ar/de/es/fr/hi/ko/ru: 用英文作为通用国际化的回退值
"""

import json
import os

LOCALES_DIR = os.path.join(os.path.dirname(__file__), "..", "src", "i18n", "locales")

# 35 个待补齐的 key
TRANSLATIONS = {
    "en-US": {
        "agentRole.browser": "Browser",
        "agentRole.coordinator": "Coordinator",
        "agentRole.developer": "Developer",
        "agentRole.executor": "Executor",
        "agentRole.planner": "Planner",
        "agentRole.researcher": "Researcher",
        "agentRole.reviewer": "Reviewer",
        "agentRole.synthesizer": "Synthesizer",
        "artifact.copiedToClipboard": "Copied to clipboard",
        "artifact.copyFailed": "Copy failed",
        "chat.workflow.errorDetail.THINKING_BLOCK_END": "End",
        "chat.workflow.errorDetail.THINKING_BLOCK_START": "Start",
        "common.key": "Key",
        "common.value": "Value",
        "datasetManager.createdSuccess": "Dataset created",
        "datasetManager.deletedSuccess": "Dataset deleted",
        "datasetManager.sampleAdded": "Sample added",
        "settings.workflow.createdSuccess": "Created successfully",
        "settings.workflow.deletedSuccess": "Deleted successfully",
        "settings.workflow.sampleAdded": "Sample added",
        "stockAnalysis.sharesUnit": "shares",
        "tray.key": "Key",
        "tray.value": "Value",
        "workflow.agentNode.agent": "Agent",
        "workflow.agentNode.roleBrowser": "Browser",
        "workflow.agentNode.roleCoordinator": "Coordinator",
        "workflow.props.bindRole": "Bind role",
        "workflow.props.expertNotFound": "Expert not found",
        "workflow.props.profileCreateFailed": "Profile create failed: {{error}}",
        "workflow.props.profileCreated": "Profile created",
        "workflow.props.profileMatched": "Matched to existing profile",
        "workflow.props.roleBrowser": "Browser",
        "workflow.props.roleCoordinator": "Coordinator",
        "workflow.props.roleTag": "Role: {{role}}",
        "workflow.props.selectRole": "Select role",
    },
    "zh-TW": {
        "agentRole.browser": "瀏覽器",
        "agentRole.coordinator": "協調者",
        "agentRole.developer": "開發者",
        "agentRole.executor": "執行者",
        "agentRole.planner": "規劃者",
        "agentRole.researcher": "研究員",
        "agentRole.reviewer": "審查者",
        "agentRole.synthesizer": "綜合者",
        "artifact.copiedToClipboard": "已複製到剪貼簿",
        "artifact.copyFailed": "複製失敗",
        "chat.workflow.errorDetail.THINKING_BLOCK_END": "結束",
        "chat.workflow.errorDetail.THINKING_BLOCK_START": "開始",
        "common.key": "鍵",
        "common.value": "值",
        "datasetManager.createdSuccess": "資料集建立成功",
        "datasetManager.deletedSuccess": "資料集已刪除",
        "datasetManager.sampleAdded": "樣本新增成功",
        "settings.workflow.createdSuccess": "建立成功",
        "settings.workflow.deletedSuccess": "刪除成功",
        "settings.workflow.sampleAdded": "範例已新增",
        "stockAnalysis.sharesUnit": "只",
        "tray.key": "鍵",
        "tray.value": "值",
        "workflow.agentNode.agent": "智能體",
        "workflow.agentNode.roleBrowser": "瀏覽器",
        "workflow.agentNode.roleCoordinator": "協調者",
        "workflow.props.bindRole": "綁定角色",
        "workflow.props.expertNotFound": "專家未找到",
        "workflow.props.profileCreateFailed": "設定建立失敗: {{error}}",
        "workflow.props.profileCreated": "設定已建立",
        "workflow.props.profileMatched": "已匹配到既有設定",
        "workflow.props.roleBrowser": "瀏覽器",
        "workflow.props.roleCoordinator": "協調者",
        "workflow.props.roleTag": "角色: {{role}}",
        "workflow.props.selectRole": "選擇角色",
    },
    "ja": {
        "agentRole.browser": "ブラウザー",
        "agentRole.coordinator": "コーディネーター",
        "agentRole.developer": "開発者",
        "agentRole.executor": "実行者",
        "agentRole.planner": "プランナー",
        "agentRole.researcher": "リサーチャー",
        "agentRole.reviewer": "レビュアー",
        "agentRole.synthesizer": "シンセサイザー",
        "artifact.copiedToClipboard": "クリップボードにコピーしました",
        "artifact.copyFailed": "コピーに失敗しました",
        "chat.workflow.errorDetail.THINKING_BLOCK_END": "終了",
        "chat.workflow.errorDetail.THINKING_BLOCK_START": "開始",
        "common.key": "キー",
        "common.value": "値",
        "datasetManager.createdSuccess": "データセットを作成しました",
        "datasetManager.deletedSuccess": "データセットを削除しました",
        "datasetManager.sampleAdded": "サンプルを追加しました",
        "settings.workflow.createdSuccess": "作成しました",
        "settings.workflow.deletedSuccess": "削除しました",
        "settings.workflow.sampleAdded": "サンプルを追加しました",
        "stockAnalysis.sharesUnit": "株",
        "tray.key": "キー",
        "tray.value": "値",
        "workflow.agentNode.agent": "エージェント",
        "workflow.agentNode.roleBrowser": "ブラウザー",
        "workflow.agentNode.roleCoordinator": "コーディネーター",
        "workflow.props.bindRole": "ロールをバインド",
        "workflow.props.expertNotFound": "エキスパートが見つかりません",
        "workflow.props.profileCreateFailed": "プロファイル作成失敗: {{error}}",
        "workflow.props.profileCreated": "プロファイルを作成しました",
        "workflow.props.profileMatched": "既存のプロファイルに一致しました",
        "workflow.props.roleBrowser": "ブラウザー",
        "workflow.props.roleCoordinator": "コーディネーター",
        "workflow.props.roleTag": "ロール: {{role}}",
        "workflow.props.selectRole": "ロールを選択",
    },
}

# 对 ar/de/es/fr/hi/ko/ru 使用英文回退
EN = TRANSLATIONS["en-US"]

# 某些需要特殊处理的 key 在特定语言
SPECIAL = {
    "ko": {
        "stockAnalysis.sharesUnit": "주",
        "common.key": "키",
        "common.value": "값",
        "tray.key": "키",
        "tray.value": "값",
    },
}

def set_nested(d, key_path, value):
    """设置嵌套 JSON 中的值，key_path 用点号分隔"""
    keys = key_path.split(".")
    for k in keys[:-1]:
        d = d[k]
    d[keys[-1]] = value

def get_nested(d, key_path):
    keys = key_path.split(".")
    for k in keys:
        d = d[k]
    return d

def main():
    # 要处理的语言文件（排除 zh-CN，因为它是源）
    targets = ["zh-TW", "ja", "en-US", "ar", "de", "es", "fr", "hi", "ko", "ru"]

    for lang in targets:
        filepath = os.path.join(LOCALES_DIR, f"{lang}.json")
        with open(filepath, "r", encoding="utf-8") as f:
            data = json.load(f)

        updated = 0
        for key_path in TRANSLATIONS["en-US"].keys():
            try:
                current = get_nested(data, key_path)
            except (KeyError, TypeError):
                print(f"  [{lang}] SKIP missing key: {key_path}")
                continue

            if isinstance(current, str) and current.strip() == "":
                # 选择翻译来源
                if lang in SPECIAL and key_path in SPECIAL[lang]:
                    value = SPECIAL[lang][key_path]
                elif lang in TRANSLATIONS:
                    value = TRANSLATIONS[lang].get(key_path, EN[key_path])
                else:
                    value = EN[key_path]

                set_nested(data, key_path, value)
                updated += 1

        if updated > 0:
            with open(filepath, "w", encoding="utf-8") as f:
                json.dump(data, f, ensure_ascii=False, indent=2)
                f.write("\n")
            print(f"[{lang}] 已补齐 {updated} 个空 key")
        else:
            print(f"[{lang}] 无待补齐 key（所有 key 已有值）")

if __name__ == "__main__":
    main()
