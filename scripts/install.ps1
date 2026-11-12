param(
    [string]$InstallDir = $env:ACS_INSTALL_DIR,
    [string]$ReleaseBase = "https://github.com/shellus/acs/releases/latest/download",
    [string]$ReleaseDir = $env:ACS_RELEASE_DIR
)
$ErrorActionPreference = "Stop"
if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne "X64" -or [Environment]::OSVersion.Platform -ne "Win32NT") { throw "Only Windows x64 is supported" }
if (-not $InstallDir) { $InstallDir = Join-Path $HOME "bin" }
$asset = "acs-cli-windows-x64.exe"
$staged = $null
$taskTmp = Join-Path ([IO.Path]::GetTempPath()) ("acs-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $taskTmp | Out-Null
try {
    foreach ($name in @($asset, "$asset.sha256")) {
        $destination = Join-Path $taskTmp $name
        if ($ReleaseDir) { Copy-Item -LiteralPath (Join-Path $ReleaseDir $name) -Destination $destination }
        else { Invoke-WebRequest -UseBasicParsing -TimeoutSec 180 "$ReleaseBase/$name" -OutFile $destination }
    }
    $lines = @(Get-Content -LiteralPath (Join-Path $taskTmp "$asset.sha256") | Where-Object { $_ -match "^([a-fA-F0-9]{64})\s+\*?$([regex]::Escape($asset))$" })
    if ($lines.Count -ne 1) { throw "Missing, duplicate or invalid SHA-256" }
    $expected = ($lines[0] -split '\s+')[0].ToLowerInvariant()
    $payload = Join-Path $taskTmp $asset
    if ((Get-FileHash -LiteralPath $payload -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw "SHA-256 mismatch; installation unchanged" }
    $bytes = [IO.File]::ReadAllBytes($payload)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 77 -or $bytes[1] -ne 90) { throw "Not a Windows PE binary" }
    $offset = [BitConverter]::ToUInt32($bytes, 60)
    if ($offset -gt $bytes.Length - 6 -or [BitConverter]::ToUInt32($bytes, $offset) -ne 17744 -or [BitConverter]::ToUInt16($bytes, $offset+4) -ne 34404) { throw "PE platform mismatch" }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $target = Join-Path $InstallDir "acs.exe"
    $staged = Join-Path $InstallDir (".acs-install-" + [guid]::NewGuid())
    Copy-Item -LiteralPath $payload -Destination $staged
    $backup = "$target.old"
    if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup }
    $existed = Test-Path -LiteralPath $target
    if ($existed) { Move-Item -LiteralPath $target -Destination $backup }
    try { Move-Item -LiteralPath $staged -Destination $target }
    catch { if ($existed) { Move-Item -LiteralPath $backup -Destination $target }; throw }
    Write-Host "Installed acs CLI to $target; add this directory to PATH if necessary."
}
finally {
    if ($staged -and (Test-Path -LiteralPath $staged)) { Remove-Item -LiteralPath $staged }
    # This is the exact temporary directory created above, never a caller-supplied path.
    $resolved = [IO.Path]::GetFullPath($taskTmp)
    if ([IO.Path]::GetDirectoryName($resolved) -ne [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/')) { throw "Unexpected temporary path" }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
