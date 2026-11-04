"""Guard profile attribution against mismatched binaries and ELF offset errors."""

from pathlib import Path

import pytest
from native_profile import executable_segments, require_build_id, resolve_samples, symbol_ranges


def test_nonzero_segment_delta_and_merged_lto_aliases():
    segments = executable_segments(
        "LOAD 0x0 0x0 0x0 0x1000 0x1000 R 0x1000\n"
        "LOAD 0x1000 0x2000 0x2000 0x200 0x200 R E 0x1000\n"
    )
    symbols = symbol_ranges(
        "00002000 00000040 t hash<Symbol>\n"
        "00002000 00000040 t hash<ObjectId>\n"
        "00002080 00000020 t next_function\n"
    )
    result = resolve_samples(
        "# header\n25%;2;worker;[.] 0x1010\n"
        "25%;2;worker;[.] 0x1085\n50%;4;libc.so.6;[.] 0x1010\n",
        segments,
        symbols,
        "worker",
    )
    assert result["total_samples"] == 8
    assert result["functions"] == [
        {"samples": 4, "percent": 50.0, "symbols": ["libc.so.6"]},
        {"samples": 2, "percent": 25.0, "symbols": ["hash<ObjectId>", "hash<Symbol>"]},
        {"samples": 2, "percent": 25.0, "symbols": ["next_function"]},
    ]
    gap = resolve_samples("100%;1;worker;[.] 0x1050", segments, symbols, "worker")
    assert gap["functions"][0]["symbols"] == ["unresolved executable offset 0x1050"]
    with pytest.raises(ValueError, match="outside executable"):
        resolve_samples("100%;1;worker;[.] 0x2000", segments, symbols, "worker")


def test_profile_must_match_build_id_and_recorded_absolute_binary_path():
    notes = "Build ID: abc123"
    binary = Path("/tmp/worker")
    assert require_build_id(notes, "abc123 /tmp/worker\n123 /lib/libc.so.6", binary) == "abc123"
    for listing in ["def456 /tmp/worker", "abc123 /tmp/other", ""]:
        with pytest.raises(ValueError, match="does not match"):
            require_build_id(notes, listing, binary)
    with pytest.raises(ValueError, match="no build ID"):
        require_build_id("", "abc123 /tmp/worker", binary)
