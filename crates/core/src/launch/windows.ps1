param([Parameter(Mandatory = $true)][string]$RequestPath, [switch]$InPackage)

$ErrorActionPreference = 'Stop'
$resultPath = Join-Path (Split-Path -Parent $RequestPath) 'result.json'

function Complete-Launch($value) {
    $temporary = $resultPath + '.tmp'
    [IO.File]::WriteAllText($temporary, ($value | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporary, $resultPath)
}

try {
    $request = Get-Content -LiteralPath $RequestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (!$InPackage) {
        # Activation inherits the broker environment, not this process's environment.
        # The in-package bootstrap explicitly restores the requested child environment.
        Invoke-CommandInDesktopPackage -PackageFamilyName $request.family -AppId App `
            -Command (Join-Path $PSHOME 'powershell.exe') -Args $request.bootstrap_arguments -ErrorAction Stop
    } else {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $request.executable
        $info.Arguments = $request.arguments
        $info.UseShellExecute = $false
        if ($request.directory) { $info.WorkingDirectory = $request.directory }
        $info.EnvironmentVariables.Clear()
        foreach ($property in $request.environment.PSObject.Properties) {
            $info.EnvironmentVariables[$property.Name] = [string]$property.Value
        }
        $process = [Diagnostics.Process]::Start($info)
        Complete-Launch @{ pid = $process.Id }
        $process.Dispose()
    }
} catch {
    Complete-Launch @{ error = $_.Exception.ToString() }
    exit 1
}
