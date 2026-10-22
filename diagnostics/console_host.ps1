param([string]$Directory, [string]$PillBinary, [string]$Project)
$ErrorActionPreference = 'Stop'
$env:Path = (Split-Path -Parent $PillBinary) + ';' + $env:Path
Set-Content -LiteralPath (Join-Path $Directory 'ready') -Value $PID
while (-not (Test-Path -LiteralPath (Join-Path $Directory 'start'))) { Start-Sleep -Milliseconds 20 }
& pill --project $Project
$code = $LASTEXITCODE
Set-Content -LiteralPath (Join-Path $Directory 'result') -Value $code
while (-not (Test-Path -LiteralPath (Join-Path $Directory 'finish'))) { Start-Sleep -Milliseconds 20 }
exit $code
