"""Durable content-addressed starts and reports for the offline arena."""

import json
import os
import re
from hashlib import file_digest, sha256
from pathlib import Path
from tempfile import NamedTemporaryFile

from oracle import canonical

REPORT_SCHEMA = 1


def sha256_file(path):
    with path.open("rb") as source:
        return file_digest(source, "sha256").hexdigest()


def _identity(value):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise ValueError("invalid arena SHA-256 identity")
    return value


def _sync_directory(path):
    if hasattr(os, "O_DIRECTORY"):
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def _ensure_directory(path):
    if path.is_dir():
        return
    _ensure_directory(path.parent)
    path.mkdir(exist_ok=True)
    _sync_directory(path.parent)


def _write(path, data):
    _ensure_directory(path.parent)
    temporary = None
    try:
        with NamedTemporaryFile(dir=path.parent, delete=False) as output:
            temporary = Path(output.name)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        _sync_directory(path.parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _put(store, category, value):
    data = canonical(value).encode("utf-8")
    identity = sha256(data).hexdigest()
    path = store / category / f"{identity}.json"
    try:
        existing = path.read_bytes()
    except FileNotFoundError:
        _write(path, data)
    else:
        if existing != data:
            raise ValueError(f"arena {category} integrity check failed for {identity}")
    return identity


def _get(store, category, identity):
    data = (store / category / f"{_identity(identity)}.json").read_bytes()
    if sha256(data).hexdigest() != identity:
        raise ValueError(f"arena {category} integrity check failed for {identity}")
    return json.loads(data)


def save_start(store, envelope):
    return _put(store, "starts", envelope)


def load_start(store, identity):
    return _get(store, "starts", identity)


def save_report(store, envelope, result):
    start = save_start(store, envelope)
    return _put(store, "reports", dict(result, schema_version=REPORT_SCHEMA, start_sha256=start))


def read_report(store, identity):
    report = _get(store, "reports", identity)
    if report["schema_version"] != REPORT_SCHEMA:
        raise ValueError("unsupported arena report schema")
    return report


def load_report(store, identity):
    report = read_report(store, identity)
    return report, load_start(store, report["start_sha256"])


def save_verification(store, identity, result):
    _write(store / "verification" / f"{_identity(identity)}.json",
           canonical(result).encode("utf-8"))
