$ErrorActionPreference = 'Stop'
$root = "d:\OneManager\AxInvest\src\i18n\locales"
Get-ChildItem -Path $root -Filter "*.json" | ForEach-Object {
  $raw = Get-Content $_.FullName -Raw
  try {
    $null = $raw | ConvertFrom-Json
    Write-Host ("OK  " + $_.Name)
  } catch {
    Write-Host ("BAD " + $_.Name + " " + $_.Exception.Message)
  }
}
