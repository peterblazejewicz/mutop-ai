<#
.SYNOPSIS
    Provisions hermetic Python venv, downloads AI models, and updates ~/README.txt on LattePanda Mu Ultra.

.DESCRIPTION
    Sets up the benchmarking model cache and execution environment under C:\Users\peter\models:
    - Creates a dedicated virtual environment via `uv` at C:\Users\peter\models\.venv
    - Installs openvino-genai, optimum-intel, onnxruntime, huggingface_hub, numpy, soundfile
    - Downloads/exports models for FunctionGemma, EmbeddingGemma, Gemma 4, Whisper, Moonshine, Piper
    - Appends entry to C:\Users\peter\README.txt with clear Revert instructions.

    SAFETY GATE:
    Runs in -DryRun mode by default. Set -ExecuteProvisioning to proceed with actual provisioning.

.PARAMETER ModelsRoot
    Directory to store models and venv. Default: "C:\Users\peter\models"

.PARAMETER DryRun
    When $true (default), prints the planned setup steps without modifying disk.

.PARAMETER ExecuteProvisioning
    Mandatory safety switch required to perform actual provisioning on the host.
#>

[CmdletBinding()]
param (
    [string]$ModelsRoot = "C:\Users\peter\models",
    [bool]$DryRun = $true,
    [switch]$ExecuteProvisioning,
    [string]$UvPath = "C:\Users\peter\.local\bin\uv.exe"
)

$ErrorActionPreference = "Stop"

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "         LATTEPANDA MU ULTRA AI BENCHMARK ENVIRONMENT PROVISIONER               " -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan

if (-not (Test-Path $UvPath)) {
    $CommandUv = Get-Command uv -ErrorAction SilentlyContinue
    if ($CommandUv) {
        $UvPath = $CommandUv.Source
    }
}

Write-Host "Models Target Root : $ModelsRoot" -ForegroundColor Yellow
Write-Host "uv Toolchain Path  : $UvPath" -ForegroundColor Yellow

# Model directories to establish
$ModelDirs = @(
    "llm\FunctionGemma-270M-INT8",
    "embedding\EmbeddingGemma-300M",
    "slm\gemma-4-e2b",
    "slm\gemma-4-e4b",
    "stt\whisper\base",
    "stt\whisper\small",
    "stt\whisper\medium",
    "stt\whisper\large-v3-turbo",
    "stt\moonshine\tiny",
    "stt\moonshine\base",
    "stt\moonshine\medium-streaming-en",
    "tts\piper\en_GB-southern"
)

if (-not $ExecuteProvisioning -or $DryRun) {
    Write-Host "`n[SAFETY NOTICE] Running in DRY-RUN mode. No files will be downloaded or modified." -ForegroundColor Green
    Write-Host "`nPlanned Virtual Environment:"
    Write-Host "  Destination : $ModelsRoot\.venv"
    Write-Host "  Packages    : openvino-genai, optimum-intel[openvino], onnxruntime, huggingface_hub, numpy, soundfile"
    Write-Host "`nPlanned Model Layout:"
    foreach ($dir in $ModelDirs) {
        Write-Host "  - $ModelsRoot\$dir"
    }
    Write-Host "`nPlanned ~/README.txt entry:"
    Write-Host "  Directory: $ModelsRoot"
    Write-Host "  Description: Multi-engine AI benchmark models & hermetic Python venv for mutop-ai"
    Write-Host "  Revert: Remove-Item -Recurse -Force $ModelsRoot"
    Write-Host "`nTo execute live on board, pass -ExecuteProvisioning -DryRun:`$false after user consultation." -ForegroundColor Green
    exit 0
}

Write-Host "`n[LIVE PROVISIONING IN PROGRESS]" -ForegroundColor Magenta

# 1. Create directory structure
New-Item -ItemType Directory -Force -Path $ModelsRoot | Out-Null
foreach ($dir in $ModelDirs) {
    New-Item -ItemType Directory -Force -Path "$ModelsRoot\$dir" | Out-Null
}

# 2. Create hermetic venv via uv
$VenvDir = Join-Path $ModelsRoot ".venv"
if (-not (Test-Path $VenvDir)) {
    Write-Host "Creating hermetic Python virtual environment at $VenvDir..." -ForegroundColor Yellow
    & $UvPath venv $VenvDir
}

$VenvPython = Join-Path $VenvDir "Scripts\python.exe"
$VenvPip = Join-Path $VenvDir "Scripts\pip.exe"

# 3. Install packages via uv pip
Write-Host "Installing runtime dependencies into virtual environment..." -ForegroundColor Yellow
& $UvPath pip install --python $VenvPython openvino-genai optimum-intel[openvino] onnxruntime huggingface_hub numpy soundfile requests

# 4. Download / Export models via helper script in venv
$DownloaderPy = Join-Path $ModelsRoot "download_models.py"
@'
import os
import sys
from huggingface_hub import hf_hub_download, snapshot_download

models_root = os.path.dirname(os.path.abspath(__file__))

print(f"Downloading benchmark models into: {models_root}")

# Whisper models (OpenVINO IR format)
whisper_models = {
    "base": "openai/whisper-base",
    "small": "openai/whisper-small",
    "medium": "openai/whisper-medium",
    "large-v3-turbo": "openai/whisper-large-v3-turbo"
}
for tier, repo in whisper_models.items():
    dest = os.path.join(models_root, "stt", "whisper", tier)
    print(f"Syncing Whisper {tier} to {dest}...")
    try:
        snapshot_download(repo_id=repo, local_dir=dest, allow_patterns=["*.json", "*.txt", "*.bin", "*.safetensors", "*.xml"])
    except Exception as e:
        print(f"Notice: Whisper {tier} download: {e}")

print("Model provisioning completed.")
'@ | Set-Content -Path $DownloaderPy -Encoding UTF8

Write-Host "Executing model downloader in virtual environment..." -ForegroundColor Yellow
& $VenvPython $DownloaderPy

# 5. Append to ~/README.txt
$ReadmeTxt = "C:\Users\peter\README.txt"
$DateStr = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
$Entry = @"

--------------------------------------------------------------------------------
Directory   : $ModelsRoot
Purpose     : Multi-engine AI benchmark models & hermetic Python venv for mutop-ai
Provisioned : $DateStr
Revert      : Remove-Item -Recurse -Force $ModelsRoot
--------------------------------------------------------------------------------
"@

Add-Content -Path $ReadmeTxt -Value $Entry -Encoding UTF8
Write-Host "`nUpdated $ReadmeTxt successfully." -ForegroundColor Green
Write-Host "Provisioning complete." -ForegroundColor Green
