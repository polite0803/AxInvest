import json
zh = json.load(open(r'd:\OneManager\AxInvest\src\i18n\locales\zh-CN.json', encoding='utf-8'))
en = json.load(open(r'd:\OneManager\AxInvest\src\i18n\locales\en-US.json', encoding='utf-8'))
for k in ['run', 'save', 'settings']:
    print(f'  zh.workflow.{k} = {zh["workflow"].get(k)}')
    print(f'  en.workflow.{k} = {en["workflow"].get(k)}')
    print()
