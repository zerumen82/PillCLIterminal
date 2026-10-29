[CmdletBinding(SupportsShouldProcess, ConfirmImpact='Medium')]
param(
    [switch]$SkipBuild,
    [switch]$SessionOnly,
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedPreviousHash
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'Pill.Installation.psm1') -Force
$context = Get-PillContext
$scope = if ($SessionOnly) { 'Process' } else { 'User' }
Assert-PillCommand $context
$processBefore = Get-PillPathSnapshot 'Process'
$processPlan = Get-PillPathAddition $processBefore.Value $processBefore.Exists $context.Bin -ExpandVariables $false
$userBefore = if (-not $SessionOnly) { Get-PillPathSnapshot 'User' } else { $null }
$userPlan = if ($null -ne $userBefore) { Get-PillPathAddition $userBefore.Value $userBefore.Exists $context.Bin -ExpandVariables ($userBefore.Kind -eq 'ExpandString') } else { $null }
if (-not $PSCmdlet.ShouldProcess($context.Binary, "Preparar pill y registrar PATH $scope sin sustituir entradas ajenas")) {
    return [pscustomobject]@{ Status='WhatIf'; Scope=$scope; Binary=$context.Binary; ProcessPathWouldAdd=$processPlan.Added; UserPathWouldAdd=($null -ne $userPlan -and $userPlan.Added) }
}

Initialize-PillNative
$mutex = Enter-PillInstallMutex $context.Settings.mutex_timeout_ms
$pins = $null; $source = $null; $stage = $null
$processWritten = $false; $userWritten = $false
$processAfter = $null; $userAfter = $null
try {
    $pins = [PillInstallation.Native]::PinDirectories($context.Root)
    if (-not $SkipBuild) {
        Push-Location -LiteralPath $context.Root
        try {
            & cargo build --release --locked --offline
            if ($LASTEXITCODE -ne 0) { throw 'Falló build; no se registra pill.' }
        } finally { Pop-Location }
    }
    if (-not [IO.File]::Exists($context.Source)) { throw "No existe el binario compilado: $($context.Source)" }
    $sourcePins = [PillInstallation.Native]::PinDirectories([IO.Path]::GetDirectoryName($context.Source))
    foreach ($pin in $sourcePins) { $pins.Add($pin) }
    $source = [PillInstallation.Native]::OpenBuildArtifact($context.Source)
    $sourceHash = Get-PillStreamHash $source
    $version = Get-PillPackageVersion $context
    if (-not [IO.Directory]::Exists($context.Bin)) { $null = [IO.Directory]::CreateDirectory($context.Bin) }
    $binPins = [PillInstallation.Native]::PinDirectories($context.Bin)
    foreach ($pin in $binPins) { $pins.Add($pin) }
    $oldManifest = Read-PillManifest $context
    $previousData = if ($null -ne $oldManifest) { $oldManifest.Data } else { $null }
    $manifestHash = if ($null -ne $oldManifest) { $oldManifest.Hash } else { $null }
    $oldHash = $null; $oldIdentity=$null
    if ([IO.File]::Exists($context.Binary)) {
        $old = [PillInstallation.Native]::OpenRegular($context.Binary,$false)
        try { $oldHash = Get-PillStreamHash $old; $oldIdentity=[PillInstallation.Native]::Identity($old) } finally { $old.Dispose() }
        if ($null -ne $previousData -and $previousData.state -in @('complete','unregistered') -and $oldIdentity -cne $previousData.binary_identity -and
            (-not $ExpectedPreviousHash -or $oldHash -ine $ExpectedPreviousHash)) {throw 'El objeto binario fue sustituido; requiere adopción explícita con su SHA actual.'}
        $knownHash = if ($null -ne $previousData -and $previousData.state -in @('complete','unregistered')) { $previousData.binary_sha256 } else { $ExpectedPreviousHash }
        if ($oldHash -cne $sourceHash -and (-not $knownHash -or $oldHash -ine $knownHash)) {
            throw "El binario existente no está acreditado. Revisa su SHA256 $oldHash y aporta -ExpectedPreviousHash para adoptar esa versión concreta."
        }
    }
    if (-not (Test-PillSameSnapshot (Get-PillPathSnapshot 'Process') $processBefore)) { throw 'El PATH de proceso cambió durante la preparación.' }
    if ($null -ne $userBefore -and -not (Test-PillSameSnapshot (Get-PillPathSnapshot 'User') $userBefore)) { throw 'El PATH de usuario cambió durante la preparación.' }
    Assert-PillCommand $context
    $identity = Get-PillProcessIdentity
    $oldProcess = if ($null -ne $previousData -and $previousData.state -ne 'prepared') { $previousData.scopes['Process'] } else { $null }
    $oldUser = if ($null -ne $previousData -and $previousData.state -ne 'prepared') { $previousData.scopes['User'] } else { $null }
    $processOwnership = Get-PillOwnership $processBefore $processPlan $oldProcess 'Process' $identity
    $userOwnership = if ($null -ne $userPlan) { Get-PillOwnership $userBefore $userPlan $oldUser 'User' $identity } else { $oldUser }
    $data = [ordered]@{
        schema=1; project_root=$context.Root; binary=$context.Binary; binary_sha256=$sourceHash; binary_identity=$oldIdentity;
        version=$version; install_id=$(if ($null -ne $previousData) { $previousData.install_id } else { [guid]::NewGuid().ToString('D') });
        state='prepared'; scopes=[ordered]@{ User=$userOwnership; Process=$processOwnership }
    }
    if ($oldHash -ceq $sourceHash -and $null -ne $previousData -and $previousData.state -eq 'complete' -and
        -not $processPlan.Added -and ($null -eq $userPlan -or -not $userPlan.Added)) {
        $comparison=[ordered]@{}; foreach($key in $data.Keys) {$comparison[$key]=$data[$key]}; $comparison.state='complete'
        if (($comparison|ConvertTo-Json -Depth 12 -Compress) -ceq ($previousData|ConvertTo-Json -Depth 12 -Compress)) {
            $stage=[PillInstallation.Native]::OpenRegular($context.Binary,$false)
            if ((Get-PillStreamHash $stage) -cne $sourceHash) {throw 'El binario cambió al verificar la instalación existente.'}
            Test-PillBinaryVersion $context.Binary $version $context.Settings.verification_timeout_ms
            Assert-PillCommand $context -MustResolve
            return [pscustomobject]@{Status='Installed';Scope=$scope;Binary=$context.Binary;Sha256=$sourceHash;Version=$version;
                UserPathAdded=$false;ProcessPathAdded=$false;Manifest=$context.Manifest;PreviousBinary=$null;EnvironmentNotified=$null;Idempotent=$true}
        }
    }
    $temporary = Join-Path $context.Bin ('pill.'+[guid]::NewGuid().ToString('N')+'.new.exe')
    $writer = [PillInstallation.Native]::OpenRegular($temporary,$true)
    try {
        $source.Position=0; $source.CopyTo($writer)
        if ($oldHash) {
            $old=[PillInstallation.Native]::OpenRegular($context.Binary,$false)
            try {
                if ((Get-PillStreamHash $old) -cne $oldHash -or [PillInstallation.Native]::Identity($old) -cne $oldIdentity) {throw 'El binario cambió antes de conservar sus permisos.'}
                [PillInstallation.Native]::CopySecurity($old,$writer)
            } finally {$old.Dispose()}
        }
        $writer.Flush($true)
    } finally { $writer.Dispose() }
    $stage = [PillInstallation.Native]::OpenRegular($temporary,$false)
    if ((Get-PillStreamHash $stage) -cne $sourceHash) { throw 'La copia preparada no coincide con el binario compilado.' }
    Test-PillBinaryVersion $temporary $version $context.Settings.verification_timeout_ms
    $data.binary_identity=[PillInstallation.Native]::Identity($stage)
    $manifestHash = Save-PillManifest $context $data $manifestHash
    $backup = Publish-PillStage $context $stage $context.Binary $oldHash $oldIdentity
    if ((Get-PillStreamHash $stage) -cne $sourceHash) { throw 'El binario publicado no conserva el SHA comprobado.' }
    if ($null -ne $userPlan -and $userPlan.Added) {
        $userAfter = New-PillDesiredSnapshot $userBefore $userPlan.Value $userPlan.Exists
        Set-PillPathSnapshot $userBefore $userAfter
        $userWritten = $true
    }
    if ($processPlan.Added) {
        $processAfter = New-PillDesiredSnapshot $processBefore $processPlan.Value $processPlan.Exists
        Set-PillPathSnapshot $processBefore $processAfter
        $processWritten = $true
    }
    Assert-PillCommand $context -MustResolve
    $data.state='complete'
    $manifestHash = Save-PillManifest $context $data $manifestHash
    $notified = if (-not $SessionOnly) { [PillInstallation.Native]::Notify([uint32]$context.Settings.notification_timeout_ms) } else { $null }
    [pscustomobject]@{ Status='Installed'; Scope=$scope; Binary=$context.Binary; Sha256=$sourceHash; Version=$version;
        UserPathAdded=($null -ne $userPlan -and $userPlan.Added); ProcessPathAdded=$processPlan.Added;
        Manifest=$context.Manifest; PreviousBinary=$backup; EnvironmentNotified=$notified; Idempotent=$false }
} catch {
    $failure = $_
    $rollbacks = [Collections.Generic.List[string]]::new()
    if ($processWritten) {
        try { if (Undo-PillPathChange $processBefore $processAfter) { $rollbacks.Add('Process restaurado') } else { $rollbacks.Add('Process cambió; conservado') } }
        catch { $rollbacks.Add("Process sin confirmar: $($_.Exception.Message)") }
    }
    if ($userWritten) {
        try { if (Undo-PillPathChange $userBefore $userAfter) { $rollbacks.Add('User restaurado') } else { $rollbacks.Add('User cambió; conservado') } }
        catch { $rollbacks.Add("User sin confirmar: $($_.Exception.Message)") }
    }
    throw "Instalación no completada: $($failure.Exception.Message). $($rollbacks -join '; '). Revisa el manifiesto/reservas de $($context.Bin); no se restauran archivos encima de cambios posteriores."
} finally {
    if ($null -ne $stage) { $stage.Dispose() }
    if ($null -ne $source) { $source.Dispose() }
    if ($null -ne $pins) { foreach ($pin in $pins) { $pin.Dispose() } }
    $mutex.ReleaseMutex(); $mutex.Dispose()
}
