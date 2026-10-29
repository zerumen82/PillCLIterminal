[CmdletBinding()]
param([string]$Evidence)

$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$projectRoot=Split-Path -Parent $PSScriptRoot
$modulePath=Join-Path $projectRoot 'scripts/Pill.Installation.psm1'
Import-Module $modulePath -Force
$settings=Get-PillInstallationSettings
$sourceBinary=Join-Path $projectRoot 'target/release/pill.exe'
if (-not [IO.File]::Exists($sourceBinary)) { throw 'Se requiere el release real; esta prueba no compila ni instala paquetes.' }
$originalProcess=Get-PillPathSnapshot 'Process'
$originalUser=Get-PillPathSnapshot 'User'
$originalMachine=[Environment]::GetEnvironmentVariable('Path','Machine')
$temporary=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('pill-install-tests-'+[guid]::NewGuid().ToString('N'))))
$checks=[Collections.Generic.List[object]]::new()
$fixtures=[Collections.Generic.List[string]]::new()
$report=[ordered]@{ fecha_utc=[DateTimeOffset]::UtcNow.ToString('O'); result='FAIL'; checks=$checks; temporary=$temporary;
    source_sha256=(Get-FileHash -LiteralPath $sourceBinary -Algorithm SHA256).Hash.ToLowerInvariant(); scope='PATH de proceso y funciones puras; binarios reales en muestras. User/Machine PATH solo se leen.' }

function Assert-Install {
    param([bool]$Condition,[string]$Name)
    $checks.Add([ordered]@{name=$Name;pass=$Condition})
    if(-not $Condition) {throw "FAIL: $Name"}
}
function Assert-InstallThrows {
    param([scriptblock]$Action,[string]$Name)
    $threw=$false
    try {& $Action | Out-Null} catch {$threw=$true}
    Assert-Install $threw $Name
}
function New-InstallFixture {
    param([string]$Name)
    $folder=Join-Path $temporary $Name
    foreach($part in @('scripts','target/release')) { $null=[IO.Directory]::CreateDirectory((Join-Path $folder $part)) }
    foreach($name in @('Install-Pill.ps1','Uninstall-Pill.ps1','Pill.Installation.psm1','installation.defaults.json')) {
        [IO.File]::Copy((Join-Path $projectRoot "scripts/$name"),(Join-Path $folder "scripts/$name"),$false)
    }
    [IO.File]::Copy((Join-Path $projectRoot 'Cargo.toml'),(Join-Path $folder 'Cargo.toml'),$false)
    [IO.File]::Copy($sourceBinary,(Join-Path $folder 'target/release/pill.exe'),$false)
    $fixtures.Add($folder)
    return $folder
}
function Invoke-FixtureInstall {
    param([string]$Folder,[switch]$WhatIf)
    & (Join-Path $Folder 'scripts/Install-Pill.ps1') -SkipBuild -SessionOnly -WhatIf:$WhatIf
}
function Invoke-FixtureUninstall {
    param([string]$Folder,[switch]$WhatIf,[switch]$RemoveBinary)
    & (Join-Path $Folder 'scripts/Uninstall-Pill.ps1') -SessionOnly -WhatIf:$WhatIf -RemoveBinary:$RemoveBinary
}

try {
    $entry=Join-Path $projectRoot 'bin'
    $raw='primera;;segunda;'
    $added=Get-PillPathAddition $raw $true $entry
    Assert-Install ($added.Value -ceq ($raw+';'+$entry)) 'añadir conserva texto, orden y entradas vacías'
    $removed=Get-PillPathRemoval $added.Value $true $entry $true $entry $true
    Assert-Install ($removed.Value -ceq $raw) 'retirar devuelve exactamente el PATH anterior'
    $caseVariant='"'+$entry.ToUpperInvariant()+'\"'
    $exists=Get-PillPathAddition $caseVariant $true $entry
    Assert-Install (-not $exists.Added -and $exists.Value -ceq $caseVariant) 'citas, mayúsculas y barra final no duplican'
    $env:PILL_INSTALL_TEST_ENTRY=$entry
    $expanded=Get-PillPathAddition '%PILL_INSTALL_TEST_ENTRY%' $true $entry
    Assert-Install (-not $expanded.Added -and $expanded.Value -ceq '%PILL_INSTALL_TEST_ENTRY%') 'expansión equivalente conserva expresión original'
    $literal=Get-PillPathAddition '%PILL_INSTALL_TEST_ENTRY%' $true $entry -ExpandVariables $false
    Assert-Install ($literal.Added -and $literal.Value -ceq ('%PILL_INSTALL_TEST_ENTRY%;'+$entry)) 'REG_SZ o proceso literal no finge expansión persistente'
    $absent=Get-PillPathAddition $null $false $entry
    $backToAbsent=Get-PillPathRemoval $absent.Value $true $entry $true $entry $false
    Assert-Install (-not $backToAbsent.Exists) 'retirar alta desde PATH ausente conserva ausencia'
    $unowned=Get-PillPathRemoval $entry $true $entry $false $null $true
    Assert-Install ($unowned.Value -ceq $entry -and -not $unowned.Removed) 'entrada preexistente no pasa a propiedad'
    Assert-InstallThrows {Get-PillPathRemoval ($entry+';'+$entry) $true $entry $true $entry $true} 'duplicado ambiguo se conserva'
    Assert-InstallThrows {Get-PillPathRemoval $caseVariant $true $entry $true $entry $true} 'entrada modificada no se retira por aproximación'
    Assert-InstallThrows {Get-PillPathRemoval $entry $true $entry $true 'C:\ajeno' $true} 'manifiesto no acredita ruta ajena'
    $env:Path='antes;otra'
    $expected=Get-PillPathSnapshot 'Process'
    $desired=New-PillDesiredSnapshot $expected 'antes;otra;propia' $true
    $env:Path='antes;concurrente'
    Assert-InstallThrows {Set-PillPathSnapshot $expected $desired} 'cambio concurrente se detecta antes de escribir'
    Assert-Install ($env:Path -ceq 'antes;concurrente') 'conflicto no sobreescribe cambio externo'
    Assert-Install (-not (Undo-PillPathChange $expected $desired)) 'rollback no pisa versión diferente'

    $env:Path='primera;;segunda;'
    $fixture=New-InstallFixture 'proyecto ñ'
    $beforeWhatIf=$env:Path
    $plan=Invoke-FixtureInstall $fixture -WhatIf
    Assert-Install ($plan.Status -eq 'WhatIf' -and $env:Path -ceq $beforeWhatIf -and -not [IO.Directory]::Exists((Join-Path $fixture 'bin'))) 'instalación WhatIf no crea bin ni cambia PATH'
    $installed=Invoke-FixtureInstall $fixture
    Assert-Install ($installed.Status -eq 'Installed' -and $installed.ProcessPathAdded) 'instalación real de proceso completa'
    $fixtureBin=Join-Path $fixture 'bin'
    $fixtureBinary=Join-Path $fixtureBin 'pill.exe'
    Assert-Install ((Get-Command pill -ErrorAction Stop).Source -ceq $fixtureBinary) 'pill se resuelve al binario publicado'
    Assert-Install ((& $fixtureBinary --version).Trim() -ceq ('pill '+$installed.Version)) 'binario publicado ejecuta versión real'
    Assert-Install ((Get-FileHash -LiteralPath $fixtureBinary).Hash.ToLowerInvariant() -ceq $report.source_sha256) 'publicación conserva SHA del release real'
    $pathAfter=$env:Path
    $manifestBefore=Get-Content -LiteralPath (Join-Path $fixtureBin 'installation.json') -Raw | ConvertFrom-Json
    $again=Invoke-FixtureInstall $fixture
    $manifestAfter=Get-Content -LiteralPath (Join-Path $fixtureBin 'installation.json') -Raw | ConvertFrom-Json
    Assert-Install ($again.Idempotent -and -not $again.ProcessPathAdded -and $env:Path -ceq $pathAfter -and $manifestBefore.install_id -eq $manifestAfter.install_id -and $manifestAfter.scopes.Process.added) 'reinstalar es idempotente y conserva propiedad'
    $simulate=Invoke-FixtureUninstall $fixture -WhatIf -RemoveBinary
    Assert-Install ($simulate.Status -eq 'WhatIf' -and [IO.File]::Exists($fixtureBinary) -and $env:Path -ceq $pathAfter) 'desinstalación WhatIf no cambia archivos ni PATH'
    $env:Path += ';añadida-por-otro'
    $unregistered=Invoke-FixtureUninstall $fixture
    Assert-Install ($unregistered.Status -eq 'Unregistered' -and $unregistered.PathEntriesRemoved -eq 1 -and [IO.File]::Exists($fixtureBinary)) 'desregistrar retira solo entrada propia y conserva binario'
    Assert-Install ($env:Path -ceq 'primera;;segunda;;añadida-por-otro') 'desregistro conserva cambios ajenos posteriores y vacíos'
    $removedBinary=Invoke-FixtureUninstall $fixture -RemoveBinary
    Assert-Install ($removedBinary.BinaryRemoved -and -not [IO.File]::Exists($fixtureBinary)) 'borrado explícito elimina únicamente binario acreditado'
    Assert-Install ([IO.File]::Exists((Join-Path $fixture 'Cargo.toml')) -and [IO.File]::Exists((Join-Path $fixture 'target/release/pill.exe'))) 'borrado no toca proyecto ni release'

    $collisionFolder=Join-Path $temporary 'colision-publicacion'
    $null=[IO.Directory]::CreateDirectory($collisionFolder)
    $collisionSource=Join-Path $collisionFolder 'nuevo.txt'
    $collisionDestination=Join-Path $collisionFolder 'ocupante.txt'
    [IO.File]::WriteAllText($collisionSource,'nuevo propio')
    [IO.File]::WriteAllText($collisionDestination,'ocupante ajeno')
    $locked=[PillInstallation.Native]::OpenRegular($collisionSource,$false)
    try { Assert-InstallThrows {[PillInstallation.Native]::RenameNoReplace($locked,$collisionDestination)} 'publicación rechaza destino ocupado sin reemplazarlo' }
    finally {$locked.Dispose()}
    Assert-Install ([IO.File]::ReadAllText($collisionSource) -ceq 'nuevo propio' -and [IO.File]::ReadAllText($collisionDestination) -ceq 'ocupante ajeno') 'colisión preserva ambos archivos reales'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'preexistente'
    $preexisting=Join-Path $fixture 'bin'
    $env:Path += ';'+$preexisting
    $before=$env:Path
    $installed=Invoke-FixtureInstall $fixture
    $unregistered=Invoke-FixtureUninstall $fixture
    Assert-Install (-not $installed.ProcessPathAdded -and $unregistered.PathEntriesRemoved -eq 0 -and $env:Path -ceq $before) 'PATH preexistente no se reclama ni elimina'
    $upgradeBinary=Join-Path $fixture 'bin/pill.exe'
    $oldBinaryHash=(Get-FileHash -LiteralPath $upgradeBinary).Hash.ToLowerInvariant()
    $security=Get-Acl -LiteralPath $upgradeBinary
    $security.SetAccessRuleProtection($true,$true)
    Set-Acl -LiteralPath $upgradeBinary -AclObject $security
    $securityBefore=(Get-Acl -LiteralPath $upgradeBinary).GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]'Owner,Group,Access')
    [IO.File]::AppendAllText((Join-Path $fixture 'target/release/pill.exe'),'cambio de compilación descartable')
    $updated=Invoke-FixtureInstall $fixture
    Assert-Install ($updated.Sha256 -cne $oldBinaryHash -and [IO.File]::Exists($updated.PreviousBinary) -and (Get-FileHash -LiteralPath $updated.PreviousBinary).Hash.ToLowerInvariant() -ceq $oldBinaryHash) 'actualizar conserva versión anterior real recuperable'
    $securityAfter=(Get-Acl -LiteralPath $upgradeBinary).GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]'Owner,Group,Access')
    Assert-Install ($securityBefore -ceq $securityAfter) 'actualizar conserva propietario, grupo y DACL protegida'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'identidad-sustituida'
    $null=Invoke-FixtureInstall $fixture
    $sameBytesBinary=Join-Path $fixture 'bin/pill.exe'
    [IO.File]::Move($sameBytesBinary,(Join-Path $fixture 'bin/pill-desplazado.exe'))
    [IO.File]::Copy($sourceBinary,$sameBytesBinary,$false)
    $before=$env:Path
    Assert-InstallThrows {Invoke-FixtureUninstall $fixture -RemoveBinary} 'sustitución por mismos bytes no hereda identidad para borrar'
    Assert-Install ($env:Path -ceq $before -and [IO.File]::Exists($sameBytesBinary)) 'sustitución conserva PATH y nuevo objeto'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'binario-ajeno'
    $null=[IO.Directory]::CreateDirectory((Join-Path $fixture 'bin'))
    $foreign=Join-Path $fixture 'bin/pill.exe'
    [IO.File]::WriteAllText($foreign,'archivo del usuario, no es ejecutable')
    $foreignBefore=(Get-FileHash -LiteralPath $foreign).Hash
    Assert-InstallThrows {Invoke-FixtureInstall $fixture} 'binario ajeno no acreditado impide sustitución'
    Assert-Install ((Get-FileHash -LiteralPath $foreign).Hash -ceq $foreignBefore -and $env:Path -ceq 'primera;segunda') 'rechazo conserva archivo ajeno y PATH'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'manifiesto-ajeno'
    $null=Invoke-FixtureInstall $fixture
    $manifestPath=Join-Path $fixture 'bin/installation.json'
    $data=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -AsHashtable
    $data.project_root=$temporary
    [IO.File]::WriteAllText($manifestPath,($data|ConvertTo-Json -Depth 12))
    $before=$env:Path
    Assert-InstallThrows {Invoke-FixtureUninstall $fixture -RemoveBinary} 'manifiesto de otra raíz no concede retirada ni borrado'
    Assert-Install ($env:Path -ceq $before -and [IO.File]::Exists((Join-Path $fixture 'bin/pill.exe'))) 'rechazo de manifiesto conserva instalación'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'hardlink'
    $null=[IO.Directory]::CreateDirectory((Join-Path $fixture 'bin'))
    $outside=Join-Path $temporary 'externo.txt'
    [IO.File]::WriteAllText($outside,'externo intacto')
    $null=New-Item -ItemType HardLink -Path (Join-Path $fixture 'bin/pill.exe') -Target $outside
    Assert-InstallThrows {Invoke-FixtureInstall $fixture} 'hardlink de destino se rechaza antes de alterar contenido'
    Assert-Install ([IO.File]::ReadAllText($outside) -ceq 'externo intacto') 'bytes del hardlink externo intactos'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'junction'
    $outsideDirectory=Join-Path $temporary 'directorio-externo'
    $null=[IO.Directory]::CreateDirectory($outsideDirectory)
    $null=New-Item -ItemType Junction -Path (Join-Path $fixture 'bin') -Target $outsideDirectory
    Assert-InstallThrows {Invoke-FixtureInstall $fixture} 'junction bin se rechaza sin publicar fuera de raíz'
    Assert-Install (@(Get-ChildItem -LiteralPath $outsideDirectory -Force).Count -eq 0) 'directorio externo no alterado'
    [IO.Directory]::Delete((Join-Path $fixture 'bin'))

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'binario-modificado'
    $null=Invoke-FixtureInstall $fixture
    $binary=Join-Path $fixture 'bin/pill.exe'
    [IO.File]::AppendAllText($binary,'cambio posterior')
    $before=$env:Path
    Assert-InstallThrows {Invoke-FixtureUninstall $fixture -RemoveBinary} 'binario cambiado impide borrado antes de retirar PATH'
    Assert-Install ($env:Path -ceq $before -and [IO.File]::Exists($binary)) 'cambio posterior conservado'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'version-incorrecta'
    $cargoFile=Join-Path $fixture 'Cargo.toml'
    $cargoText=[IO.File]::ReadAllText($cargoFile)
    $cargoText=[regex]::Replace($cargoText,'(?m)^version\s*=\s*"[^"]+"','version = "999.0.0"',1)
    [IO.File]::WriteAllText($cargoFile,$cargoText)
    Assert-InstallThrows {Invoke-FixtureInstall $fixture} 'versión real distinta al manifiesto impide publicar'
    Assert-Install (-not [IO.File]::Exists((Join-Path $fixture 'bin/pill.exe')) -and $env:Path -ceq 'primera;segunda') 'versión errónea no registra ni publica pill'

    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'comando-conflictivo'
    Set-Alias -Name pill -Value Get-Date -Scope Global
    try { Assert-InstallThrows {Invoke-FixtureInstall $fixture} 'alias pill existente impide instalación sin cambiarlo' }
    finally {if ((Get-Alias pill -ErrorAction SilentlyContinue).Definition -eq 'Get-Date') {Remove-Item -LiteralPath 'Alias:pill' -Force}}
    Assert-Install (-not [IO.Directory]::Exists((Join-Path $fixture 'bin')) -and $env:Path -ceq 'primera;segunda') 'conflicto se rechaza antes de crear bin o tocar PATH'
    $env:Path='primera;segunda'
    $fixture=New-InstallFixture 'cargo-hardlink-source'
    $artifact=Join-Path $fixture 'target/release/pill.exe'
    $aliasDirectory=Join-Path $fixture 'target/release/deps'
    $null=[IO.Directory]::CreateDirectory($aliasDirectory)
    $artifactAlias=Join-Path $aliasDirectory 'pill.exe'
    $null=New-Item -ItemType HardLink -Path $artifactAlias -Target $artifact
    $artifactHash=(Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash
    $lockedArtifact=[PillInstallation.Native]::OpenBuildArtifact($artifact)
    try {
        Assert-InstallThrows {$writer=[IO.File]::Open($artifactAlias,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::Read);$writer.Dispose()} 'artefacto bloquea escritura por otro hardlink mientras se lee'
    } finally {$lockedArtifact.Dispose()}
    Assert-InstallThrows {$strict=[PillInstallation.Native]::OpenRegular($artifact,$false);$strict.Dispose()} 'lector estricto mantiene rechazo de hardlinks'
    $installedArtifact=Invoke-FixtureInstall $fixture
    Assert-Install ($installedArtifact.Status -eq 'Installed' -and (Get-FileHash -LiteralPath (Join-Path $fixture 'bin/pill.exe') -Algorithm SHA256).Hash -ceq $artifactHash) 'instalar artefacto Cargo con dos enlaces copia bytes verificados'
    $null=Invoke-FixtureUninstall $fixture -RemoveBinary
    Assert-Install ((Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash -ceq $artifactHash -and (Get-FileHash -LiteralPath $artifactAlias -Algorithm SHA256).Hash -ceq $artifactHash) 'install y uninstall de muestra conservan ambos alias del artefacto'
    $report.result='PASS' 
} catch {
    $report.error=$_.Exception.ToString()
    throw
} finally {
    [Environment]::SetEnvironmentVariable('Path',$(if($originalProcess.Exists){$originalProcess.Value}else{$null}),'Process')
    [Environment]::SetEnvironmentVariable('PILL_INSTALL_TEST_ENTRY',$null,'Process')
    Import-Module $modulePath -Force
    $userSame=Test-PillSameSnapshot (Get-PillPathSnapshot 'User') $originalUser
    $machineSame=[Environment]::GetEnvironmentVariable('Path','Machine') -ceq $originalMachine
    $report.user_path_unchanged=$userSame; $report.machine_path_unchanged=$machineSame
    if (-not $userSame -or -not $machineSame) { $report.result='FAIL' }
    $report.fixtures=$fixtures
    if ($Evidence) {
        $evidencePath=[IO.Path]::GetFullPath($Evidence)
        $null=[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($evidencePath))
        [IO.File]::WriteAllText($evidencePath,($report|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
    }
    # Las muestras y reservas se conservan para revisión, nunca hay rm recursivo.
    Write-Output "$($report.result): $($checks.Count) comprobaciones; User/Machine PATH intactos=$($userSame -and $machineSame); muestras=$temporary"
}
