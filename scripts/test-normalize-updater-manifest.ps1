$ErrorActionPreference = "Stop"

$normalizer = Join-Path $PSScriptRoot 'normalize-updater-manifest.ps1'
$tempDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "sessiondock-updater-manifest-test-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $tempDirectory | Out-Null

try {
  $manifestPath = Join-Path $tempDirectory 'latest.json'
  @{
    version = '2.0.3'
    notes = 'test'
    pub_date = '2026-11-19T00:00:00Z'
    platforms = @{
      'windows-x86_64' = @{
        signature = 'signed-value'
        url = 'https://api.github.com/repos/nshitritIntel/SessionDock/releases/assets/1'
      }
      'windows-x86_64-nsis' = @{
        signature = 'signed-value'
        url = 'https://api.github.com/repos/nshitritIntel/SessionDock/releases/assets/1'
      }
    }
  } | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath -Encoding utf8

  & $normalizer `
    -ManifestPath $manifestPath `
    -Repository 'nshitritIntel/SessionDock' `
    -Tag 'v2.0.3' `
    -InstallerName 'SessionDock_2.0.3_x64-setup.exe'

  $normalized = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  $expectedUrl = 'https://github.com/nshitritIntel/SessionDock/releases/download/v2.0.3/SessionDock_2.0.3_x64-setup.exe'
  foreach ($platformName in @('windows-x86_64', 'windows-x86_64-nsis')) {
    $platform = $normalized.platforms.PSObject.Properties[$platformName].Value
    if ($platform.url -ne $expectedUrl) {
      throw "Platform $platformName retained a non-public updater URL."
    }
    if ($platform.signature -ne 'signed-value') {
      throw "Platform $platformName signature changed during normalization."
    }
  }

  $normalized.platforms.'windows-x86_64'.signature = ''
  $normalized | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath -Encoding utf8
  $rejectedUnsignedManifest = $false
  try {
    & $normalizer `
      -ManifestPath $manifestPath `
      -Repository 'nshitritIntel/SessionDock' `
      -Tag 'v2.0.3' `
      -InstallerName 'SessionDock_2.0.3_x64-setup.exe'
  }
  catch {
    $rejectedUnsignedManifest = $_.Exception.Message -match 'has no updater signature'
  }
  if (-not $rejectedUnsignedManifest) {
    throw "Normalizer accepted a manifest without a signature."
  }

  Write-Host 'Updater manifest normalization tests passed.'
}
finally {
  Remove-Item -LiteralPath $tempDirectory -Recurse -Force -ErrorAction SilentlyContinue
}