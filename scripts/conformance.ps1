# S3A xAPI 1.0.3 ADL Conformance Test Suite Runner
param(
    [string]$Port = "8089",
    [string]$AuthUser = "test",
    [string]$AuthPass = "password",
    [string]$Grep = "",
    [switch]$Bail,
    [string]$Directory = ""
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$ServerExe = Join-Path $ProjectRoot "target\debug\s3a-lrs-server.exe"

if (-not (Test-Path $ServerExe)) {
    Write-Host "[conformance.ps1] Building s3a-lrs-server..." -ForegroundColor Cyan
    & cargo build --bin s3a-lrs-server --manifest-path (Join-Path $ProjectRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Failed to build s3a-lrs-server"
        exit 1
    }
}

$TempArchive = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "s3a_lrs_conformance_$([System.Guid]::NewGuid().ToString('N')).s3a")
$TempAudit = $TempArchive + ".audit.s3a"
$Endpoint = "http://127.0.0.1:$Port/xapi/"

Write-Host "[conformance.ps1] Starting S3A LRS Server on port $Port..." -ForegroundColor Cyan
Write-Host "  Archive: $TempArchive"
Write-Host "  Audit:   $TempAudit"

$ServerProcess = Start-Process -FilePath $ServerExe `
    -ArgumentList "--port $Port -a `"$TempArchive`" --audit-archive `"$TempAudit`" -u $AuthUser -p $AuthPass" `
    -PassThru -NoNewWindow

try {
    # Wait for server to become responsive
    $Ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 200
        try {
            $resp = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/xapi/about" -Method Get -TimeoutSec 1 -ErrorAction SilentlyContinue
            if ($resp -and $resp.version) {
                $Ready = $true
                break
            }
        } catch {
            # Continue waiting
        }
    }

    if (-not $Ready) {
        Write-Error "[conformance.ps1] Server failed to become ready on port $Port within 6 seconds"
        exit 1
    }

    Write-Host "[conformance.ps1] S3A LRS Server is ready at $Endpoint" -ForegroundColor Green

    # Build runner arguments
    $RunnerArgs = @("bin/console_runner.js", "-x", "1.0.3", "-e", $Endpoint, "-a", "-u", $AuthUser, "-p", $AuthPass)
    if ($Grep) {
        $RunnerArgs += @("-g", $Grep)
    }
    if ($Bail) {
        $RunnerArgs += @("-b")
    }
    if ($Directory) {
        $RunnerArgs += @("-d", $Directory)
    }

    Write-Host "[conformance.ps1] Executing ADL LRS Conformance Test Suite..." -ForegroundColor Cyan
    $AdlDir = Join-Path $ProjectRoot "tests\adl-conformance"

    Push-Location $AdlDir
    try {
        $PrevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $Output = & node --no-deprecation $RunnerArgs 2>&1
        $SuiteExitCode = $LASTEXITCODE
        $ErrorActionPreference = $PrevEAP
    } finally {
        Pop-Location
    }

    $OutputString = ($Output | Out-String)
    Write-Host $OutputString

    # Save results to docs/conformance-results.md
    $ResultsPath = Join-Path $ProjectRoot "docs\conformance-results.md"
    $Timestamp = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss UTC")
    
    $Markdown = @"
# ADL xAPI 1.0.3 LRS Conformance Test Results

**Date**: $Timestamp  
**Target Server**: S3A Native LRS Server (`s3a-lrs-server`)  
**Storage Architecture**: Native S3A Archive Format (Zero SQL / DuckDB / external DB)  
**Endpoint**: `$Endpoint`  
**Authentication**: HTTP Basic Auth (SHA256 constant-time comparison)  

## Test Run Summary
- **Exit Code**: $SuiteExitCode
- **Pass / Fail Output**:

````text
$OutputString
````

## Conformance Verification
The test suite validates compliance against the Official ADL xAPI 1.0.3 Test Suite:
- **Statements API**: PUT, POST, GET, HEAD (voiding, attachments, multipart/mixed, canonical/exact/ids formats, query filtering, pagination)
- **State API**: Activities State document persistence, ETag concurrency, If-Match / If-None-Match
- **Activity Profile API**: Profiles storage, ETag concurrency, merging
- **Agent Profile API**: Agent profile documents, Person object retrieval
- **About API**: Version 1.0.3 declaration
"@

    Set-Content -Path $ResultsPath -Value $Markdown -Encoding utf8
    Write-Host "[conformance.ps1] Results written to $ResultsPath" -ForegroundColor Green

    if ($SuiteExitCode -ne 0) {
        Write-Warning "[conformance.ps1] ADL Conformance Suite reported failures (exit code: $SuiteExitCode)"
        exit $SuiteExitCode
    } else {
        Write-Host "[conformance.ps1] All conformance tests passed successfully!" -ForegroundColor Green
    }
} finally {
    Write-Host "[conformance.ps1] Stopping server process $($ServerProcess.Id)..." -ForegroundColor Yellow
    if ($ServerProcess -and -not $ServerProcess.HasExited) {
        Stop-Process -Id $ServerProcess.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 300
    if (Test-Path $TempArchive) {
        Remove-Item -Force $TempArchive -ErrorAction SilentlyContinue
    }
    if (Test-Path $TempAudit) {
        Remove-Item -Force $TempAudit -ErrorAction SilentlyContinue
    }
}
