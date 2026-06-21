Get-ChildItem -Path 'd:\OneManager\AxInvest\src-tauri\agency_experts\stock-analysis\*.md' | ForEach-Object {
    $content = Get-Content $_.FullName -Raw
    $matches = [regex]::Matches($content, '\{\{[^}]*\}\}')
    foreach ($m in $matches) {
        $val = $m.Value
        # 提取 {{...}} 中间内容
        $inner = $val.Substring(2, $val.Length - 4).Trim()
        $isValid = $inner -match '^[a-zA-Z_][a-zA-Z0-9_.]*$' -or $inner.StartsWith('if ') -or $inner.StartsWith('/')
        if (-not $isValid) {
            Write-Host ("FILE: {0} | VAL: {1} | INNER: '{2}'" -f $_.Name, $val, $inner)
        }
    }
}
Write-Host "---DONE---"
