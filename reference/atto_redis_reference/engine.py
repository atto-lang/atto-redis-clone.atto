"""Crash-consistent, sharded key/value reference engine for AttoLang conformance.

Python is an executable design oracle, NOT the AttoLang production backend.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from threading import RLock, Thread
from typing import Optional
import json
import os
import queue
import re
import struct
import time
import uuid
import zlib

WAL_MAGIC = b"AWL1"
SST_MAGIC = b"AST1"
FRAME = struct.Struct(">4sII")  # magic, payload length, crc32
RECORD = struct.Struct(">QBII") # sequence, op, key length, value length
SET = 1
DELETE = 2
SEGMENT_NAME = re.compile(r"^shard-[0-9]{3}-[0-9a-f]{32}\.sst$")


class StoreError(Exception):
    pass


class CorruptData(StoreError):
    pass


class Backpressure(StoreError):
    pass


@dataclass(frozen=True)
class Config:
    shards: int = 64
    spill_bytes: int = 64 * 1024 * 1024
    pending_limit: int = 4
    max_key_bytes: int = 1024 * 1024
    max_value_bytes: int = 64 * 1024 * 1024
    fsync_policy: str = "always"  # 'everysec' can lose up to ~1s on power failure.

    def __post_init__(self) -> None:
        if not 1 <= self.shards <= 256 or self.spill_bytes < 1 or self.pending_limit < 1:
            raise ValueError("invalid shard, spill, or backpressure limits")
        if self.max_key_bytes < 1 or self.max_value_bytes < 0:
            raise ValueError("invalid key/value limits")
        if self.fsync_policy not in ("always", "everysec"):
            raise ValueError("unknown fsync policy")


@dataclass(frozen=True)
class Entry:
    seq: int
    value: Optional[bytes]  # None is a DEL tombstone; b'' is an ordinary value


def sync_directory(directory: Path) -> None:
    if os.name == "nt":
        return  # Windows directory durability needs platform-specific support.
    fd = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def encode_frame(magic: bytes, seq: int, key: bytes, value: Optional[bytes]) -> bytes:
    op = DELETE if value is None else SET
    val = b"" if value is None else value
    payload = RECORD.pack(seq, op, len(key), len(val)) + key + val
    return FRAME.pack(magic, len(payload), zlib.crc32(payload)) + payload


def scan_frames(path: Path, magic: bytes, max_frame: int, *, salvage_tail: bool = False):
    """Yield (byte_offset, sequence, key, value). Full corrupt frames always fail closed."""
    with path.open("r+b" if salvage_tail else "rb") as fh:
        prev_seq = -1
        while True:
            start = fh.tell()
            hdr = fh.read(FRAME.size)
            if not hdr:
                return
            if len(hdr) < FRAME.size:
                if salvage_tail:
                    fh.truncate(start)
                    fh.flush()
                    os.fsync(fh.fileno())
                    return
                raise CorruptData(f"truncated frame header: {path}")
            got_magic, n, crc = FRAME.unpack(hdr)
            if got_magic != magic or n < RECORD.size or n > max_frame:
                raise CorruptData(f"invalid frame header at {path}:{start}")
            payload = fh.read(n)
            if len(payload) < n:
                if salvage_tail:
                    fh.truncate(start)
                    fh.flush()
                    os.fsync(fh.fileno())
                    return
                raise CorruptData(f"truncated payload: {path}:{start}")
            if zlib.crc32(payload) != crc:
                raise CorruptData(f"checksum mismatch: {path}:{start}")
            seq, op, nk, nv = RECORD.unpack_from(payload)
            if op not in (SET, DELETE) or (op == DELETE and nv != 0) or nk == 0:
                raise CorruptData(f"invalid record: {path}:{start}")
            if RECORD.size + nk + nv != n or seq <= prev_seq:
                raise CorruptData(f"malformed/out-of-order record: {path}:{start}")
            prev_seq = seq
            key = payload[RECORD.size:RECORD.size + nk]
            value = None if op == DELETE else payload[RECORD.size + nk:]
            yield start, seq, key, value


class Wal:
    def __init__(self, path: Path, config: Config):
        self.path = path
        self.max_frame = RECORD.size + config.max_key_bytes + config.max_value_bytes
        self.policy = config.fsync_policy
        self.lock = RLock()
        self.seq = 0
        self.last_sync = time.monotonic()
        path.touch(exist_ok=True)
        self.recovered = list(scan_frames(path, WAL_MAGIC, self.max_frame, salvage_tail=True))
        if self.recovered:
            self.seq = self.recovered[-1][1]
        self.fh = path.open("ab", buffering=0)

    def append(self, key: bytes, value: Optional[bytes]) -> int:
        with self.lock:
            seq = self.seq + 1
            self.fh.write(encode_frame(WAL_MAGIC, seq, key, value))
            self.fh.flush()
            now = time.monotonic()
            if self.policy == "always" or now - self.last_sync >= 1.0:
                os.fsync(self.fh.fileno())
                self.last_sync = now
            self.seq = seq
            return seq

    def rewrite_after(self, boundary: int) -> None:
        """Discard checkpoint-covered records only AFTER the manifest is durable."""
        with self.lock:
            tmp = self.path.with_suffix(".log.tmp")
            try:
                with tmp.open("wb") as dest:
                    for _, seq, key, val in scan_frames(self.path, WAL_MAGIC, self.max_frame):
                        if seq > boundary:
                            dest.write(encode_frame(WAL_MAGIC, seq, key, val))
                    dest.flush()
                    os.fsync(dest.fileno())
                self.fh.close()
                os.replace(tmp, self.path)
                sync_directory(self.path.parent)
            finally:
                if not self.fh.closed:
                    self.fh.close()
                self.fh = self.path.open("ab", buffering=0)
                tmp.unlink(missing_ok=True)

    def close(self) -> None:
        with self.lock:
            if not self.fh.closed:
                self.fh.flush()
                # 'everysec' policy becomes durable on graceful shutdown.
                os.fsync(self.fh.fileno())
                self.fh.close()


@dataclass
class Segment:
    path: Path
    index: dict[bytes, int]

    @classmethod
    def load(cls, path: Path, max_frame: int) -> "Segment":
        index = {}
        last_key = None
        for offset, _, key, _ in scan_frames(path, SST_MAGIC, max_frame):
            if last_key is not None and key <= last_key:
                raise CorruptData(f"segment keys not sorted: {path}")
            index[key] = offset
            last_key = key
        return cls(path, index)

    @classmethod
    def create(cls, directory: Path, shard_id: int, data: dict[bytes, Entry], max_frame: int) -> "Segment":
        name = f"shard-{shard_id:03d}-{uuid.uuid4().hex}.sst"
        path = directory / name
        tmp = directory / (name + ".tmp")
        try:
            with tmp.open("xb") as fh:
                for key, entry in sorted(data.items()):
                    fh.write(encode_frame(SST_MAGIC, entry.seq, key, entry.value))
                fh.flush()
                os.fsync(fh.fileno())
            os.replace(tmp, path)
            sync_directory(directory)
            return cls.load(path, max_frame)
        finally:
            tmp.unlink(missing_ok=True)

    def get(self, key: bytes, max_frame: int) -> Optional[Entry]:
        offset = self.index.get(key)
        if offset is None:
            return None
        with self.path.open("rb") as fh:
            fh.seek(offset)
            hdr = fh.read(FRAME.size)
            if len(hdr) != FRAME.size:
                raise CorruptData("missing indexed segment frame")
            magic, n, crc = FRAME.unpack(hdr)
            if magic != SST_MAGIC or n > max_frame or n < RECORD.size:
                raise CorruptData("invalid indexed segment frame")
            payload = fh.read(n)
            if len(payload) != n or zlib.crc32(payload) != crc:
                raise CorruptData("corrupt indexed segment frame")
            seq, op, nk, nv = RECORD.unpack_from(payload)
            if op not in (SET, DELETE) or (op == DELETE and nv != 0):
                raise CorruptData("invalid indexed record operation")
            if nk + nv + RECORD.size != n or payload[RECORD.size:RECORD.size + nk] != key:
                raise CorruptData("indexed segment key mismatch")
            return Entry(seq, None if op == DELETE else payload[RECORD.size + nk:])


@dataclass
class Shard:
    lock: RLock = field(default_factory=RLock)
    active: dict[bytes, Entry] = field(default_factory=dict)
    active_bytes: int = 0
    pending: deque = field(default_factory=deque)
    segments: list[Segment] = field(default_factory=list)


class Engine:
    """Thread-safe sharded engine. Each shard has an independent mutation lock."""

    def __init__(self, directory: str | Path, config: Config = Config()):
        self.dir = Path(directory)
        self.dir.mkdir(parents=True, exist_ok=True)
        self.config = config
        self.gate = RLock()  # short global checkpoint cut; WAL establishes total LSN order
        self.manifest_lock = RLock()
        self.shards = [Shard() for _ in range(config.shards)]
        self.manifest_path = self.dir / "MANIFEST.json"
        self.max_frame = RECORD.size + config.max_key_bytes + config.max_value_bytes
        self.manifest = self._read_manifest()
        for i, shard in enumerate(self.shards):
            for filename in self.manifest["segments"][i]:
                shard.segments.append(Segment.load(self.dir / filename, self.max_frame))
        self.wal = Wal(self.dir / "WAL.log", config)
        self.wal.seq = max(self.wal.seq, *self.manifest["checkpoints"])
        for _, seq, key, val in self.wal.recovered:
            index = self.shard_index(key)
            if seq > self.manifest["checkpoints"][index]:
                shard = self.shards[index]
                old = shard.active.get(key)
                shard.active[key] = Entry(seq, val)
                shard.active_bytes += self._entry_size(key, val) - (
                    self._entry_size(key, old.value) if old is not None else 0
                )
        self.fail: Optional[BaseException] = None
        self.closed = False
        self.jobs: queue.Queue = queue.Queue()
        self.worker = Thread(target=self._flush_worker, daemon=True, name="atto-sst-flush")
        self.worker.start()

    def _read_manifest(self) -> dict:
        if not self.manifest_path.exists():
            return {"format": 1, "shards": self.config.shards,
                    "checkpoints": [0] * self.config.shards,
                    "segments": [[] for _ in range(self.config.shards)]}
        obj = json.loads(self.manifest_path.read_text("utf-8"))
        if obj.get("format") != 1 or obj.get("shards") != self.config.shards:
            raise CorruptData("manifest version/shard count mismatch")
        cps, lists = obj.get("checkpoints"), obj.get("segments")
        if not isinstance(cps, list) or not isinstance(lists, list) or (
            len(cps) != self.config.shards or len(lists) != self.config.shards
        ):
            raise CorruptData("malformed manifest")
        for i, names in enumerate(lists):
            if not isinstance(cps[i], int) or cps[i] < 0 or not isinstance(names, list):
                raise CorruptData("invalid manifest entry")
            if len(names) != len(set(names)):
                raise CorruptData("duplicate segment")
            if any(not isinstance(n, str) or not SEGMENT_NAME.fullmatch(n) or
                   not n.startswith(f"shard-{i:03d}-") for n in names):
                raise CorruptData("unsafe segment filename")
        return obj

    def _save_manifest(self) -> None:
        tmp = self.dir / "MANIFEST.json.tmp"
        with tmp.open("w", encoding="utf-8") as fh:
            json.dump(self.manifest, fh, sort_keys=True, separators=(",", ":"))
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, self.manifest_path)
        sync_directory(self.dir)

    def shard_index(self, key: bytes) -> int:
        return zlib.crc32(key) % self.config.shards

    @staticmethod
    def _entry_size(key: bytes, val: Optional[bytes]) -> int:
        return len(key) + (len(val) if val is not None else 0) + 48

    def _validate(self, key: bytes, value: Optional[bytes] = None) -> None:
        if not isinstance(key, bytes) or not 1 <= len(key) <= self.config.max_key_bytes:
            raise ValueError("key must be nonempty bytes within configured limit")
        if value is not None and (
            not isinstance(value, bytes) or len(value) > self.config.max_value_bytes
        ):
            raise ValueError("value must be bytes within configured limit")

    def _ensure_healthy(self) -> None:
        if self.closed:
            raise StoreError("engine closed")
        if self.fail is not None:
            raise StoreError("background flush failed; writes halted") from self.fail

    def _rotate_locked(self, index: int, shard: Shard) -> None:
        if not shard.active:
            return
        generation = shard.active
        shard.active = {}
        shard.active_bytes = 0
        shard.pending.append(generation)
        self.jobs.put((index, generation))

    def _write(self, key: bytes, value: Optional[bytes]) -> int:
        self._validate(key, value)
        with self.gate:
            self._ensure_healthy()
            idx = self.shard_index(key)
            shard = self.shards[idx]
            with shard.lock:
                if len(shard.pending) >= self.config.pending_limit:
                    raise Backpressure("too many immutable memtable generations")
                seq = self.wal.append(key, value)
                old = shard.active.get(key)
                shard.active[key] = Entry(seq, value)
                shard.active_bytes += self._entry_size(key, value) - (
                    self._entry_size(key, old.value) if old is not None else 0
                )
                if shard.active_bytes >= self.config.spill_bytes:
                    self._rotate_locked(idx, shard)
                return seq

    def set(self, key: bytes, value: bytes) -> int:
        if value is None:
            raise ValueError("SET value cannot be None")
        return self._write(key, value)

    def delete(self, key: bytes) -> int:
        return self._write(key, None)

    def get(self, key: bytes) -> Optional[bytes]:
        self._validate(key)
        shard = self.shards[self.shard_index(key)]
        with shard.lock:
            entry = shard.active.get(key)
            if entry is not None:
                return entry.value
            for generation in reversed(shard.pending):
                entry = generation.get(key)
                if entry is not None:
                    return entry.value
            for segment in reversed(shard.segments):
                entry = segment.get(key, self.max_frame)
                if entry is not None:
                    return entry.value
        return None

    def _flush_worker(self) -> None:
        while True:
            job = self.jobs.get()
            try:
                if job is None:
                    return
                if self.fail is not None:
                    continue
                index, generation = job
                segment = Segment.create(self.dir, index, generation, self.max_frame)
                shard = self.shards[index]
                with shard.lock:
                    with self.manifest_lock:
                        if not shard.pending or shard.pending[0] is not generation:
                            raise StoreError("out-of-order immutable generation")
                        self.manifest["segments"][index].append(segment.path.name)
                        self.manifest["checkpoints"][index] = max(
                            self.manifest["checkpoints"][index],
                            max(v.seq for v in generation.values())
                        )
                        self._save_manifest()
                        shard.segments.append(segment)
                        shard.pending.popleft()
            except BaseException as exc:
                self.fail = exc
            finally:
                self.jobs.task_done()

    def checkpoint(self) -> int:
        """Briefly freeze writers, flush immutable generations, then prune WAL.

        New writes can proceed during disk flush. The checkpoint fence is the
        last LSN seen when ALL shards were rotated under the global gate.
        """
        with self.gate:
            self._ensure_healthy()
            fence = self.wal.seq
            for index, shard in enumerate(self.shards):
                with shard.lock:
                    self._rotate_locked(index, shard)
        self.jobs.join()
        self._ensure_healthy()
        with self.gate:
            # Fence never advances beyond what the durable manifests cover.
            if max(self.manifest["checkpoints"]) < fence:
                raise StoreError("missing checkpoint coverage")
            self.wal.rewrite_after(fence)
        return fence

    def compact(self) -> int:
        """Merge each shard's SSTables, preserving newest value/tombstone."""
        self.checkpoint()
        merged = 0
        for index, shard in enumerate(self.shards):
            with shard.lock:
                if len(shard.segments) < 2:
                    continue
                prior = list(shard.segments)
                combined: dict[bytes, Entry] = {}
                for segment in prior:
                    for key in segment.index:
                        entry = segment.get(key, self.max_frame)
                        assert entry is not None
                        if key not in combined or entry.seq > combined[key].seq:
                            combined[key] = entry
                # Only drop tombstones when ALL previously published segments
                # are included in this compaction. Pending/new writes are newer.
                combined = {k: v for k, v in combined.items() if v.value is not None}
                new_segment = (Segment.create(self.dir, index, combined, self.max_frame)
                               if combined else None)
                with self.manifest_lock:
                    self.manifest["segments"][index] = (
                        [new_segment.path.name] if new_segment else []
                    )
                    self._save_manifest()
                    shard.segments = [new_segment] if new_segment else []
                for old in prior:
                    old.path.unlink(missing_ok=True)
                sync_directory(self.dir)
                merged += 1
        return merged

    def close(self, checkpoint: bool = False) -> None:
        if self.closed:
            return
        try:
            if checkpoint:
                self.checkpoint()
            self.jobs.join()
        finally:
            with self.gate:
                self.closed = True
                self.jobs.put(None)
                self.worker.join()
                self.wal.close()

    def __enter__(self) -> "Engine":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()
