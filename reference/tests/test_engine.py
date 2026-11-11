"""WAL recovery, checkpoint, spilling, tombstones, concurrency, corruption."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import json
import tempfile
import unittest
from atto_redis_reference.engine import Config, CorruptData, Engine


class EngineTests(unittest.TestCase):
    def test_basic_and_recover_wal(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=4)
            e = Engine(d, cfg)
            self.assertIsNone(e.get(b"k"))
            self.assertEqual(e.set(b"k", b"\x00\xff\r\n"), 1)
            self.assertEqual(e.get(b"k"), b"\x00\xff\r\n")
            self.assertEqual(e.delete_with_count(b"k"), 1)
            self.assertEqual(e.delete_with_count(b"k"), 0)
            e.set(b"live", b"value")
            e.close()
            r = Engine(d, cfg)
            self.assertIsNone(r.get(b"k"))
            self.assertEqual(r.get(b"live"), b"value")
            self.assertEqual(r.set(b"next", b"456"), 5)
            r.close()

    def test_spill_checkpoint_restart(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=8, spill_bytes=140, pending_limit=128)
            e = Engine(d, cfg)
            for n in range(120):
                e.set(f"k{n}".encode(), bytes([n]) * 80)
            self.assertEqual(e.checkpoint(), 120)
            self.assertEqual((Path(d) / "WAL.log").stat().st_size, 0)
            for n in range(120):
                self.assertEqual(e.get(f"k{n}".encode()), bytes([n]) * 80)
            e.close()
            r = Engine(d, cfg)
            for n in range(120):
                self.assertEqual(r.get(f"k{n}".encode()), bytes([n]) * 80)
            self.assertEqual(r.set(b"later", b"ok"), 121)
            r.close()

    def test_compact_tombstones_and_reopen(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=1, spill_bytes=1, pending_limit=12)
            e = Engine(d, cfg)
            e.set(b"a", b"one")
            e.set(b"b", b"two")
            e.set(b"a", b"updated")
            e.delete(b"b")
            e.checkpoint()
            self.assertGreaterEqual(len(e.shards[0].segments), 3)
            self.assertEqual(e.compact(), 1)
            self.assertEqual(len(e.shards[0].segments), 1)
            self.assertEqual(e.get(b"a"), b"updated")
            self.assertIsNone(e.get(b"b"))
            e.close()
            r = Engine(d, cfg)
            self.assertEqual(r.get(b"a"), b"updated")
            self.assertIsNone(r.get(b"b"))
            r.close()

    def test_compaction_to_empty_preserves_lsn(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=1, spill_bytes=1)
            e = Engine(d, cfg)
            e.set(b"gone", b"one")
            e.delete(b"gone")
            e.checkpoint()
            e.compact()
            self.assertEqual(e.shards[0].segments, [])
            e.close()
            r = Engine(d, cfg)
            self.assertIsNone(r.get(b"gone"))
            self.assertEqual(r.set(b"fresh", b"v"), 3)
            r.close()

    def test_truncated_wal_tail_recoverable(self):
        with tempfile.TemporaryDirectory() as d:
            e = Engine(d, Config(shards=2))
            e.set(b"key", b"value")
            e.close()
            with (Path(d) / "WAL.log").open("ab") as fh:
                fh.write(b"AW")
            r = Engine(d, Config(shards=2))
            self.assertEqual(r.get(b"key"), b"value")
            r.close()

    def test_full_corrupted_wal_frame_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            e = Engine(d, Config(shards=2))
            e.set(b"key", b"value")
            e.close()
            p = Path(d) / "WAL.log"
            raw = bytearray(p.read_bytes())
            raw[-1] ^= 0x01
            p.write_bytes(raw)
            with self.assertRaises(CorruptData):
                Engine(d, Config(shards=2))

    def test_missing_manifest_segment_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=1, spill_bytes=1)
            e = Engine(d, cfg)
            e.set(b"a", b"b")
            e.checkpoint()
            e.close()
            m = json.loads((Path(d) / "MANIFEST.json").read_text())
            (Path(d) / m["segments"][0][0]).unlink()
            with self.assertRaises(FileNotFoundError):
                Engine(d, cfg)

    def test_parallel_writers_and_recovery(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = Config(shards=16, spill_bytes=512, pending_limit=128)
            e = Engine(d, cfg)
            def worker(n):
                for k in range(40):
                    key = f"client-{n}-{k}".encode()
                    e.set(key, b"hello" * 20)
                    self.assertEqual(e.get(key), b"hello" * 20)
            with ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(worker, range(8)))
            e.checkpoint()
            e.close()
            r = Engine(d, cfg)
            for n in range(8):
                for k in range(40):
                    self.assertEqual(r.get(f"client-{n}-{k}".encode()), b"hello" * 20)
            r.close()

    def test_forced_process_crash_recovers_fsynced_writes(self):
        import os
        import subprocess
        import sys
        with tempfile.TemporaryDirectory() as d:
            code = (
                "import os,sys; from atto_redis_reference.engine import Engine;"
                "db=Engine(sys.argv[1]); db.set(b'crash',b'durable'); os._exit(0)"
            )
            env = dict(os.environ)
            env["PYTHONPATH"] = str(Path(__file__).resolve().parents[1])
            subprocess.run([sys.executable, "-c", code, d],
                           check=True, env=env, timeout=15)
            r = Engine(d)
            self.assertEqual(r.get(b"crash"), b"durable")
            r.close()

    def test_process_exclusive_wal_lock(self):
        with tempfile.TemporaryDirectory() as d:
            e = Engine(d, Config(shards=2))
            from atto_redis_reference.engine import StoreError
            with self.assertRaises(StoreError):
                Engine(d, Config(shards=2))
            e.close()
            r = Engine(d, Config(shards=2))
            r.close()

    def test_limits(self):
        with tempfile.TemporaryDirectory() as d:
            e = Engine(d, Config(shards=2, max_key_bytes=3, max_value_bytes=3))
            for key, val in ((b"", b"v"), (b"long", b"v"), (b"a", b"long")):
                with self.assertRaises(ValueError):
                    e.set(key, val)
            e.close()


if __name__ == "__main__":
    unittest.main()
