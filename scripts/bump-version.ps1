# Bump SessionDock version across all configuration files
# Usage: .\scripts\bump-version.ps1 -Version "0.4.0"

param(
    [Parameter(Mandatory=$true)]
    [string]$Version
)

$ErrorActionPreference = "Stop"

# Validate semver format
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Host "ERROR: Version must be in semver format (e.g. 0.4.0)" -ForegroundColor Red
    exit 1
}

Write-Host "Bumping SessionDock to v$Version" -ForegroundColor Cyan

# Keep package.json and package-lock.json synchronized.
npm version $Version --no-git-tag-version --allow-same-version
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Use structured JSON serialization and atomically replace files that an editor may memory-map.
$tauriPath = "src-tauri\tauri.conf.json"
$tauriTempPath = "$PWD\$tauriPath.sessiondock.tmp"
try {
    $env:SESSIONDOCK_VERSION = $Version
    node -e "const fs=require('fs');const source='src-tauri/tauri.conf.json';const target=source+'.sessiondock.tmp';const config=JSON.parse(fs.readFileSync(source,'utf8'));config.version=process.env.SESSIONDOCK_VERSION;fs.writeFileSync(target,JSON.stringify(config,null,2)+'\n')"
    if ($LASTEXITCODE -ne 0) { throw "Unable to serialize tauri.conf.json" }
    Move-Item -LiteralPath $tauriTempPath -Destination "$PWD\$tauriPath" -Force
}
finally {
    Remove-Item Env:SESSIONDOCK_VERSION -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $tauriTempPath -Force -ErrorAction SilentlyContinue
}
Write-Host "  tauri.conf.json: $Version"

# Cargo.toml package version (dependency versions may appear inline)
$cargoPath = "src-tauri\Cargo.toml"
$cargo = Get-Content $cargoPath -Raw
$cargo = $cargo -replace '(?m)^version = "\d+\.\d+\.\d+"(?=\r?$)', "version = `"$Version`""
[System.IO.File]::WriteAllText("$PWD\$cargoPath", $cargo)
Write-Host "  Cargo.toml: $Version"

# Update only the SessionDock package entry in Cargo.lock.
$cargoLockPath = "src-tauri\Cargo.lock"
$cargoLock = Get-Content $cargoLockPath -Raw
$cargoLock = $cargoLock -replace '(?s)(name = "sessiondock"\r?\nversion = ")\d+\.\d+\.\d+("\r?\n)', "`${1}$Version`${2}"
[System.IO.File]::WriteAllText("$PWD\$cargoLockPath", $cargoLock)
Write-Host "  Cargo.lock: $Version"

# Keep documentation examples on the current release filename using explicit UTF-8 I/O.
node -e "const fs=require('fs');const p='README.md';let s=fs.readFileSync(p,'utf8');s=s.replace(/version-\d+\.\d+\.\d+-blue/g,'version-$Version-blue').replace(/SessionDock_\d+\.\d+\.\d+_x64-setup\.exe/g,'SessionDock_${Version}_x64-setup.exe');fs.writeFileSync(p,s,'utf8')"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "  README.md: $Version"

# Validate
Write-Host ""
& "$PSScriptRoot\validate-version.ps1"
