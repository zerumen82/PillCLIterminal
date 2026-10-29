[CmdletBinding(SupportsShouldProcess, ConfirmImpact='Medium')]
param([switch]$SessionOnly, [switch]$RemoveBinary)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'Pill.Installation.psm1') -Force
$context = Get-PillContext
$scope = if ($SessionOnly) { 'Process' } else { 'User' }
if ($WhatIfPreference) {
    $null = $PSCmdlet.ShouldProcess($context.Bin,"Retirar solo el PATH $scope acreditado por el manifiesto; borrar binario=$([bool]$RemoveBinary)")
    return [pscustomobject]@{ Status='WhatIf'; Scope=$scope; Binary=$context.Binary; RemoveBinary=[bool]$RemoveBinary }
}
Initialize-PillNative
$mutex = Enter-PillInstallMutex $context.Settings.mutex_timeout_ms
$pins=$null; $binary=$null; $changes=[Collections.Generic.List[object]]::new()
try {
    $pins=[PillInstallation.Native]::PinDirectories($context.Root)
    if (-not [IO.Directory]::Exists($context.Bin)) { return [pscustomobject]@{ Status='NotRegistered'; Scope=$scope; BinaryRemoved=$false } }
    $binPins=[PillInstallation.Native]::PinDirectories($context.Bin)
    foreach($pin in $binPins) { $pins.Add($pin) }
    $manifest=Read-PillManifest $context
    if ($null -eq $manifest) { throw 'No hay manifiesto de propiedad; no se retiran rutas ni archivos por suposición.' }
    if ($manifest.Data.state -eq 'prepared') { throw 'Hay instalación incompleta; revisa su evidencia antes de desregistrar.' }
    $data=$manifest.Data
    $identity=Get-PillProcessIdentity
    $plans=[Collections.Generic.List[object]]::new()
    foreach($targetScope in $(if($SessionOnly) { @('Process') } else { @('User','Process') })) {
        $ownership=$data.scopes[$targetScope]
        if ($null -eq $ownership -or -not $ownership.added) { continue }
        if ($targetScope -eq 'Process' -and ($ownership.process_id -ne $identity.Id -or $ownership.creation_ticks -ne $identity.Creation)) { continue }
        $before=Get-PillPathSnapshot $targetScope
        $plan=Get-PillPathRemoval $before.Value $before.Exists $context.Bin $true $ownership.raw_entry $ownership.before_exists -ExpandVariables ($targetScope -eq 'User' -and $before.Kind -eq 'ExpandString')
        $after=New-PillDesiredSnapshot $before $plan.Value $plan.Exists
        $plans.Add([pscustomobject]@{ Before=$before; After=$after; Scope=$targetScope })
    }
    if ($RemoveBinary -and [IO.File]::Exists($context.Binary)) {
        $binary=[PillInstallation.Native]::OpenRegular($context.Binary,$false)
        if ((Get-PillStreamHash $binary) -cne $data.binary_sha256 -or [PillInstallation.Native]::Identity($binary) -cne $data.binary_identity) { throw 'El binario cambió de contenido o identidad; no se elimina.' }
    }
    $action = "Retirar $($plans.Count) entrada(s) propia(s) del PATH; borrar binario=$([bool]$RemoveBinary)"
    if (-not $PSCmdlet.ShouldProcess($context.Bin,$action)) { return [pscustomobject]@{ Status='Cancelled'; Scope=$scope; BinaryRemoved=$false } }
    foreach($plan in $plans) {
        Set-PillPathSnapshot $plan.Before $plan.After
        $changes.Add($plan)
        $data.scopes[$plan.Scope].added=$false
    }
    $removed=$false
    if ($null -ne $binary) {
        [PillInstallation.Native]::DeleteOwned($binary)
        $binary.Dispose(); $binary=$null
        if ([IO.File]::Exists($context.Binary)) { throw 'El archivo propio se eliminó, pero apareció otro destino; se conserva y requiere revisión.' }
        $removed=$true
    }
    $data.state='unregistered'
    $null=Save-PillManifest $context $data $manifest.Hash
    $notified=if(-not $SessionOnly -and @($plans|Where-Object Scope -eq 'User').Count) { [PillInstallation.Native]::Notify([uint32]$context.Settings.notification_timeout_ms) } else { $null }
    [pscustomobject]@{ Status='Unregistered'; Scope=$scope; PathEntriesRemoved=$plans.Count; BinaryRemoved=$removed;
        Binary=$context.Binary; Manifest=$context.Manifest; EnvironmentNotified=$notified }
} catch {
    $failure=$_; $notes=[Collections.Generic.List[string]]::new()
    for($index=$changes.Count-1;$index -ge 0;$index--) {
        try { if(Undo-PillPathChange $changes[$index].Before $changes[$index].After) { $notes.Add($changes[$index].Scope+' restaurado') } else { $notes.Add($changes[$index].Scope+' cambió; conservado') } }
        catch { $notes.Add($_.Exception.Message) }
    }
    throw "Desregistro no completado: $($failure.Exception.Message). $($notes -join '; '). Conserva el manifiesto y las reservas para revisar el estado."
} finally {
    if ($null -ne $binary) { $binary.Dispose() }
    if ($null -ne $pins) { foreach($pin in $pins) { $pin.Dispose() } }
    $mutex.ReleaseMutex(); $mutex.Dispose()
}
