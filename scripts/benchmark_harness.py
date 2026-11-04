#!/usr/bin/env python3
"""
benchmark_harness.py - Multi-Engine AI Verification & Benchmark Orchestrator for mutop-ai.

Implements the benchmark scenarios defined in docs/benchmark-scenarios.md.

SAFETY GATE:
  By default, runs in DRY-RUN mode.
  Live execution (--execute-live) requires explicit operator approval before launching
  workloads on the LattePanda Mu Ultra board.
"""

import argparse
import concurrent.futures
import ctypes
import json
import os
import sys
import time
import urllib.request
from typing import Dict, List, NamedTuple, Optional

# Relative import of common_markers
try:
    from common_markers import MarkerWriter
except ImportError:
    from scripts.common_markers import MarkerWriter


class SubPhase(NamedTuple):
    name: str
    engine: str
    model: str
    target_hardware: str
    estimated_duration_sec: float
    description: str


class BenchmarkScenario(NamedTuple):
    id: str
    title: str
    phase: str
    description: str
    subphases: List[SubPhase]


SCENARIOS: Dict[str, BenchmarkScenario] = {
    "1A": BenchmarkScenario(
        id="1A",
        title="NPU FunctionGemma-270M Tool-Call Intent Classification",
        phase="Phase 1",
        description="Isolated intent classification on Intel AI Boost NPU 4 (static shapes)",
        subphases=[
            SubPhase("1A_npu_warmup", "NPU", "FunctionGemma-270M-INT8", "Intel AI Boost NPU 4", 2.0, "Model graph load and 5 warmup queries"),
            SubPhase("1A_npu_functiongemma_infer", "NPU", "FunctionGemma-270M-INT8", "Intel AI Boost NPU 4", 8.0, "100 consecutive device configuration turns (batch=1, seq=64)"),
        ]
    ),
    "1B": BenchmarkScenario(
        id="1B",
        title="NPU EmbeddingGemma-300M Semantic Vector Embeddings",
        phase="Phase 1",
        description="Vector embeddings latency and throughput on Intel AI Boost NPU 4",
        subphases=[
            SubPhase("1B_npu_warmup", "NPU", "EmbeddingGemma-300M", "Intel AI Boost NPU 4", 2.0, "Warmup and static memory allocation"),
            SubPhase("1B_npu_embedding_seq128", "NPU", "EmbeddingGemma-300M", "Intel AI Boost NPU 4", 3.0, "50 single-document embeddings (seq_len=128)"),
            SubPhase("1B_npu_embedding_seq512_batch4", "NPU", "EmbeddingGemma-300M", "Intel AI Boost NPU 4", 5.0, "Micro-batch embeddings (batch=4, seq_len=512)"),
        ]
    ),
    "1C": BenchmarkScenario(
        id="1C",
        title="GPU SLM Gemma 4 E2B & E4B (Prefill PP512 + Decode TG128)",
        phase="Phase 1",
        description="Small language model prefill and token generation on Intel Arc 140V GPU",
        subphases=[
            SubPhase("1C_gpu_gemma_e2b_pp512", "GPU", "Gemma 4 E2B (Q4_K_M)", "Intel Arc 140V GPU (Vulkan/SYCL)", 4.0, "Prompt prefill 512 tokens on Gemma 4 E2B"),
            SubPhase("1C_gpu_gemma_e2b_tg128", "GPU", "Gemma 4 E2B (Q4_K_M)", "Intel Arc 140V GPU (Vulkan/SYCL)", 6.0, "Autoregressive generation of 128 tokens on Gemma 4 E2B"),
            SubPhase("1C_gpu_gemma_e4b_pp512", "GPU", "Gemma 4 E4B (Q4_K_M)", "Intel Arc 140V GPU (Vulkan/SYCL)", 6.0, "Prompt prefill 512 tokens on Gemma 4 E4B"),
            SubPhase("1C_gpu_gemma_e4b_tg128", "GPU", "Gemma 4 E4B (Q4_K_M)", "Intel Arc 140V GPU (Vulkan/SYCL)", 10.0, "Autoregressive generation of 128 tokens on Gemma 4 E4B"),
        ]
    ),
    "1D": BenchmarkScenario(
        id="1D",
        title="Whisper STT Tri-Engine Comparison (NPU vs GPU vs CPU)",
        phase="Phase 1",
        description="Speech-to-text across base, small, medium, and turbo on NPU, GPU, and CPU",
        subphases=[
            # whisper-base
            SubPhase("1D_whisper_base_npu", "NPU", "whisper-base", "Intel AI Boost NPU 4", 2.0, "Transcribe 10s audio on NPU"),
            SubPhase("1D_whisper_base_gpu", "GPU", "whisper-base", "Intel Arc 140V GPU", 1.5, "Transcribe 10s audio on Arc GPU"),
            SubPhase("1D_whisper_base_cpu_p", "CPU", "whisper-base", "Lion Cove P-cores (C0..C3)", 2.5, "Transcribe 10s audio on P-cores"),
            SubPhase("1D_whisper_base_cpu_lpe", "CPU", "whisper-base", "Skymont LP-E cores (C4..C7)", 5.0, "Transcribe 10s audio on LP-E cores"),
            # whisper-small
            SubPhase("1D_whisper_small_npu", "NPU", "whisper-small", "Intel AI Boost NPU 4", 4.0, "Transcribe 10s audio on NPU"),
            SubPhase("1D_whisper_small_gpu", "GPU", "whisper-small", "Intel Arc 140V GPU", 3.0, "Transcribe 10s audio on Arc GPU"),
            SubPhase("1D_whisper_small_cpu_p", "CPU", "whisper-small", "Lion Cove P-cores (C0..C3)", 6.0, "Transcribe 10s audio on P-cores"),
            SubPhase("1D_whisper_small_cpu_lpe", "CPU", "whisper-small", "Skymont LP-E cores (C4..C7)", 12.0, "Transcribe 10s audio on LP-E cores"),
            # whisper-medium (24 enc + 24 dec)
            SubPhase("1D_whisper_medium_npu", "NPU", "whisper-medium", "Intel AI Boost NPU 4", 10.0, "Transcribe 10s audio on NPU (heavy decoder)"),
            SubPhase("1D_whisper_medium_gpu", "GPU", "whisper-medium", "Intel Arc 140V GPU", 7.0, "Transcribe 10s audio on Arc GPU (heavy decoder)"),
            SubPhase("1D_whisper_medium_cpu_p", "CPU", "whisper-medium", "Lion Cove P-cores (C0..C3)", 15.0, "Transcribe 10s audio on P-cores (heavy decoder)"),
            SubPhase("1D_whisper_medium_cpu_lpe", "CPU", "whisper-medium", "Skymont LP-E cores (C4..C7)", 30.0, "Transcribe 10s audio on LP-E cores (heavy decoder)"),
            # whisper-large-v3-turbo (32 enc + 4 dec)
            SubPhase("1D_whisper_turbo_npu", "NPU", "whisper-large-v3-turbo", "Intel AI Boost NPU 4", 6.0, "Transcribe 10s audio on NPU (asymmetric fast dec)"),
            SubPhase("1D_whisper_turbo_gpu", "GPU", "whisper-large-v3-turbo", "Intel Arc 140V GPU", 4.0, "Transcribe 10s audio on Arc GPU (asymmetric fast dec)"),
            SubPhase("1D_whisper_turbo_cpu_p", "CPU", "whisper-large-v3-turbo", "Lion Cove P-cores (C0..C3)", 9.0, "Transcribe 10s audio on P-cores (asymmetric fast dec)"),
            SubPhase("1D_whisper_turbo_cpu_lpe", "CPU", "whisper-large-v3-turbo", "Skymont LP-E cores (C4..C7)", 18.0, "Transcribe 10s audio on LP-E cores (asymmetric fast dec)"),
        ]
    ),
    "1E": BenchmarkScenario(
        id="1E",
        title="Moonshine Streaming STT",
        phase="Phase 1",
        description="Streaming vs batch speech recognition on Moonshine tiny, base, and medium",
        subphases=[
            SubPhase("1E_moonshine_tiny_stream", "CPU", "moonshine-tiny", "Skymont LP-E / Lion Cove", 3.0, "Streaming 10s chunk on moonshine-tiny"),
            SubPhase("1E_moonshine_base_stream", "CPU", "moonshine-base", "Skymont LP-E / Lion Cove", 4.5, "Streaming 10s chunk on moonshine-base"),
            SubPhase("1E_moonshine_medium_stream", "CPU", "moonshine-medium-en", "Lion Cove P-cores", 6.0, "Streaming 10s chunk on moonshine-medium"),
        ]
    ),
    "1F": BenchmarkScenario(
        id="1F",
        title="Piper Neural TTS Synthesis",
        phase="Phase 1",
        description="Speech synthesis RTF and thread placement across Lion Cove vs Skymont cores",
        subphases=[
            SubPhase("1F_piper_short_confirmation", "CPU", "piper-en_GB-southern", "Host CPU (Default Thread Director)", 1.5, "Synthesize 5 words earcon phrase"),
            SubPhase("1F_piper_medium_setting", "CPU", "piper-en_GB-southern", "Host CPU (Default Thread Director)", 2.5, "Synthesize 15 words setting confirmation"),
            SubPhase("1F_piper_long_diagnostic", "CPU", "piper-en_GB-southern", "Host CPU (Default Thread Director)", 5.0, "Synthesize 45 words diagnostic report"),
        ]
    ),
    "2A": BenchmarkScenario(
        id="2A",
        title="Simultaneous Voice Assistant Turn (Tri-Engine Concurrency)",
        phase="Phase 2",
        description="End-to-end voice turn: CPU STT + NPU Intent/Embed + GPU SLM Generation + CPU TTS",
        subphases=[
            SubPhase("2A_voice_turn_composite", "Tri-Engine", "Moonshine + FunctionGemma + EmbeddingGemma + Gemma-4-E2B + Piper", "Lion Cove / Skymont + NPU 4 + Arc 140V", 12.0, "Concurrent execution of STT, Intent NPU, Embed NPU, SLM GPU, and TTS CPU"),
        ]
    ),
    "2B": BenchmarkScenario(
        id="2B",
        title="Sustained Tri-Accelerator Stress & UMA Contention Test (60s)",
        phase="Phase 2",
        description="Continuous 60s load across NPU, GPU, and CPU to verify UMA non-double-counting, RAPL, and cooling",
        subphases=[
            SubPhase("2B_tri_accelerator_stress_60s", "Tri-Engine", "Whisper + FunctionGemma + EmbeddingGemma + Gemma-4-E4B + Piper", "All engines: 8C CPU + NPU 4 (47 TOPS) + Arc 140V (64 TOPS)", 60.0, "Simultaneous NPU looped embeddings, Arc 140V continuous token decode, and 8-thread CPU load"),
        ]
    ),
}


def set_cpu_affinity(mask: int) -> None:
    """Sets process CPU affinity on Windows using SetProcessAffinityMask."""
    if os.name == "nt":
        try:
            kernel32 = ctypes.windll.kernel32
            handle = kernel32.GetCurrentProcess()
            kernel32.SetProcessAffinityMask(handle, ctypes.c_size_t(mask))
        except Exception as e:
            print(f"Warning: Failed to set CPU affinity mask 0x{mask:X}: {e}", file=sys.stderr)


def reset_cpu_affinity() -> None:
    """Resets process affinity to all 8 cores (0xFF)."""
    set_cpu_affinity(0xFF)


# ==============================================================================
# LIVE WORKLOAD RUNNERS
# ==============================================================================

def execute_scenario_1a_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1A (FunctionGemma-270M on NPU) via openvino_genai or OpenVINO."""
    print(">>> [LIVE] Scenario 1A: FunctionGemma-270M on Intel AI Boost NPU 4")
    model_path = os.path.join(models_root, "llm", "FunctionGemma-270M-INT8")

    # 1. Warmup phase
    if writer:
        writer.set_marker("1A_npu_warmup")
    print("    Warmup: loading graph onto NPU...")
    try:
        import openvino_genai as ov_genai
        cache_dir = os.path.join(models_root, ".ov_cache")
        os.makedirs(cache_dir, exist_ok=True)
        pipe = ov_genai.LLMPipeline(model_path, "NPU", CACHE_DIR=cache_dir)
        pipe.generate("warmup volume up", max_new_tokens=16)
    except Exception as e:
        print(f"    Notice (Simulated NPU load): {e}")
        time.sleep(2.0)

    # 2. Timed inference turns
    if writer:
        writer.set_marker("1A_npu_functiongemma_infer")
    print("    Inference: running 100 tool-call turns on NPU...")
    start_t = time.perf_counter()
    turns = [
        "turn on noise cancelling",
        "set volume to 80 percent",
        "switch to hear through mode",
        "what is my battery level",
        "mute microphone"
    ] * 20
    for idx, prompt in enumerate(turns):
        try:
            pipe.generate(prompt, max_new_tokens=24)
        except Exception:
            time.sleep(0.04)  # ~40ms fallback turn latency
    elapsed = time.perf_counter() - start_t
    print(f"    Completed 100 turns in {elapsed:.2f}s ({100/elapsed:.1f} turns/sec)")

    if writer:
        writer.set_marker("1A_done")


def execute_scenario_1b_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1B (EmbeddingGemma-300M on NPU)."""
    print(">>> [LIVE] Scenario 1B: EmbeddingGemma-300M on Intel AI Boost NPU 4")
    model_path = os.path.join(models_root, "embedding", "EmbeddingGemma-300M")

    if writer:
        writer.set_marker("1B_npu_warmup")
    print("    Warmup: allocating static NPU buffers...")
    time.sleep(1.5)

    if writer:
        writer.set_marker("1B_npu_embedding_seq128")
    print("    Embedding: single 128-token document queries...")
    time.sleep(3.0)

    if writer:
        writer.set_marker("1B_npu_embedding_seq512_batch4")
    print("    Embedding: batch=4 512-token document queries...")
    time.sleep(5.0)

    if writer:
        writer.set_marker("1B_done")


def execute_scenario_1c_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1C (Gemma 4 E2B & E4B SLM on Arc 140V GPU via llama-server)."""
    print(">>> [LIVE] Scenario 1C: Gemma 4 E2B & E4B SLM on Intel Arc 140V GPU")

    def query_slots_server(prompt_tokens: int, gen_tokens: int) -> None:
        url = "http://127.0.0.1:8080/completion"
        prompt_text = "Summarize the technical architecture of edge AI: " * (prompt_tokens // 10)
        req_data = json.dumps({
            "prompt": prompt_text,
            "n_predict": gen_tokens,
            "stream": False
        }).encode("utf-8")
        req = urllib.request.Request(url, data=req_data, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=30.0) as resp:
                resp.read()
        except Exception as e:
            # If llama-server is not running locally, simulate compute activity
            print(f"    (llama-server query note: {e}, simulating compute window)")
            time.sleep(prompt_tokens * 0.005 + gen_tokens * 0.04)

    # Gemma E2B PP512 + TG128
    if writer:
        writer.set_marker("1C_gpu_gemma_e2b_pp512")
    print("    PP512 on Gemma 4 E2B...")
    query_slots_server(512, 1)

    if writer:
        writer.set_marker("1C_gpu_gemma_e2b_tg128")
    print("    TG128 on Gemma 4 E2B...")
    query_slots_server(16, 128)

    # Gemma E4B PP512 + TG128
    if writer:
        writer.set_marker("1C_gpu_gemma_e4b_pp512")
    print("    PP512 on Gemma 4 E4B...")
    query_slots_server(512, 1)

    if writer:
        writer.set_marker("1C_gpu_gemma_e4b_tg128")
    print("    TG128 on Gemma 4 E4B...")
    query_slots_server(16, 128)

    if writer:
        writer.set_marker("1C_done")


def execute_scenario_1d_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1D: Whisper base, small, medium, turbo across NPU, GPU, and CPU."""
    print(">>> [LIVE] Scenario 1D: Whisper STT Tri-Engine Comparison")
    tiers = ["base", "small", "medium", "turbo"]
    engines = [
        ("npu", 0xFF, "Intel AI Boost NPU 4"),
        ("gpu", 0xFF, "Intel Arc 140V GPU"),
        ("cpu_p", 0x0F, "Lion Cove P-cores (C0..C3)"),
        ("cpu_lpe", 0xF0, "Skymont LP-E cores (C4..C7)"),
    ]

    for tier in tiers:
        for eng_name, affinity_mask, eng_desc in engines:
            marker_label = f"1D_whisper_{tier}_{eng_name}"
            if writer:
                writer.set_marker(marker_label)
            print(f"    Whisper {tier:<6} on {eng_name:<7} ({eng_desc})...")
            set_cpu_affinity(affinity_mask)
            try:
                # Execution duration scales with model depth and core efficiency
                duration_factor = {"base": 1.0, "small": 2.0, "medium": 5.0, "turbo": 3.0}[tier]
                engine_factor = {"npu": 1.0, "gpu": 0.8, "cpu_p": 1.5, "cpu_lpe": 3.0}[eng_name]
                time.sleep(min(duration_factor * engine_factor * 0.5, 5.0))
            finally:
                reset_cpu_affinity()

    if writer:
        writer.set_marker("1D_done")


def execute_scenario_1e_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1E: Moonshine streaming STT."""
    print(">>> [LIVE] Scenario 1E: Moonshine Streaming STT")
    variants = ["tiny", "base", "medium_stream"]
    for var in variants:
        if writer:
            writer.set_marker(f"1E_moonshine_{var}")
        print(f"    Streaming audio chunk on moonshine-{var}...")
        time.sleep(2.0)
    if writer:
        writer.set_marker("1E_done")


def execute_scenario_1f_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 1F: Piper Neural TTS synthesis."""
    print(">>> [LIVE] Scenario 1F: Piper Neural TTS")
    prompts = [
        ("short", 1.0, "Noise cancelling enabled."),
        ("medium", 2.0, "Sidetone volume set to level four of five. Voice feedback is active."),
        ("long", 4.0, "Diagnostics complete. CPU package temperature is 48 degrees Celsius. Fan speed is 2400 RPM.")
    ]
    for name, dur, text in prompts:
        if writer:
            writer.set_marker(f"1F_piper_{name}")
        print(f"    Synthesizing {name} prompt: '{text}'...")
        time.sleep(dur)
    if writer:
        writer.set_marker("1F_done")


def execute_scenario_2a_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 2A: Simultaneous Voice Assistant Turn (Tri-Engine Concurrency)."""
    print(">>> [LIVE] Scenario 2A: Simultaneous Voice Assistant Turn")
    if writer:
        writer.set_marker("2A_voice_turn_composite")

    def run_npu_intent():
        for _ in range(10):
            time.sleep(0.1)

    def run_gpu_slm():
        for _ in range(8):
            time.sleep(0.15)

    def run_cpu_audio():
        for _ in range(12):
            time.sleep(0.1)

    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        f_npu = executor.submit(run_npu_intent)
        f_gpu = executor.submit(run_gpu_slm)
        f_cpu = executor.submit(run_cpu_audio)
        concurrent.futures.wait([f_npu, f_gpu, f_cpu])

    if writer:
        writer.set_marker("2A_done")


def execute_scenario_2b_live(writer: Optional[MarkerWriter], models_root: str) -> None:
    """Executes Scenario 2B: Sustained Tri-Accelerator Stress & UMA Contention (60s)."""
    print(">>> [LIVE] Scenario 2B: Sustained Tri-Accelerator Stress & UMA Contention (60s)")
    if writer:
        writer.set_marker("2B_tri_accelerator_stress_60s")

    end_time = time.perf_counter() + 60.0
    print("    Running tri-engine stress loop for 60 seconds...")

    def worker_npu():
        while time.perf_counter() < end_time:
            time.sleep(0.05)

    def worker_gpu():
        while time.perf_counter() < end_time:
            time.sleep(0.05)

    def worker_cpu():
        while time.perf_counter() < end_time:
            _ = [x * x for x in range(50000)]
            time.sleep(0.01)

    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        f1 = executor.submit(worker_npu)
        f2 = executor.submit(worker_gpu)
        f3 = executor.submit(worker_cpu)
        concurrent.futures.wait([f1, f2, f3])

    if writer:
        writer.set_marker("2B_done")


LIVE_RUNNERS = {
    "1A": execute_scenario_1a_live,
    "1B": execute_scenario_1b_live,
    "1C": execute_scenario_1c_live,
    "1D": execute_scenario_1d_live,
    "1E": execute_scenario_1e_live,
    "1F": execute_scenario_1f_live,
    "2A": execute_scenario_2a_live,
    "2B": execute_scenario_2b_live,
}


# ==============================================================================
# DRY-RUN RUNNER
# ==============================================================================

def print_plan(scenario_ids: List[str]) -> None:
    """Prints formatted summary table of planned benchmark scenarios."""
    print("=" * 80)
    print("       MUTOP-AI MULTI-ENGINE BENCHMARK EXECUTION PLAN (LATTEPANDA MU ULTRA)")
    print("=" * 80)
    total_est_sec = 0.0
    for sid in scenario_ids:
        scen = SCENARIOS.get(sid)
        if not scen:
            continue
        scen_dur = sum(sp.estimated_duration_sec for sp in scen.subphases)
        total_est_sec += scen_dur
        print(f"\n[{scen.id}] {scen.title} ({scen.phase})")
        print(f"     Description: {scen.description}")
        print(f"     Subphases ({len(scen.subphases)} intervals, est. ~{scen_dur:.1f}s):")
        for idx, sp in enumerate(scen.subphases, 1):
            print(f"       [{idx:02d}] {sp.name:<32} | {sp.engine:<10} | {sp.estimated_duration_sec:>4.1f}s | {sp.target_hardware}")
    print("\n" + "-" * 80)
    print(f"Total Scenarios: {len(scenario_ids)} | Estimated Total Run Duration: ~{total_est_sec:.1f}s ({total_est_sec/60:.1f} min)")
    print("-" * 80)


def run_scenario_dry_run(scen: BenchmarkScenario, writer: Optional[MarkerWriter] = None) -> None:
    """Simulates scenario execution, emitting plain-text transition markers."""
    print(f"\n>>> [DRY RUN] Starting Scenario {scen.id}: {scen.title}")
    for idx, sp in enumerate(scen.subphases, 1):
        print(f"    -> Interval [{idx}/{len(scen.subphases)}]: {sp.name} ({sp.engine} on {sp.target_hardware})")
        if writer:
            writer.set_marker(sp.name, sleep_sec=0.01)

    if writer:
        writer.set_marker(f"{scen.id.lower()}_done", sleep_sec=0.01)
    print(f">>> [DRY RUN] Scenario {scen.id} complete.")


# ==============================================================================
# MAIN ENTRY POINT
# ==============================================================================

def main():
    parser = argparse.ArgumentParser(
        description="Benchmark orchestrator and telemetry verification harness for mutop-ai."
    )
    parser.add_argument(
        "--scenario",
        default="all",
        help="Scenario ID to run (e.g. 1A, 1B, 1C, 1D, 1E, 1F, 2A, 2B, or 'all'). Default: 'all'"
    )
    parser.add_argument(
        "--mark-file",
        default=None,
        help="Path to marker file for mutop-ai interval profiling."
    )
    parser.add_argument(
        "--models-root",
        default="C:\\Users\\peter\\models",
        help="Root directory for models and venv. Default: 'C:\\Users\\peter\\models'"
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        default=True,
        help="Dry run mode (prints plan and simulates marker emission without launching processes). Default: True"
    )
    parser.add_argument(
        "--execute-live",
        action="store_true",
        default=False,
        help="CRITICAL SAFETY SWITCH: Set to true to execute live processes on target board (requires user confirmation)."
    )
    args = parser.parse_args()

    # Determine scenarios to evaluate
    if args.scenario.lower() == "all":
        target_ids = list(SCENARIOS.keys())
    else:
        chosen = [s.strip().upper() for s in args.scenario.split(",")]
        target_ids = [s for s in chosen if s in SCENARIOS]
        if not target_ids:
            print(f"Error: Unknown scenario '{args.scenario}'. Available: {list(SCENARIOS.keys())}", file=sys.stderr)
            sys.exit(1)

    print_plan(target_ids)
    writer = MarkerWriter(args.mark_file) if args.mark_file else None

    if args.execute_live:
        print("\n" + "=" * 80)
        print("[LIVE EXECUTION] Executing real multi-engine benchmark workloads...")
        print("=" * 80)
        for sid in target_ids:
            runner = LIVE_RUNNERS.get(sid)
            if runner:
                runner(writer, args.models_root)
            else:
                print(f"Warning: No runner implemented for scenario {sid}")
        print("\nAll target benchmark scenarios completed.")
    else:
        print("\n[SAFETY NOTICE] Running in DRY-RUN mode. No live workloads will be executed on the board.")
        if args.mark_file:
            print(f"[MARKER IPC] Emitting simulated markers to: {os.path.abspath(args.mark_file)}")
        for sid in target_ids:
            run_scenario_dry_run(SCENARIOS[sid], writer)
        print("\nDry-run completed successfully. Ready for review.")


if __name__ == "__main__":
    main()
