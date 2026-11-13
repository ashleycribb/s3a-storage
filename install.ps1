# S3A One-Line Installer for Windows PowerShell
$ErrorActionPreference = 'Stop'

$Repo = "ashleycribb/s3a-storage"
$InstallDir = Join-Path $env:USERPROFILE ".s3a\bin"

Write-Host "=================================================" -ForegroundColor Cyan
Write-Host "   Stratified Simplicial Storage Architecture   " -ForegroundColor Cyan
Write-Host "                 S3A Installer                  " -ForegroundColor Cyan
Write-Host "=================================================" -ForegroundColor Cyan

$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -ne "AMD64" -and $Arch -ne "ARM64") {
    Write-Error "Unsupported Windows architecture: $Arch (expected AMD64 or ARM64)"
    exit 1
}

$Target = "x86_64-pc-windows-msvc"
$ArchiveName = "s3a-$Target.zip"
$ChecksumName = "$ArchiveName.sha256"

# Query latest release tag
$LatestTag = $null
try {
    $ReleaseInfo = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "S3A-Installer" }
    $LatestTag = $ReleaseInfo.tag_name
} catch {
    $LatestTag = "v0.1.0"
}

$BaseUrl = "https://github.com/$Repo/releases/download/$LatestTag"
$DownloadUrl = "$BaseUrl/$ArchiveName"
$ChecksumUrl = "$BaseUrl/$ChecksumName"

$TempDir = Join-Path $env:TEMP ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TempDir $ArchiveName
    Write-Host "Downloading S3A ($LatestTag for $Target)..." -ForegroundColor Green
    try {
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
    } catch {
        Write-Warning "Prebuilt binary release asset not found at $DownloadUrl."
        if (Get-Command cargo -ErrorAction SilentlyContinue) {
            Write-Host "Compiling from source via 'cargo install s3a-cli'..." -ForegroundColor Yellow
            cargo install s3a-cli
            exit 0
        } else {
            Write-Error "Failed to download binary and Cargo is not installed. Install Rust or download release manually."
            exit 1
        }
    }

    # Verify SHA-256 checksum if available
    try {
        $ExpectedChecksumRaw = (Invoke-WebRequest -Uri $ChecksumUrl -UseBasicParsing).Content.Trim()
        $ExpectedHash = ($ExpectedChecksumRaw -split '\s+')[0].ToLower()
        $ActualHash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()

        if ($ExpectedHash -and ($ActualHash -ne $ExpectedHash)) {
            Write-Error "Checksum verification failed! Expected: $ExpectedHash, Got: $ActualHash"
            exit 1
        }
        Write-Host "Checksum verification passed." -ForegroundColor Green
    } catch {
        # Checksum file optional if release newly created
    }

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    Write-Host "Extracting binaries to $InstallDir..." -ForegroundColor Green
    Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force
    
    Get-ChildItem -Path $TempDir -Recurse -Filter "s3a*.exe" | ForEach-Object {
        Copy-Item -Path $_.FullName -Destination $InstallDir -Force
    }

    # Add to User PATH if not already present
    $UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
    if ($UserPath -notlike "*$InstallDir*") {
        Write-Host "Adding $InstallDir to User PATH..." -ForegroundColor Cyan
        [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", [EnvironmentVariableTarget]::User)
        $env:Path += ";$InstallDir"
    }

    Write-Host ""
    Write-Host "=================================================" -ForegroundColor Green
    Write-Host "S3A installed successfully!" -ForegroundColor Green
    $ExePath = Join-Path $InstallDir "s3a.exe"
    if (Test-Path $ExePath) {
        & $ExePath --version
    }
    Write-Host ""
    Write-Host "Next steps:" -ForegroundColor Cyan
    Write-Host "  1. Initialize your first vault:"
    Write-Host "     s3a init"
    Write-Host "  2. Run the interactive tour:"
    Write-Host "     s3a tour"
    Write-Host "  3. See available commands:"
    Write-Host "     s3a help"
    Write-Host "=================================================" -ForegroundColor Green
} finally {
    Remove-Item -Path $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
