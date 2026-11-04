#!/usr/bin/env python3
"""
common_markers.py - Atomic marker file writer for mutop-ai interval profiling.

In edgetop-core / mutop-ai, `consume_mark_file` reads non-empty lines from the mark file,
joins them with " | ", and atomically truncates the file. Consecutive markers form
labeled intervals in `MARKER PHASES & INTERVAL PROFILING` (e.g. [1] Phase : 1A_warmup -> 1A_infer).

This module writes a single plain-text token (e.g. `1A_npu_warmup\n`) and sleeps
for at least 0.25s so mutop-ai (-s 200) reliably samples the transition.
"""

import os
import sys
import time
from contextlib import contextmanager
from typing import Optional


class MarkerWriter:
    """Writes plain-text phase transition markers to a mark file."""

    def __init__(self, mark_file_path: Optional[str] = None):
        self.mark_file_path = os.path.abspath(mark_file_path) if mark_file_path else None
        if self.mark_file_path:
            parent_dir = os.path.dirname(self.mark_file_path)
            if parent_dir and not os.path.exists(parent_dir):
                os.makedirs(parent_dir, exist_ok=True)
            # Ensure file exists and is empty
            with open(self.mark_file_path, "w", encoding="utf-8") as f:
                f.write("")

    def set_marker(self, marker_text: str, sleep_sec: float = 0.25) -> None:
        """Writes a single plain-text marker line by overwriting the file and waits."""
        token = marker_text.strip()
        if not token:
            return
        if self.mark_file_path:
            with open(self.mark_file_path, "w", encoding="utf-8") as f:
                f.write(f"{token}\n")
                f.flush()
            if sleep_sec > 0:
                time.sleep(sleep_sec)

    @contextmanager
    def phase(self, start_marker: str, end_marker: Optional[str] = None, sleep_sec: float = 0.25):
        """Context manager setting a phase start marker, yielding, then setting end marker."""
        self.set_marker(start_marker, sleep_sec=sleep_sec)
        try:
            yield
        finally:
            end_token = end_marker if end_marker else f"{start_marker}_done"
            self.set_marker(end_token, sleep_sec=sleep_sec)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <mark_file_path> <marker_token> [sleep_sec]", file=sys.stderr)
        sys.exit(1)
    path = sys.argv[1]
    tok = sys.argv[2]
    sleep_t = float(sys.argv[3]) if len(sys.argv) > 3 else 0.25
    writer = MarkerWriter(path)
    writer.set_marker(tok, sleep_sec=sleep_t)
