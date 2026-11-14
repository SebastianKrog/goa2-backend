"""Read the version 1 index-ordered fixture pack without extracting it."""

import ctypes
import hashlib
import json
import os
import re
import struct
import threading
from collections import OrderedDict
from functools import lru_cache
from pathlib import Path

FRAME_BYTES = 4 * 1024 * 1024
CACHE_FRAMES = 32
MAX_PAYLOAD_BYTES = 256 * 1024 * 1024
MAX_FRAMES = 100_000
MAX_INDEX_BYTES = 16 * 1024 * 1024


class FixturePack:
    def __init__(self, directory):
        self.archive = directory / "fixtures-4MiB.tar.zst"
        self.lock = threading.Lock()
        self.cache = OrderedDict()
        self.frames = []
        with self.archive.open("rb") as source:
            size = source.seek(0, os.SEEK_END)
            if size < 17:
                raise ValueError("Truncated fixture pack")
            source.seek(-9, os.SEEK_END)
            count, flags, magic = struct.unpack("<IBI", source.read(9))
            if magic != 0x8F92EAB1 or flags != 0 or not 0 < count <= MAX_FRAMES:
                raise ValueError("Unsupported fixture pack seek table")
            table = size - 17 - count * 8
            if table <= 0:
                raise ValueError("Invalid fixture pack seek table offset")
            source.seek(table)
            if struct.unpack("<II", source.read(8)) != (0x184D2A5E, count * 8 + 9):
                raise ValueError("Invalid fixture pack seek table header")
            offset = total = 0
            for i in range(count):
                packed, expanded = struct.unpack("<II", source.read(8))
                if not 0 < packed <= FRAME_BYTES + FRAME_BYTES // 128:
                    raise ValueError("Invalid compressed fixture frame size")
                if not 0 < expanded <= FRAME_BYTES or (i + 1 < count and expanded != FRAME_BYTES):
                    raise ValueError("Invalid decoded fixture frame size")
                self.frames.append((offset, packed, expanded))
                offset += packed
                total += expanded
            if offset != table:
                raise ValueError("Fixture pack frame sizes do not match the seek table")
        with (directory / "fixtures.index.json").open("rb") as source:
            encoded = source.read(MAX_INDEX_BYTES + 1)
        if len(encoded) > MAX_INDEX_BYTES:
            raise ValueError("Fixture pack index exceeds its size limit")
        index = json.loads(encoded)
        if index["schema"] not in (1, 2) or not isinstance(index["fixtures"], dict):
            raise ValueError("Unsupported fixture pack index")
        self.entries = index["fixtures"]
        for name, (start, length, digest) in self.entries.items():
            if (
                not name.endswith(".json")
                or any(part in ("", ".", "..") for part in name.split("/"))
                or "\\" in name
                or not isinstance(start, int)
                or not isinstance(length, int)
                or not 0 <= start <= total
                or not 0 <= length <= min(total - start, MAX_PAYLOAD_BYTES)
                or not re.fullmatch("[0-9a-f]{64}", digest)
            ):
                raise ValueError(f"Invalid fixture pack index entry: {name}")
        self.aliases = index.get("aliases", {})
        if index["schema"] == 1 and self.aliases:
            raise ValueError("Fixture pack aliases require index schema 2")
        for alias, (representative, old, new) in self.aliases.items():
            if (
                alias not in self.entries
                or representative not in self.entries
                or representative in self.aliases
                or self.entries[alias][:2] != self.entries[representative][:2]
                or not old.startswith('{"name":')
                or not old.endswith(",")
                or not new.startswith('{"name":')
                or not new.endswith(",")
                or len(old.encode()) > self.entries[alias][1]
                or self.entries[alias][1] - len(old.encode()) + len(new.encode())
                > MAX_PAYLOAD_BYTES
            ):
                raise ValueError(f"Invalid fixture pack alias: {alias}")
        try:
            self.zstd = ctypes.CDLL("libzstd.so.1")
        except OSError as error:
            raise RuntimeError("Reading a seekable fixture pack requires libzstd.so.1") from error
        self.zstd.ZSTD_decompress.argtypes = [
            ctypes.c_void_p,
            ctypes.c_size_t,
            ctypes.c_char_p,
            ctypes.c_size_t,
        ]
        self.zstd.ZSTD_decompress.restype = ctypes.c_size_t

    def _frame(self, number):
        with self.lock:
            if number in self.cache:
                self.cache.move_to_end(number)
                return self.cache[number]
            start, packed, expanded = self.frames[number]
            with self.archive.open("rb") as source:
                source.seek(start)
                compressed = source.read(packed)
            decoded = ctypes.create_string_buffer(expanded)
            if (
                len(compressed) != packed
                or self.zstd.ZSTD_decompress(decoded, expanded, compressed, packed) != expanded
            ):
                raise ValueError(f"Cannot decode fixture pack frame {number}")
            if len(self.cache) == CACHE_FRAMES:
                self.cache.popitem(last=False)
            frame = decoded.raw
            self.cache[number] = frame
            return frame

    def read(self, name):
        try:
            start, length, _ = self.entries[name]
        except KeyError:
            raise FileNotFoundError(f"Fixture {name} is missing from {self.archive}") from None
        parts = []
        while length:
            number, within = divmod(start, FRAME_BYTES)
            frame = self._frame(number)
            take = min(length, len(frame) - within)
            parts.append(memoryview(frame)[within : within + take])
            start += take
            length -= take
        data = b"".join(parts)
        if name in self.aliases:
            _, old, new = self.aliases[name]
            prefix = old.encode()
            if not data.startswith(prefix):
                raise ValueError(f"Fixture pack alias prefix mismatch: {name}")
            restored = new.encode() + data[len(prefix) :]
            if hashlib.sha256(restored).hexdigest() != self.entries[name][2]:
                raise ValueError(f"Fixture pack alias checksum mismatch: {name}")
            return restored
        return data


@lru_cache(maxsize=1)
def open_pack(directory):
    return FixturePack(Path(directory))
