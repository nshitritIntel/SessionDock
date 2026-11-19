param(
  [Parameter(Mandatory = $true)]
  [string]$ManifestPath,

  [Parameter(Mandatory = $true)]
  [string]$Repository,

  [Parameter(Mandatory = $true)]
  [string]$Tag,

  [Parameter(Mandatory = $true)]
  [string]$InstallerName
)

$ErrorActionPreference = "Stop"

if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
  throw "Repository must use the owner/name format."
}
if ($Tag -notmatch '^v\d+\.\d+\.\d+$') {
  throw "Tag must be a semantic version prefixed with v."
}

$version = $Tag.Substring(1)
$expectedInstallerName = "SessionDock_${version}_x64-setup.exe"
if ($InstallerName -ne $expectedInstallerName) {
  throw "Expected installer $expectedInstallerName, found $InstallerName."
}
if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
  throw "Updater manifest not found: $ManifestPath"
}

$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ($manifest.version -ne $version) {
  throw "Manifest version $($manifest.version) does not match tag $Tag."
}

$platformNames = @('windows-x86_64', 'windows-x86_64-nsis')
$downloadUrl = "https://github.com/$Repository/releases/download/$Tag/$InstallerName"
foreach ($platformName in $platformNames) {
  $platform = $manifest.platforms.PSObject.Properties[$platformName].Value
  if ($null -eq $platform) {
    throw "Manifest is missing platform $platformName."
  }
  if ([string]::IsNullOrWhiteSpace([string]$platform.signature)) {
    throw "Manifest platform $platformName has no updater signature."
  }
  $platform.url = $downloadUrl
}

$json = $manifest | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText(
  (Resolve-Path -LiteralPath $ManifestPath),
  $json + [Environment]::NewLine,
  [System.Text.UTF8Encoding]::new($false)
)

Write-Host "Normalized updater URL: $downloadUrl"