<#
.SYNOPSIS
  Installs codie on Windows x64 from a published GitHub release.

.DESCRIPTION
  One-command install:

    irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex

  The installer downloads a prebuilt archive, verifies its SHA-256 checksum,
  extracts codie.exe into %LOCALAPPDATA%\Programs\codie, and adds that
  directory to the user PATH.

  Environment variables:
    CODIE_VERSION       Version to install (for example 0.9.0).
                        Defaults to the newest published release.
    CODIE_INSTALL_DIR   Install directory. Defaults to
                        %LOCALAPPDATA%\Programs\codie.
    CODIE_NO_PATH       Set to 1 to skip the user PATH update.

  The script never uses elevation and never edits machine-wide settings.
#>

& {
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if ($PSVersionTable.PSVersion.Major -lt 5 -or
    ($PSVersionTable.PSVersion.Major -eq 5 -and $PSVersionTable.PSVersion.Minor -lt 1)) {
    throw 'codie requires PowerShell 5.1 or newer'
}

# Windows PowerShell 5.1 defaults to older TLS on some systems.
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch {
    # Newer PowerShell already negotiates TLS correctly.
}

$Repository = 'amardeepio/codie'
$Target = 'x86_64-pc-windows-msvc'
$DownloadBase = "https://github.com/$Repository/releases/download"

function Write-Step([string]$Message) {
    Write-Host "codie: $Message"
}

function Get-AssetVersion {
    if ($env:CODIE_VERSION) {
        return $env:CODIE_VERSION.TrimStart('v')
    }

    Write-Step 'resolving the newest release'
    $headers = @{ 'User-Agent' = 'codie-install' }
    try {
        $releases = Invoke-RestMethod -UseBasicParsing -Headers $headers `
            -Uri "https://api.github.com/repos/$Repository/releases?per_page=30"
        $release = $releases | Where-Object { -not $_.draft } | Select-Object -First 1
        if ($release) {
            return $release.tag_name.TrimStart('v')
        }
    } catch {
        Write-Verbose "release lookup failed: $_"
    }

    try {
        $tags = Invoke-RestMethod -UseBasicParsing -Headers $headers `
            -Uri "https://api.github.com/repos/$Repository/tags"
        if ($tags) {
            return $tags[0].name.TrimStart('v')
        }
    } catch {
        Write-Verbose "tag lookup failed: $_"
    }

    throw 'cannot determine the newest codie release; set CODIE_VERSION to pin one'
}

function Get-Checksum([string]$AssetUrl, [string]$AssetName) {
    foreach ($url in @("$AssetUrl.sha256", "$DownloadBase/v$Version/sha256.sum")) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $url
            $text = if ($response.Content -is [byte[]]) {
                [Text.Encoding]::UTF8.GetString($response.Content)
            } else {
                [string]$response.Content
            }
            $line = $text -split "`n" |
                Where-Object { $_ -match [regex]::Escape($AssetName) } |
                Select-Object -First 1
            if (-not $line) {
                $line = $text -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -First 1
            }
            if ($line) {
                $hash = ($line.Trim() -split '\s+')[0]
                if ($hash -match '^[0-9a-fA-F]{64}$') {
                    return $hash
                }
            }
        } catch {
            Write-Verbose "checksum lookup failed for ${url}: $_"
        }
    }
    return $null
}

function Add-ToUserPath([string]$Directory) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @()
    if ($current) {
        $entries = $current -split ';' | Where-Object { $_ }
    }
    if ($entries | Where-Object { $_.TrimEnd('\') -ieq $Directory.TrimEnd('\') }) {
        return $false
    }
    $updated = (@($entries) + $Directory) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
    return $true
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'codie requires 64-bit Windows'
}

$architecture = $env:PROCESSOR_ARCHITECTURE
if ($architecture -and $architecture -notin @('AMD64', 'x86_64')) {
    throw "unsupported architecture: $architecture (only Windows x64 is supported)"
}

$Version = Get-AssetVersion
$AssetName = "codie-v$Version-$Target.zip"
$AssetUrl = "$DownloadBase/v$Version/$AssetName"

$InstallDir = if ($env:CODIE_INSTALL_DIR) {
    $env:CODIE_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\codie'
}
$NoPath = $env:CODIE_NO_PATH -eq '1'

$TempDir = Join-Path $env:TEMP ("codie-install-" + [Guid]::NewGuid().ToString('N'))
$Archive = Join-Path $TempDir $AssetName
$ExtractDir = Join-Path $TempDir 'extract'

try {
    New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
    Write-Step "downloading $AssetName"
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $AssetUrl -OutFile $Archive
    } catch {
        throw "cannot download $AssetUrl`nNo Windows x64 archive may be published for codie $Version.`n$_"
    }

    $expected = Get-Checksum $AssetUrl $AssetName
    if (-not $expected) {
        throw "cannot find a checksum for $AssetName"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash
    if ($actual -ine $expected) {
        throw "checksum verification failed for $AssetName"
    }
    Write-Step 'checksum verified'

    New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
    Expand-Archive -LiteralPath $Archive -DestinationPath $ExtractDir -Force
    $source = Get-ChildItem -Path $ExtractDir -Filter 'codie.exe' -Recurse |
        Select-Object -First 1
    if (-not $source) {
        throw "no codie.exe found in $AssetName"
    }

    if (Get-Process -Name codie -ErrorAction SilentlyContinue) {
        throw 'codie is running; close it and run the installer again'
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $destination = Join-Path $InstallDir 'codie.exe'
    Copy-Item -LiteralPath $source.FullName -Destination $destination -Force

    $addedPath = $false
    if (-not $NoPath) {
        $addedPath = Add-ToUserPath $InstallDir
    }

    $installed = & $destination --version
    if ($LASTEXITCODE -ne 0) {
        throw "installed codie.exe did not run correctly"
    }
    Write-Step "installed $installed in $InstallDir"

    if ($addedPath) {
        Write-Host ''
        Write-Host "Added $InstallDir to your user PATH."
        Write-Host 'Open a new terminal for the change to take effect.'
    } elseif (-not $NoPath) {
        Write-Host ''
        Write-Host "$InstallDir is already on your user PATH."
    }
} finally {
    if (Test-Path -LiteralPath $TempDir) {
        Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
}
