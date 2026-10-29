Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-PillInstallationSettings {
    $settings = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'installation.defaults.json') -Raw | ConvertFrom-Json -AsHashtable
    $names = @('verification_timeout_ms', 'mutex_timeout_ms', 'notification_timeout_ms', 'max_manifest_bytes')
    if ($settings.Count -ne $names.Count) { throw 'Configuración del instalador inválida.' }
    foreach ($name in $names) {
        if (-not $settings.Contains($name) -or $settings[$name] -isnot [long] -or $settings[$name] -lt 1 -or $settings[$name] -gt 1048576) {
            throw "Límite del instalador inválido: $name"
        }
    }
    return $settings
}

function Get-PillNormalizedPath {
    param([AllowNull()][AllowEmptyString()][string]$Value, [bool]$ExpandVariables=$true)
    if ([string]::IsNullOrWhiteSpace($Value)) { return $null }
    $candidate = $Value.Trim()
    if ($candidate.StartsWith('"') -and $candidate.EndsWith('"') -and $candidate.Length -gt 1) {
        $candidate = $candidate.Substring(1, $candidate.Length - 2)
    }
    if ($ExpandVariables) { $candidate = [Environment]::ExpandEnvironmentVariables($candidate) }
    $candidate = $candidate.Replace('/', '\')
    if (-not [IO.Path]::IsPathFullyQualified($candidate) -or $candidate.Contains('%')) { return $null }
    try {
        $candidate = [IO.Path]::GetFullPath($candidate)
        $volume = [IO.Path]::GetPathRoot($candidate)
        if ($candidate.Length -gt $volume.Length) { $candidate = $candidate.TrimEnd('\') }
        return $candidate.ToUpperInvariant()
    } catch { return $null }
}

function Get-PillPathTokens {
    param([AllowNull()][AllowEmptyString()][string]$Value, [bool]$Exists = $true)
    if (-not $Exists) { return ,([string[]]@()) }
    return ,([string[]]$Value.Split([char]';', [StringSplitOptions]::None))
}

function Get-PillPathAddition {
    param([AllowNull()][AllowEmptyString()][string]$Value, [bool]$Exists, [Parameter(Mandatory)][string]$Entry, [bool]$ExpandVariables=$true)
    $normal = Get-PillNormalizedPath $Entry
    if ($null -eq $normal -or $Entry.Contains(';')) { throw 'La entrada de pill debe ser absoluta y sin punto y coma.' }
    $tokens = Get-PillPathTokens -Value $Value -Exists $Exists
    $matches = @($tokens | Where-Object { (Get-PillNormalizedPath $_ -ExpandVariables $ExpandVariables) -ceq $normal })
    $next = if ($matches.Count) { $Value } elseif ($Exists) { $Value + ';' + $Entry } else { $Entry }
    [pscustomobject]@{ Value = $next; Exists = $true; Added = $matches.Count -eq 0; Matches = $matches.Count; Entry = $Entry }
}

function Get-PillPathRemoval {
    param([AllowNull()][AllowEmptyString()][string]$Value, [bool]$Exists,
          [Parameter(Mandatory)][string]$Entry, [bool]$Owned, [AllowNull()][string]$RawEntry,
          [bool]$ExistedBefore = $true, [bool]$ExpandVariables=$true)
    if (-not $Owned) { return [pscustomobject]@{ Value = $Value; Exists = $Exists; Removed = $false } }
    $normal = Get-PillNormalizedPath $Entry
    if ($null -eq $normal -or (Get-PillNormalizedPath $RawEntry) -cne $normal) { throw 'La entrada acreditada no pertenece a esta instalación.' }
    if (-not $Exists) { throw 'La entrada acreditada desapareció del PATH; no se retira otra.' }
    $tokens = Get-PillPathTokens -Value $Value -Exists $Exists
    $indices = @(for ($index = 0; $index -lt $tokens.Length; $index++) {
        if ((Get-PillNormalizedPath $tokens[$index] -ExpandVariables $ExpandVariables) -ceq $normal) { $index }
    })
    if ($indices.Count -ne 1 -or $tokens[$indices[0]] -cne $RawEntry) {
        throw 'La entrada propia cambió o es ambigua; se conserva el PATH sin modificar.'
    }
    $remaining = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $tokens.Length; $index++) {
        if ($index -ne $indices[0]) { $remaining.Add($tokens[$index]) }
    }
    $nextExists = $remaining.Count -gt 0 -or $ExistedBefore
    [pscustomobject]@{ Value = if ($nextExists) { [string]::Join(';', $remaining) } else { $null }; Exists = $nextExists; Removed = $true }
}

function Get-PillTextHash {
    param([AllowNull()][AllowEmptyString()][string]$Value)
    $tagged = if ($null -eq $Value) { 'null:' } else { 'text:' + $Value }
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($tagged))).ToLowerInvariant()
}

function Get-PillPathSnapshot {
    param([ValidateSet('Process', 'User')][string]$Scope)
    if ($Scope -eq 'Process') {
        $value = [Environment]::GetEnvironmentVariable('Path', 'Process')
        return [pscustomobject]@{ Scope = $Scope; Exists = $null -ne $value; Value = $value; Kind = 'String' }
    }
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
    try {
        $exists = $null -ne $key -and @($key.GetValueNames() | Where-Object { $_ -ieq 'Path' }).Count -gt 0
        if (-not $exists) { return [pscustomobject]@{ Scope = $Scope; Exists = $false; Value = $null; Kind = 'ExpandString' } }
        $kind = $key.GetValueKind('Path').ToString()
        if ($kind -notin @('String', 'ExpandString')) { throw 'El PATH de usuario no es String/ExpandString; no se modifica.' }
        $value = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($value -isnot [string]) { throw 'El PATH de usuario no contiene texto.' }
        return [pscustomobject]@{ Scope = $Scope; Exists = $true; Value = $value; Kind = $kind }
    } finally { if ($null -ne $key) { $key.Dispose() } }
}

function Test-PillSameSnapshot {
    param($Left, $Right)
    return $Left.Scope -ceq $Right.Scope -and $Left.Exists -eq $Right.Exists -and $Left.Value -ceq $Right.Value -and $Left.Kind -ceq $Right.Kind
}

function Set-PillPathSnapshot {
    param([Parameter(Mandatory)]$Expected, [Parameter(Mandatory)]$Desired)
    if (-not (Test-PillSameSnapshot (Get-PillPathSnapshot $Expected.Scope) $Expected)) { throw 'Conflicto: el PATH cambió antes de escribir; no se sobreescribe.' }
    if ($Expected.Scope -eq 'Process') {
        [Environment]::SetEnvironmentVariable('Path', $(if ($Desired.Exists) { $Desired.Value } else { $null }), 'Process')
    } else {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment', $true)
        try {
            if (-not (Test-PillSameSnapshot (Get-PillPathSnapshot 'User') $Expected)) { throw 'Conflicto: el PATH cambió al abrir el Registro.' }
            if ($Desired.Exists) {
                $kind = [Microsoft.Win32.RegistryValueKind]$Desired.Kind
                $key.SetValue('Path', $Desired.Value, $kind)
            } else { $key.DeleteValue('Path', $false) }
            $key.Flush()
        } finally { $key.Dispose() }
    }
    $observed = Get-PillPathSnapshot $Expected.Scope
    if (-not (Test-PillSameSnapshot $observed $Desired)) { throw 'Resultado de PATH desconocido: cambió tras escribir; no se restaura encima de cambios ajenos.' }
}

function New-PillDesiredSnapshot {
    param($Original, [AllowNull()][AllowEmptyString()][string]$Value, [bool]$Exists)
    [pscustomobject]@{ Scope = $Original.Scope; Exists = $Exists; Value = if ($Exists) { $Value } else { $null }; Kind = $Original.Kind }
}

function Initialize-PillNative {
    if ('PillInstallation.Native' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Collections.Generic;
using Microsoft.Win32.SafeHandles;
namespace PillInstallation {
 public static class Native {
  [StructLayout(LayoutKind.Sequential)] public struct Info {
   public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Creation;
   public System.Runtime.InteropServices.ComTypes.FILETIME Access;
   public System.Runtime.InteropServices.ComTypes.FILETIME Write;
   public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
  }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern SafeFileHandle CreateFileW(string path,uint access,uint share,IntPtr security,uint creation,uint flags,IntPtr template);
  [StructLayout(LayoutKind.Sequential)] struct UnicodeName { public ushort Length,MaximumLength;public IntPtr Buffer; }
  [StructLayout(LayoutKind.Sequential)] struct Attributes { public uint Length;public IntPtr RootDirectory,ObjectName;public uint Flags;public IntPtr SecurityDescriptor,QualityOfService; }
  [StructLayout(LayoutKind.Sequential)] struct IoStatus { public IntPtr Status;public UIntPtr Information; }
  [DllImport("ntdll.dll")] static extern int NtCreateFile(out SafeFileHandle handle,uint access,ref Attributes attributes,out IoStatus status,IntPtr allocation,uint fileAttributes,uint share,uint disposition,uint options,IntPtr ea,uint eaLength);
  [DllImport("ntdll.dll")] static extern uint RtlNtStatusToDosError(int status);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool GetFileInformationByHandle(SafeFileHandle handle,out Info info);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool SetFileInformationByHandle(SafeFileHandle handle,int kind,IntPtr data,uint size);
  [DllImport("advapi32.dll")] static extern uint GetSecurityInfo(SafeFileHandle handle,int kind,uint flags,out IntPtr owner,out IntPtr group,out IntPtr dacl,out IntPtr sacl,out IntPtr descriptor);
  [DllImport("advapi32.dll")] static extern uint SetSecurityInfo(SafeFileHandle handle,int kind,uint flags,IntPtr owner,IntPtr group,IntPtr dacl,IntPtr sacl);
  [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetSecurityDescriptorControl(IntPtr descriptor,out ushort control,out uint revision);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,IntPtr first,string second,uint flags,uint timeout,out IntPtr result);
  static SafeFileHandle Open(string path,uint access,uint share,uint creation,uint flags) {
   var handle=CreateFileW(path,access,share,IntPtr.Zero,creation,flags|0x00200000,IntPtr.Zero);
   if(handle.IsInvalid) { int error=Marshal.GetLastWin32Error();handle.Dispose();throw new Win32Exception(error,path); }
   return handle;
  }
  public static Info Inspect(SafeFileHandle handle) {
   Info info;if(!GetFileInformationByHandle(handle,out info)) throw new Win32Exception(Marshal.GetLastWin32Error());
   if((info.Attributes&0x400)!=0) throw new IOException("Enlace/reparse no admitido.");
   return info;
  }
  public static List<SafeFileHandle> PinDirectories(string path) {
   var paths=new List<string>(); var current=new DirectoryInfo(Path.GetFullPath(path));
   while(current!=null) { paths.Add(current.FullName);current=current.Parent; }
   paths.Reverse();var handles=new List<SafeFileHandle>();
   try { foreach(var directory in paths) { var handle=Open(directory,0x80000000,3,3,0x02000000);
    handles.Add(handle);if((Inspect(handle).Attributes&0x10)==0) throw new IOException("Un padre no es directorio."); }
    return handles;
   } catch { foreach(var handle in handles) handle.Dispose();throw; }
  }
  public static FileStream OpenRegular(string path,bool create) {return OpenFile(path,create,false);}
  public static FileStream OpenBuildArtifact(string path) {return OpenFile(path,false,true);}
  private static FileStream OpenFile(string path,bool create,bool buildArtifact) {
   var full=Path.GetFullPath(path);var pins=PinDirectories(Path.GetDirectoryName(full));
   IntPtr text=IntPtr.Zero,namePointer=IntPtr.Zero;SafeFileHandle handle=null;
   try { string leaf=Path.GetFileName(full);int byteLength=Encoding.Unicode.GetByteCount(leaf);
    if(byteLength>65532) throw new IOException("Nombre de archivo demasiado largo.");
    text=Marshal.StringToHGlobalUni(leaf);var name=new UnicodeName{Length=(ushort)byteLength,MaximumLength=(ushort)(byteLength+2),Buffer=text};
    namePointer=Marshal.AllocHGlobal(Marshal.SizeOf<UnicodeName>());Marshal.StructureToPtr(name,namePointer,false);
    var parent=pins[pins.Count-1];Inspect(parent);
    var attributes=new Attributes{Length=(uint)Marshal.SizeOf<Attributes>(),RootDirectory=parent.DangerousGetHandle(),ObjectName=namePointer,Flags=0x40};
    IoStatus result;int status=NtCreateFile(out handle,(buildArtifact?0x80100000U:0x80110000U)|(create?0x400C0000U:0U),ref attributes,out result,IntPtr.Zero,0,1,create?2U:1U,0x00200060,IntPtr.Zero,0);
    if(status<0) throw new Win32Exception((int)RtlNtStatusToDosError(status),path);
    Inspect(parent);var info=Inspect(handle);
    if((info.Attributes&0x10)!=0 || info.Links==0 || (!buildArtifact && info.Links!=1)) throw new IOException("Archivo no regular o hardlink.");
    var stream=new FileStream(handle,create?FileAccess.ReadWrite:FileAccess.Read);handle=null;return stream;
   } finally {if(handle!=null) handle.Dispose();if(text!=IntPtr.Zero) Marshal.FreeHGlobal(text);if(namePointer!=IntPtr.Zero) Marshal.FreeHGlobal(namePointer);foreach(var pin in pins) pin.Dispose();}
  }
  public static string Identity(FileStream file) { var info=Inspect(file.SafeFileHandle);return info.Volume+":"+info.IndexHigh+":"+info.IndexLow; }
  public static void CopySecurity(FileStream source,FileStream destination) {
   IntPtr owner,group,dacl,sacl,descriptor;uint result=GetSecurityInfo(source.SafeFileHandle,1,7,out owner,out group,out dacl,out sacl,out descriptor);
   if(result!=0) throw new Win32Exception((int)result);
   try {ushort control;uint revision;if(!GetSecurityDescriptorControl(descriptor,out control,out revision)) throw new Win32Exception(Marshal.GetLastWin32Error());
    result=SetSecurityInfo(destination.SafeFileHandle,1,7U|((control&0x1000)!=0?0x80000000U:0x20000000U),owner,group,dacl,IntPtr.Zero);
    if(result!=0) throw new Win32Exception((int)result);
   } finally {LocalFree(descriptor);}
  }
  public static void RenameNoReplace(FileStream file,string destination) {
   byte[] name=Encoding.Unicode.GetBytes(Path.GetFullPath(destination));int offset=IntPtr.Size==8?20:12;
   IntPtr buffer=Marshal.AllocHGlobal(offset+name.Length+2);
   try { for(int i=0;i<offset+name.Length+2;i++) Marshal.WriteByte(buffer,i,0);
    Marshal.WriteInt32(buffer,IntPtr.Size==8?16:8,name.Length);Marshal.Copy(name,0,IntPtr.Add(buffer,offset),name.Length);
    if(!SetFileInformationByHandle(file.SafeFileHandle,3,buffer,(uint)(offset+name.Length+2))) {
     int error=Marshal.GetLastWin32Error();throw new Win32Exception(error,new Win32Exception(error).Message+" ["+error+"] "+destination);
    }
   } finally {Marshal.FreeHGlobal(buffer);}
  }
  public static void DeleteOwned(FileStream file) {
   IntPtr data=Marshal.AllocHGlobal(1);try {Marshal.WriteByte(data,1);
    if(!SetFileInformationByHandle(file.SafeFileHandle,4,data,1)) throw new Win32Exception(Marshal.GetLastWin32Error());
   } finally {Marshal.FreeHGlobal(data);}
  }
  public static bool Notify(uint timeout) {IntPtr result;return SendMessageTimeoutW(new IntPtr(0xffff),0x001A,IntPtr.Zero,"Environment",2,timeout,out result)!=IntPtr.Zero;}
 }
}
'@
}

function Get-PillStreamHash {
    param([Parameter(Mandatory)][IO.FileStream]$Stream)
    $Stream.Position = 0
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToHexString($algorithm.ComputeHash($Stream)).ToLowerInvariant() }
    finally { $algorithm.Dispose(); $Stream.Position = 0 }
}

function Get-PillContext {
    $root = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
    if ($root.Contains(';')) { throw 'La raíz de pill contiene punto y coma y no se puede registrar en PATH.' }
    $bin = Join-Path $root 'bin'
    [pscustomobject]@{ Root=$root; Bin=$bin; Binary=(Join-Path $bin 'pill.exe'); Manifest=(Join-Path $bin 'installation.json'); Source=(Join-Path $root 'target/release/pill.exe'); Settings=(Get-PillInstallationSettings) }
}

function Get-PillPackageVersion {
    param($Context)
    $text = Get-Content -LiteralPath (Join-Path $Context.Root 'Cargo.toml') -Raw
    $package = [regex]::Match($text, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
    $match = [regex]::Match($package, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if (-not $match.Success) { throw 'No se encontró package.version en Cargo.toml.' }
    return $match.Groups[1].Value
}

function Assert-PillCommand {
    param($Context, [switch]$MustResolve)
    $commands = @(Get-Command pill -All -ErrorAction SilentlyContinue)
    foreach ($command in $commands) {
        if ($command.CommandType -ne 'Application' -or (Get-PillNormalizedPath $command.Source) -cne (Get-PillNormalizedPath $Context.Binary)) {
            throw "El comando pill está ocupado por $($command.CommandType): $($command.Source). No se modifica esa instalación."
        }
    }
    if ($MustResolve -and $commands.Count -eq 0) { throw 'El PATH no permite resolver pill tras registrar; no se anuncia éxito.' }
}

function Read-PillManifest {
    param($Context)
    if (-not [IO.File]::Exists($Context.Manifest)) { return $null }
    $file = [PillInstallation.Native]::OpenRegular($Context.Manifest, $false)
    try {
        if ($file.Length -gt $Context.Settings.max_manifest_bytes) { throw 'Manifiesto de instalación demasiado grande.' }
        $reader = [IO.StreamReader]::new($file, [Text.UTF8Encoding]::new($false,$true), $true, 1024, $true)
        try { $raw = $reader.ReadToEnd() } finally { $reader.Dispose() }
        $data = $raw | ConvertFrom-Json -AsHashtable
        $hash = Get-PillStreamHash $file
    } finally { $file.Dispose() }
    if ($data.schema -ne 1 -or (Get-PillNormalizedPath $data.project_root) -cne (Get-PillNormalizedPath $Context.Root) -or
        (Get-PillNormalizedPath $data.binary) -cne (Get-PillNormalizedPath $Context.Binary) -or $data.binary_sha256 -cnotmatch '^[a-f0-9]{64}$' -or
        $data.binary_identity -cnotmatch '^\d+:\d+:\d+$' -or
        $data.state -notin @('prepared','complete','unregistered')) { throw 'Manifiesto corrupto o perteneciente a otra instalación.' }
    foreach ($scope in @('User','Process')) {
        $entry = $data.scopes[$scope]
        if ($null -ne $entry) {
            if ($entry.added -isnot [bool] -or $entry.before_exists -isnot [bool]) { throw 'Propiedad de PATH inválida en el manifiesto.' }
            if ($entry.added -and ((Get-PillNormalizedPath $entry.raw_entry) -cne (Get-PillNormalizedPath $Context.Bin) -or
                $entry.before_sha256 -cnotmatch '^[a-f0-9]{64}$' -or $entry.after_sha256 -cnotmatch '^[a-f0-9]{64}$')) { throw 'Entrada de PATH ajena o corrupta en el manifiesto.' }
            if ($scope -eq 'Process' -and ($entry.process_id -isnot [long] -or $entry.creation_ticks -isnot [long])) { throw 'Identidad de proceso inválida.' }
        }
    }
    return [pscustomobject]@{ Data=$data; Hash=$hash }
}

function Publish-PillStage {
    param($Context, [Parameter(Mandatory)][IO.FileStream]$Stage, [Parameter(Mandatory)][string]$Destination,
          [AllowNull()][string]$ExpectedHash, [AllowNull()][string]$ExpectedIdentity)
    if ([IO.Path]::GetDirectoryName($Destination) -cne $Context.Bin) { throw 'Destino de publicación fuera de bin.' }
    $old = $null; $backup = $null
    try {
        if ([IO.File]::Exists($Destination)) {
            $old = [PillInstallation.Native]::OpenRegular($Destination, $false)
            if (-not $ExpectedHash -or (Get-PillStreamHash $old) -cne $ExpectedHash) { throw 'El destino cambió antes de publicar; no se sobreescribe.' }
            if ($ExpectedIdentity -and [PillInstallation.Native]::Identity($old) -cne $ExpectedIdentity) {throw 'La identidad del destino cambió antes de publicar.'}
            $backup = Join-Path $Context.Bin (([IO.Path]::GetFileName($Destination)) + '.previous.' + [guid]::NewGuid().ToString('N'))
            [PillInstallation.Native]::RenameNoReplace($old, $backup)
        } elseif ($ExpectedHash) { throw 'El destino esperado desapareció antes de publicar.' }
        try { [PillInstallation.Native]::RenameNoReplace($Stage, $Destination) }
        catch {
            if ($null -ne $old -and -not [IO.File]::Exists($Destination)) {
                try { [PillInstallation.Native]::RenameNoReplace($old,$Destination) } catch { }
            }
            throw "Publicación incompleta de $Destination; reserva $backup conservada. $($_.Exception.Message)"
        }
        return $backup
    } finally { if ($null -ne $old) { $old.Dispose() } }
}

function Save-PillManifest {
    param($Context, [Parameter(Mandatory)]$Data, [AllowNull()][string]$ExpectedHash)
    $bytes = [Text.UTF8Encoding]::new($false,$true).GetBytes(($Data | ConvertTo-Json -Depth 12))
    if ($bytes.Length -gt $Context.Settings.max_manifest_bytes) { throw 'El manifiesto excede la cota configurada.' }
    $temporary = Join-Path $Context.Bin ('installation.' + [guid]::NewGuid().ToString('N') + '.new')
    $file = [PillInstallation.Native]::OpenRegular($temporary,$true)
    try {
        $file.Write($bytes); $file.Flush($true)
        $hash = Get-PillStreamHash $file
        $null = Publish-PillStage $Context $file $Context.Manifest $ExpectedHash
        return $hash
    } finally { $file.Dispose() }
}

function Test-PillBinaryVersion {
    param([string]$Path,[string]$Version,[int]$TimeoutMs)
    $info = [Diagnostics.ProcessStartInfo]::new($Path)
    $info.UseShellExecute = $false; $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
    $info.ArgumentList.Add('--version')
    $process = [Diagnostics.Process]::new(); $process.StartInfo=$info
    try {
        if (-not $process.Start()) { throw 'No se inició la verificación del binario.' }
        $output = $process.StandardOutput.ReadToEndAsync(); $errors=$process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMs)) { $process.Kill($true); $process.WaitForExit(); throw 'Timeout verificando el binario propio.' }
        if ($process.ExitCode -ne 0 -or $output.GetAwaiter().GetResult().Trim() -cne "pill $Version" -or $errors.GetAwaiter().GetResult().Trim()) {
            throw 'El binario no acredita la versión pill del proyecto.'
        }
    } finally { $process.Dispose() }
}

function Get-PillProcessIdentity {
    $process = [Diagnostics.Process]::GetCurrentProcess()
    try { return [pscustomobject]@{ Id=[long]$process.Id; Creation=[long]$process.StartTime.ToUniversalTime().Ticks } }
    finally { $process.Dispose() }
}

function Get-PillOwnership {
    param($Before,$Plan,$Previous,[string]$Scope,$Identity)
    $retain = $false
    if ($null -ne $Previous -and $Previous.added) {
        $tokens = Get-PillPathTokens $Before.Value $Before.Exists
        $retain = @($tokens | Where-Object { $_ -ceq $Previous.raw_entry }).Count -eq 1 -and $Plan.Matches -eq 1
        if ($Scope -eq 'Process') { $retain = $retain -and $Previous.process_id -eq $Identity.Id -and $Previous.creation_ticks -eq $Identity.Creation }
    }
    if ($retain) { return $Previous }
    $record = [ordered]@{ added=[bool]$Plan.Added; raw_entry=$(if ($Plan.Added) { $Plan.Entry } else { $null }); before_exists=[bool]$Before.Exists;
        before_sha256=(Get-PillTextHash $Before.Value); after_sha256=(Get-PillTextHash $Plan.Value) }
    if ($Scope -eq 'Process') { $record.process_id=$Identity.Id; $record.creation_ticks=$Identity.Creation }
    return $record
}

function Enter-PillInstallMutex {
    param([int]$TimeoutMs)
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {$sid = $identity.User.Value} finally {$identity.Dispose()}
    $mutex = [Threading.Mutex]::new($false, ('Local\PILLCLI.Install.' + $sid))
    $owned=$false
    try { try { $owned=$mutex.WaitOne($TimeoutMs) } catch [Threading.AbandonedMutexException] { $owned=$true }
        if (-not $owned) { throw 'Otra instalación/desinstalación de pill sigue activa.' }
        return $mutex
    } catch { $mutex.Dispose(); throw }
}

function Undo-PillPathChange {
    param($Before,$After)
    if (Test-PillSameSnapshot (Get-PillPathSnapshot $Before.Scope) $After) { Set-PillPathSnapshot $After $Before; return $true }
    return $false
}

Export-ModuleMember -Function Get-PillInstallationSettings,Get-PillNormalizedPath,Get-PillPathTokens,Get-PillPathAddition,Get-PillPathRemoval,Get-PillPathSnapshot,Test-PillSameSnapshot,Set-PillPathSnapshot,New-PillDesiredSnapshot,Initialize-PillNative,Get-PillStreamHash,Get-PillContext,Get-PillPackageVersion,Assert-PillCommand,Read-PillManifest,Publish-PillStage,Save-PillManifest,Test-PillBinaryVersion,Get-PillProcessIdentity,Get-PillOwnership,Enter-PillInstallMutex,Undo-PillPathChange
