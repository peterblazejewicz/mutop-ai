<#
.SYNOPSIS
    Orchestrates the multi-engine AI benchmark suite for mutop-ai on LattePanda Mu Ultra.

.DESCRIPTION
    Executes benchmark scenarios from docs/benchmark-scenarios.md while mutop-ai captures
    high-resolution telemetry and interval summaries via --mark-file.

    SAFETY GATE:
    By default, this script operates strictly in -DryRun mode.
    Live execution on the board requires explicit -ExecuteLive and prior operator consultation.

.PARAMETER Scenario
    Scenario ID to run (e.g. '1A', '1B', '1C', '1D', '1E', '1F', '2A', '2B', or 'all'). Default: 'all'.

.PARAMETER DryRun
    When $true (default), plans and validates markers without launching compute engines on the board.

.PARAMETER ExecuteLive
    Mandatory safety gate switch required for live execution.

.PARAMETER MarkFile
    Path to the shared marker file for IPC interval profiling.

.PARAMETER SamplingIntervalMs
    Telemetry sampling interval for mutop-ai in milliseconds. Default: 200.

.PARAMETER ModelsRoot
    Root directory for models and Python virtual environment. Default: "C:\Users\peter\models".
#>

[CmdletBinding()]
param (
    [string]$Scenario = "all",
    [switch]$DryRun,
    [switch]$ExecuteLive,
    [switch]$Confirmed,
    [string]$MarkFile = "$PSScriptRoot\mutop_markers.txt",
    [int]$SamplingIntervalMs = 200,
    [string]$ModelsRoot = "C:\Users\peter\models"
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
if (-not $ScriptDir) { $ScriptDir = "C:\develop\mutop\scripts" }
if (-not $MarkFile -or $MarkFile -eq "\mutop_markers.txt" -or $MarkFile -eq "C:\mutop_markers.txt") {
    $MarkFile = Join-Path $ScriptDir "mutop_markers.txt"
}

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "         MUTOP-AI MULTI-ENGINE BENCHMARK ORCHESTRATOR (POWERSHELL)              " -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan

# 1. Resolve Python Interpreter (prefer hermetic venv from C:\Users\peter\models\.venv)
$VenvPython = Join-Path $ModelsRoot ".venv\Scripts\python.exe"
if (Test-Path $VenvPython) {
    $PythonExe = $VenvPython
    Write-Host "Using Hermetic venv Python : $PythonExe" -ForegroundColor Green
} else {
    $UvCmd = Get-Command uv -ErrorAction SilentlyContinue
    if ($UvCmd) {
        $PythonExe = "uv"
        $PythonPrefixArgs = @("run", "python")
        Write-Host "Using uv runtime           : uv run python" -ForegroundColor Green
    } else {
        $SystemPython = (Get-Command python -ErrorAction SilentlyContinue).Source
        if (-not $SystemPython) {
            $SystemPython = (Get-Command python3 -ErrorAction SilentlyContinue).Source
        }
        if (-not $SystemPython) {
            Write-Error "No Python interpreter or uv toolchain found. Run scripts/provision_mu.ps1 first."
            exit 1
        }
        $PythonExe = $SystemPython
        Write-Host "Using System Python        : $PythonExe" -ForegroundColor Yellow
    }
}

$HarnessPy = Join-Path $ScriptDir "benchmark_harness.py"
if (-not (Test-Path $HarnessPy)) {
    Write-Error "Could not locate benchmark harness at '$HarnessPy'."
    exit 1
}

# Resolve MarkFile path
$ResolvedMarkFile = [System.IO.Path]::GetFullPath($MarkFile)
Write-Host "Marker IPC File            : $ResolvedMarkFile" -ForegroundColor Yellow
Write-Host "Target Scenario            : $Scenario" -ForegroundColor Yellow
Write-Host "Sampling Rate              : $SamplingIntervalMs ms" -ForegroundColor Yellow

# Helper invocation function
function Invoke-BenchmarkHarness([string[]]$extraArgs) {
    if ($PythonExe -eq "uv") {
        & uv run python $HarnessPy @extraArgs
    } else {
        & $PythonExe $HarnessPy @extraArgs
    }
}

if (-not $ExecuteLive -or $DryRun) {
    Write-Host "`n[SAFETY NOTICE] Dry-run validation active. No live workloads will be launched." -ForegroundColor Green
    Invoke-BenchmarkHarness @("--scenario", $Scenario, "--mark-file", $ResolvedMarkFile, "--dry-run")
    Write-Host "`n[DRY RUN COMPLETE] Plan verified. To run live on board, operator consultation is required." -ForegroundColor Green
    exit 0
}

Write-Host "`n[CRITICAL SAFETY GATE CHECK]" -ForegroundColor Red
Write-Host "Before running live workloads on LattePanda Mu Ultra ('mu'), verify board availability with the operator!" -ForegroundColor Yellow
if (-not $Confirmed) {
    $Confirmation = Read-Host "Has the operator confirmed board occupancy for this benchmark run? (y/N)"
    if ($Confirmation -ne "y" -and $Confirmation -ne "Y") {
        Write-Warning "Benchmark cancelled. Board occupancy was not confirmed."
        exit 2
    }
} else {
    Write-Host "Board occupancy pre-confirmed by operator switch (-Confirmed)." -ForegroundColor Green
}

# 2. Locate mutop-ai executable
$MutopExe = (Get-Command "mutop-ai" -ErrorAction SilentlyContinue).Source
if (-not $MutopExe) {
    $LocalMutop = Join-Path $PSScriptRoot "..\target\release\mutop-ai.exe"
    if (Test-Path $LocalMutop) {
        $MutopExe = [System.IO.Path]::GetFullPath($LocalMutop)
    } elseif (Test-Path "C:\develop\mutop\mutop-ai.exe") {
        $MutopExe = "C:\develop\mutop\mutop-ai.exe"
    } else {
        Write-Error "Could not locate mutop-ai.exe. Ensure it is built in release mode or in PATH."
        exit 1
    }
}

Write-Host "`n>>> Initializing mark file: $ResolvedMarkFile" -ForegroundColor Cyan
Set-Content -Path $ResolvedMarkFile -Value "" -Encoding UTF8

# 3. Launch mutop-ai background collector
Write-Host ">>> Launching mutop-ai background telemetry collector..." -ForegroundColor Magenta
$TelemetryLog = Join-Path $ScriptDir "telemetry_${Scenario}.ndjson"
if (Test-Path $TelemetryLog) { Remove-Item $TelemetryLog -Force }

$MutopArgs = "-s $SamplingIntervalMs --mark-file `"$ResolvedMarkFile`" --proc llama-server,python --threads --summary"
Write-Host "    $MutopExe $MutopArgs" -ForegroundColor DarkGray
Write-Host "    Redirecting stream output to: $TelemetryLog" -ForegroundColor DarkGray

$MutopProc = Start-Process -FilePath $MutopExe -ArgumentList $MutopArgs -PassThru -NoNewWindow -RedirectStandardOutput $TelemetryLog
Start-Sleep -Milliseconds 500

try {
    # 4. Execute Live Benchmark Workloads
    Write-Host "`n>>> Executing Live Benchmark Workloads..." -ForegroundColor Green
    Invoke-BenchmarkHarness @("--scenario", $Scenario, "--mark-file", $ResolvedMarkFile, "--execute-live")
}
finally {
    # 5. Signal mutop-ai completion
    Write-Host "`n>>> Signaling benchmark completion..." -ForegroundColor Cyan
    Set-Content -Path $ResolvedMarkFile -Value "STREAM_END" -Encoding UTF8
    
    # Wait up to 5 seconds for mutop-ai to cleanly exit and write summary
    $waitCount = 0
    while (-not $MutopProc.HasExited -and $waitCount -lt 25) {
        Start-Sleep -Milliseconds 200
        $waitCount++
    }

    if (-not $MutopProc.HasExited) {
        Write-Host "Stopping mutop-ai telemetry process..." -ForegroundColor Yellow
        Stop-Process -Id $MutopProc.Id -Force -ErrorAction SilentlyContinue
    }
}

# Display summary from TelemetryLog if present
if (Test-Path $TelemetryLog) {
    Write-Host "`n================================================================================" -ForegroundColor Cyan
    Write-Host "                      MUTOP-AI TELEMETRY SUMMARY REPORT                         " -ForegroundColor Cyan
    Write-Host "================================================================================" -ForegroundColor Cyan
    $lines = Get-Content $TelemetryLog -Encoding UTF8
    $inSummary = $false
    foreach ($line in $lines) {
        if ($line -match "MUTOP-AI RUN SUMMARY REPORT" -or $line -match "SUMMARY REPORT") {
            $inSummary = $true
        }
        if ($inSummary) {
            Write-Host $line
        }
    }
}

Write-Host "`n================================================================================" -ForegroundColor Cyan
Write-Host "Benchmark execution and telemetry summary collection complete." -ForegroundColor Green
Write-Host "================================================================================" -ForegroundColor Cyan
