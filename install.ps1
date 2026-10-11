# Verified release acquisition; Universal Agent Plugins owns client configuration.
$ErrorActionPreference = 'Stop'
$Version = '0.3.0'
$Repository = '777genius/agent-wait-on'
$UapArguments = @($args)
if ($UapArguments -contains '--help' -or $UapArguments -contains '-h') {
    Write-Output 'Usage: .\install.ps1 [agentplugins add options]'
    Write-Output 'Example: .\install.ps1 --target codex,claude'
    Write-Output 'Omit --target for Universal Agent Plugins client discovery.'
    exit 0
}
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne 'X64') {
    throw 'install.ps1 supports Windows amd64. On macOS/Linux, use install.sh.'
}
$AgentPlugins = Get-Command agentplugins -ErrorAction SilentlyContinue
$Npx = Get-Command npx -ErrorAction SilentlyContinue
if (-not $AgentPlugins -and -not $Npx) { throw 'Install Universal Agent Plugins (agentplugins), or Node.js with npx, then retry.' }
function Invoke-AgentPlugins([string[]] $Arguments) {
    if ($AgentPlugins) { & $AgentPlugins.Source @Arguments }
    else { & $Npx.Source --yes universal-agent-plugins@0.1.80 @Arguments }
    if ($LASTEXITCODE -ne 0) { throw "Universal Agent Plugins failed (exit $LASTEXITCODE)." }
}
if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is required for the persistent release source cache.' }
$Asset = "agent-wait-on-v$Version-windows-amd64.zip"
$Cache = Join-Path $env:LOCALAPPDATA "agent-wait-on/releases/v$Version/windows-amd64"
$Source = Join-Path $Cache 'package'
$Url = "https://github.com/$Repository/releases/download/v$Version"
New-Item -ItemType Directory -Path $Cache -Force | Out-Null
$Work = Join-Path $Cache ('.install.' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Work | Out-Null
try {
    $Checksums = Join-Path $Cache 'SHA256SUMS'
    if (-not (Test-Path -LiteralPath $Checksums -PathType Leaf)) {
        $Downloaded = Join-Path $Work 'SHA256SUMS'
        Invoke-WebRequest -Uri "$Url/SHA256SUMS" -OutFile $Downloaded
        Move-Item -LiteralPath $Downloaded -Destination $Checksums
    }
    $MatchesForAsset = @(Get-Content -LiteralPath $Checksums | Where-Object {
        $_ -match ('^([A-Fa-f0-9]{64})\s+\*?' + [Regex]::Escape($Asset) + '$')
    })
    if ($MatchesForAsset.Count -ne 1) { throw "Release checksums must contain exactly one entry for $Asset." }
    $Expected = ($MatchesForAsset[0] -split '\s+')[0].ToLowerInvariant()
    $Archive = Join-Path $Cache $Asset
    if (-not (Test-Path -LiteralPath $Archive -PathType Leaf)) {
        $Downloaded = Join-Path $Work $Asset
        Invoke-WebRequest -Uri "$Url/$Asset" -OutFile $Downloaded
        if ((Get-FileHash -LiteralPath $Downloaded -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected) {
            throw "SHA256 verification failed for $Asset."
        }
        Move-Item -LiteralPath $Downloaded -Destination $Archive
    }
    if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected) {
        throw "Cached archive SHA256 verification failed: $Archive"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        foreach ($Entry in $Zip.Entries) {
            if ($Entry.FullName -match '^[/\\]|(^|[/\\])\.\.($|[/\\])|^[A-Za-z]:') {
                throw 'Archive contains unsafe paths.'
            }
        }
    } finally { $Zip.Dispose() }
    $Staged = Join-Path $Work 'package'
    Expand-Archive -LiteralPath $Archive -DestinationPath $Staged
    foreach ($Required in @('plugin.json', 'mcp.json', 'bin/agent-wait-on.exe', 'bin/schema/agent-exec.schema.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $Staged $Required) -PathType Leaf)) {
            throw "Release archive is missing required package file: $Required"
        }
    }
    function Get-PackageInventory([string] $Root) {
        @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File | ForEach-Object {
            $_.FullName.Substring($Root.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        } | Sort-Object)
    }
    if (Test-Path -LiteralPath $Source) {
        $Fresh = Get-PackageInventory $Staged
        $Existing = Get-PackageInventory $Source
        if (@(Compare-Object $Fresh $Existing).Count -ne 0) {
            throw "Cached package differs from the verified release: $Source. Preserve it for diagnosis and move it aside before retrying."
        }
    } else { Move-Item -LiteralPath $Staged -Destination $Source }
    Write-Output "Verified agent-wait-on $Version (windows-amd64). Package source: $Source"
    Invoke-AgentPlugins -Arguments @('validate', $Source)
    Invoke-AgentPlugins -Arguments (@('add', $Source) + $UapArguments)
} finally {
    Remove-Item -LiteralPath $Work -Recurse -Force
}
