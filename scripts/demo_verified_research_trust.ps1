# S3A Verified Research Trust End-to-End Demonstration Script
# Demonstrates:
#   1. Seeding a Scholar snapshot with claims, citation fingerprints, and paper provenance
#   2. Sealing the snapshot with Reed-Solomon K+M parity and cryptographic directory
#   3. Anchoring snapshot fingerprint to OpenTimestamps (mock calendar)
#   4. Verifying the sealed archive and cryptographic trust anchor (Match: yes)
#   5. Tampering with one byte and demonstrating verification failure (Integrity check)
#   6. Resolving and printing the end-to-end Claim Evidence Chain (Human-readable and PROV-O JSON-LD)
#   7. Exporting bibliography with companion .proof.json

param(
    [switch]$KeepArtifacts
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$CliExe = Join-Path $ProjectRoot "target\debug\s3a-cli.exe"

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "         S3A VERIFIED RESEARCH TRUST END-TO-END DEMO                            " -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan

# Step 0: Ensure s3a-cli is compiled
if (-not (Test-Path $CliExe)) {
    Write-Host "[Step 0] Building s3a-cli..." -ForegroundColor Yellow
    & cargo build --bin s3a-cli --manifest-path (Join-Path $ProjectRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Failed to build s3a-cli"
        exit 1
    }
}

$DemoDir = Join-Path ([System.IO.Path]::GetTempPath()) "s3a_demo_$([System.Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $DemoDir -Force | Out-Null
Write-Host "Working Directory: $DemoDir`n" -ForegroundColor Gray

$RawSnapshot = Join-Path $DemoDir "scholar_snapshot_raw.s3a"
$SealedSnapshot = Join-Path $DemoDir "scholar_snapshot_sealed.s3a"
$TamperedSnapshot = Join-Path $DemoDir "scholar_snapshot_tampered.s3a"
$BibOutput = Join-Path $DemoDir "bibliography.bib"
$ProofOutput = Join-Path $DemoDir "bibliography.bib.proof.json"

try {
    # Step 1: Seed Scholar research snapshot
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 1: Seed Scholar Research Snapshot with Claims and Citation Fingerprints   " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe scholar demo-seed $RawSnapshot
    if ($LASTEXITCODE -ne 0) { throw "demo-seed failed" }
    Write-Host "Snapshot seeded at $RawSnapshot`n" -ForegroundColor Green

    # Step 2: Seal snapshot with Reed-Solomon parity and cryptographic directory
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 2: Seal Archive with Reed-Solomon Parity and Cryptographic Directory      " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe seal $RawSnapshot $SealedSnapshot
    if ($LASTEXITCODE -ne 0) { throw "seal failed" }
    Write-Host "Snapshot sealed at $SealedSnapshot`n" -ForegroundColor Green

    # Step 3: Anchor snapshot fingerprint to OpenTimestamps (mock calendar)
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 3: Anchor Snapshot Fingerprint via OpenTimestamps                         " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe anchor $SealedSnapshot --mock
    if ($LASTEXITCODE -ne 0) { throw "anchor failed" }
    Write-Host "Snapshot anchored with cryptographic proof in footer ANCHORS section`n" -ForegroundColor Green

    # Step 4: Verify sealed snapshot and trust anchor
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 4: Verify Cryptographic Integrity and Trust Anchor Match                 " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe verify --anchor $SealedSnapshot
    if ($LASTEXITCODE -ne 0) { throw "verify anchor failed" }
    Write-Host "Sealed snapshot verified: 100% hardware CRC32C and Trust Anchor Match: yes`n" -ForegroundColor Green

    # Step 5: Tamper with one byte and demonstrate verification failure
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 5: Tamper Simulation - Mutate 1 Byte in Sealed Archive Payload           " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Copy-Item $SealedSnapshot $TamperedSnapshot
    $bytes = [System.IO.File]::ReadAllBytes($TamperedSnapshot)
    # Mutate byte at offset 512 (first tile payload/header boundary)
    $tamperedOffset = 512
    $bytes[$tamperedOffset] = [byte]($bytes[$tamperedOffset] -bxor 0xFF)
    [System.IO.File]::WriteAllBytes($TamperedSnapshot, $bytes)
    Write-Host "Injected 1-byte corruption at offset $tamperedOffset (bit flipped)." -ForegroundColor Yellow

    Write-Host "Running s3a-cli verify on tampered archive (Expected: FAILURE / CORRUPTION DETECTED):"
    $tamperOut = & $CliExe verify $TamperedSnapshot 2>&1
    Write-Host ($tamperOut -join "`n")
    if ($tamperOut -match "FAILED|corrupt|CRC mismatch|Error|DEGRADED|repaired") {
        Write-Host "Integrity check succeeded: Corruption was positively detected!`n" -ForegroundColor Green
    } else {
        throw "Tampered verification did not detect corruption!"
    }

    # Step 6: Print Claim Evidence Chain end-to-end
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 6: Resolve and Print Claim Evidence Chain End-to-End                      " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe scholar proof $SealedSnapshot --claim 42
    if ($LASTEXITCODE -ne 0) { throw "scholar proof failed" }
    Write-Host ""

    # Step 7: Export Claim Evidence Chain as PROV-O JSON-LD
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 7: Export Claim Evidence Chain as W3C PROV-O JSON-LD                      " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe scholar proof $SealedSnapshot --claim 42 --json-ld
    if ($LASTEXITCODE -ne 0) { throw "scholar proof json-ld failed" }
    Write-Host ""

    # Step 8: Export Bibliography with Companion .proof.json
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host " STEP 8: Export Bibliography (BibTeX) with Companion Cryptographic Proof        " -ForegroundColor Cyan
    Write-Host "--------------------------------------------------------------------------------" -ForegroundColor Cyan
    & $CliExe scholar export-bib $SealedSnapshot --out $BibOutput --format bibtex
    if ($LASTEXITCODE -ne 0) { throw "scholar export-bib failed" }

    Write-Host "`nGenerated BibTeX ($BibOutput):" -ForegroundColor Gray
    Get-Content $BibOutput | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }

    Write-Host "`nCompanion Cryptographic Proof ($ProofOutput):" -ForegroundColor Gray
    Get-Content $ProofOutput | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
    Write-Host "`nBibliography and companion proof successfully generated!`n" -ForegroundColor Green

    Write-Host "================================================================================" -ForegroundColor Green
    Write-Host " DEMONSTRATION COMPLETE: ALL VERIFIED RESEARCH TRUST GATES PASSED!              " -ForegroundColor Green
    Write-Host "================================================================================" -ForegroundColor Green

} finally {
    if (-not $KeepArtifacts) {
        Remove-Item -Path $DemoDir -Recurse -Force -ErrorAction SilentlyContinue
    } else {
        Write-Host "Artifacts preserved at $DemoDir" -ForegroundColor Cyan
    }
}
