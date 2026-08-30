# Install the latest tinyanalyzer release on 64-bit Windows.
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$Repository = "tinyhumansai/tinyanalyzer"
$Binary = "tinyanalyzer.exe"

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "tinyanalyzer installer: 64-bit Windows is required"
}

$Version = $env:TINYANALYZER_VERSION
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Release = Invoke-RestMethod \
        -Uri "https://api.github.com/repos/$Repository/releases/latest" \
        -Headers @{ "User-Agent" = "tinyanalyzer-installer" }
    $Version = $Release.tag_name
}
if (-not $Version.StartsWith("v")) {
    $Version = "v$Version"
}

$Asset = "tinyanalyzer-$Version-windows-x86_64.zip"
$ReleaseUrl = "https://github.com/$Repository/releases/download/$Version"
$Temporary = Join-Path ([IO.Path]::GetTempPath()) ("tinyanalyzer-install-" + [Guid]::NewGuid())

try {
    New-Item -ItemType Directory -Path $Temporary | Out-Null
    $Archive = Join-Path $Temporary $Asset
    $Checksums = Join-Path $Temporary "SHA256SUMS"

    Write-Host "Downloading $Asset"
    Invoke-WebRequest -Uri "$ReleaseUrl/$Asset" -OutFile $Archive
    Invoke-WebRequest -Uri "$ReleaseUrl/SHA256SUMS" -OutFile $Checksums

    $ChecksumLine = Get-Content $Checksums | Where-Object {
        $_ -match "^[0-9a-fA-F]{64}\s+(\./)?$([Regex]::Escape($Asset))$"
    } | Select-Object -First 1
    if (-not $ChecksumLine) {
        throw "tinyanalyzer installer: $Asset is absent from SHA256SUMS"
    }

    $Expected = ($ChecksumLine -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 -Path $Archive).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "tinyanalyzer installer: checksum verification failed"
    }

    Expand-Archive -Path $Archive -DestinationPath $Temporary
    $SourceBinary = Join-Path $Temporary "tinyanalyzer-$Version-windows-x86_64\$Binary"
    if (-not (Test-Path -LiteralPath $SourceBinary -PathType Leaf)) {
        throw "tinyanalyzer installer: archive does not contain $Binary"
    }

    $InstallDirectory = $env:TINYANALYZER_INSTALL_DIR
    if ([string]::IsNullOrWhiteSpace($InstallDirectory)) {
        $InstallDirectory = Join-Path $env:LOCALAPPDATA "Programs\tinyanalyzer\bin"
    }
    New-Item -ItemType Directory -Force -Path $InstallDirectory | Out-Null
    Copy-Item -Force -LiteralPath $SourceBinary -Destination (Join-Path $InstallDirectory $Binary)

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathEntries = @($UserPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($InstallDirectory -notin $PathEntries) {
        $UpdatedPath = (@($PathEntries) + $InstallDirectory) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $UpdatedPath, "User")
        $env:Path = "$InstallDirectory;$env:Path"
        Write-Host "Added $InstallDirectory to your user PATH; open a new terminal to use it."
    }

    Write-Host "Installed tinyanalyzer $Version to $(Join-Path $InstallDirectory $Binary)"
}
finally {
    if (Test-Path -LiteralPath $Temporary) {
        Remove-Item -LiteralPath $Temporary -Recurse -Force
    }
}
