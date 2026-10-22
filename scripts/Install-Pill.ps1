[CmdletBinding()]
param([switch]$SkipBuild, [switch]$SessionOnly)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$binDirectory = Join-Path $projectRoot 'bin'
$destination = Join-Path $binDirectory 'pill.exe'
$existing = Get-Command pill -ErrorAction SilentlyContinue
if ($existing -and ($existing.CommandType -ne 'Application' -or $existing.Source -ne $destination)) {
    throw "Ya existe un comando pill: $($existing.Source). Resuelve el conflicto antes de registrar este proyecto."
}

if (-not $SkipBuild) {
    Push-Location -LiteralPath $projectRoot
    try {
        & cargo build --release --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'La compilación ha fallado; no se registra pill.' }
    } finally { Pop-Location }
}

$source = Join-Path $projectRoot 'target\release\pill.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "No existe el binario compilado: $source" }
New-Item -ItemType Directory -Path $binDirectory -Force | Out-Null
Copy-Item -LiteralPath $source -Destination $destination -Force

# Conserva las entradas del usuario y no modifica perfiles de PowerShell.
if (($env:Path -split ';') -notcontains $binDirectory) { $env:Path += ';' + $binDirectory }
if (-not $SessionOnly) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($userPath -split ';' | Where-Object { $_ })
    if ($entries -notcontains $binDirectory) {
        $newPath = if ([string]::IsNullOrEmpty($userPath)) { $binDirectory } else { $userPath.TrimEnd(';') + ';' + $binDirectory }
        try { [Environment]::SetEnvironmentVariable('Path', $newPath, 'User') }
        catch { throw "Binario preparado y PATH de esta sesión actualizado. No se pudo guardar PATH de usuario: $($_.Exception.Message)" }
    }
}

Write-Output "Comando registrado: $destination"
if ($SessionOnly) { Write-Output 'Puedes escribir pill en esta sesión. No se ha cambiado el PATH persistente.' }
else { Write-Output 'Puedes escribir pill en esta sesión. Otras sesiones deben heredar el PATH actualizado de usuario.' }
