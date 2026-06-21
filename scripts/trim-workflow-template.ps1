$path = 'd:\OneManager\AxInvest\src-tauri\src\commands\workflow_template.rs'
$lines = [System.IO.File]::ReadAllLines($path, [System.Text.Encoding]::UTF8)
Write-Host "Total lines: $($lines.Length)"
$head = $lines[0..2340]
$enc = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllLines($path, $head, $enc)
$verify = [System.IO.File]::ReadAllLines($path, [System.Text.Encoding]::UTF8)
Write-Host "After trim: $($verify.Length) lines, file size: $((Get-Item $path).Length) bytes"
