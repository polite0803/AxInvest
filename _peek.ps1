$path = 'C:\Users\polit\Downloads\A股反思复盘.json'
$lines = Get-Content $path -TotalCount 16
for ($i=0; $i -lt $lines.Count; $i++) {
    Write-Host ("L{0}: {1}" -f ($i+1), $lines[$i])
}
Write-Host "---"
Write-Host ("File size: {0} bytes" -f (Get-Item $path).Length)
