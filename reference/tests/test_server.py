"""RESP2 TCP end-to-end validation without external dependencies."""
import asyncio
import tempfile
import unittest
from atto_redis_reference.engine import Engine, Config
from atto_redis_reference.server import serve_client


def encode(*args: bytes) -> bytes:
    return b"*" + str(len(args)).encode() + b"\r\n" + b"".join(
        b"$" + str(len(a)).encode() + b"\r\n" + a + b"\r\n" for a in args
    )


class RespTests(unittest.TestCase):
    def test_resp_roundtrip(self):
        async def run():
            with tempfile.TemporaryDirectory() as d:
                db = Engine(d, Config(shards=4))
                server = await asyncio.start_server(
                    lambda r, w: serve_client(r, w, db), "127.0.0.1", 0
                )
                try:
                    port = server.sockets[0].getsockname()[1]
                    r, w = await asyncio.open_connection("127.0.0.1", port)
                    w.write(encode(b"PING") + encode(b"SET", b"k", b"hi\x00") +
                            encode(b"GET", b"k") + encode(b"DEL", b"k") +
                            encode(b"GET", b"k") + encode(b"QUIT"))
                    await w.drain()
                    responses = [await r.readline() for _ in range(7)]
                    self.assertEqual(responses, [
                        b"+PONG\r\n", b"+OK\r\n", b"$3\r\n",
                        b"hi\x00\r\n", b":1\r\n", b"$-1\r\n", b"+OK\r\n"
                    ])
                    w.close()
                    await w.wait_closed()
                finally:
                    server.close()
                    await server.wait_closed()
                    db.close()
        asyncio.run(run())

    def test_invalid_framing(self):
        async def run():
            with tempfile.TemporaryDirectory() as d:
                db = Engine(d, Config(shards=1))
                server = await asyncio.start_server(
                    lambda r, w: serve_client(r, w, db), "127.0.0.1", 0
                )
                try:
                    port = server.sockets[0].getsockname()[1]
                    r, w = await asyncio.open_connection("127.0.0.1", port)
                    w.write(b"*1\r\n$4\r\nPINGxx")
                    await w.drain()
                    self.assertEqual(await r.readline(), b"-ERR invalid RESP request\r\n")
                    w.close()
                    await w.wait_closed()
                finally:
                    server.close()
                    await server.wait_closed()
                    db.close()
        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
