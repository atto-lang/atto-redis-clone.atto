"""Minimal Redis RESP2 TCP service: PING, ECHO, SET, GET, DEL, QUIT.

Executable Python reference oracle, NOT the AttoLang backend. Loopback by default.
"""
from __future__ import annotations
import argparse
import asyncio
from contextlib import suppress
from .engine import Config, Engine, StoreError

MAX_ARGS = 16
MAX_HEADER = 128


class ProtocolError(Exception):
    pass


async def read_command(reader: asyncio.StreamReader, max_bulk: int):
    first = await reader.readline()
    if not first:
        return None
    if len(first) > MAX_HEADER or not first.endswith(b"\r\n") or not first.startswith(b"*"):
        raise ProtocolError("expected RESP array")
    try:
        count = int(first[1:-2])
    except ValueError as exc:
        raise ProtocolError("bad array count") from exc
    if not 1 <= count <= MAX_ARGS:
        raise ProtocolError("invalid array count")
    args = []
    for _ in range(count):
        header = await reader.readline()
        if (len(header) > MAX_HEADER or not header.endswith(b"\r\n")
                or not header.startswith(b"$")):
            raise ProtocolError("expected bulk string")
        try:
            size = int(header[1:-2])
        except ValueError as exc:
            raise ProtocolError("bad bulk length") from exc
        if not 0 <= size <= max_bulk:
            raise ProtocolError("bulk exceeds configured limit")
        data = await reader.readexactly(size + 2)
        if data[-2:] != b"\r\n":
            raise ProtocolError("invalid bulk terminator")
        args.append(data[:-2])
    return args


def bulk(value: bytes | None) -> bytes:
    if value is None:
        return b"$-1\r\n"
    return b"$" + str(len(value)).encode() + b"\r\n" + value + b"\r\n"


async def serve_client(reader: asyncio.StreamReader,
                       writer: asyncio.StreamWriter, engine: Engine) -> None:
    try:
        while True:
            try:
                args = await read_command(
                    reader, max(engine.config.max_key_bytes, engine.config.max_value_bytes)
                )
                if args is None:
                    return
                op = args[0].upper()
                if op == b"PING" and len(args) in (1, 2):
                    response = b"+PONG\r\n" if len(args) == 1 else bulk(args[1])
                elif op == b"ECHO" and len(args) == 2:
                    response = bulk(args[1])
                elif op == b"SET" and len(args) == 3:
                    await asyncio.to_thread(engine.set, args[1], args[2])
                    response = b"+OK\r\n"
                elif op == b"GET" and len(args) == 2:
                    result = await asyncio.to_thread(engine.get, args[1])
                    response = bulk(result)
                elif op == b"DEL" and len(args) == 2:
                    count = await asyncio.to_thread(engine.delete_with_count, args[1])
                    response = b":" + str(count).encode() + b"\r\n"
                elif op == b"QUIT" and len(args) == 1:
                    writer.write(b"+OK\r\n")
                    await writer.drain()
                    return
                else:
                    response = b"-ERR unsupported command or argument count\r\n"
            except (StoreError, ValueError):
                response = b"-ERR storage operation failed\r\n"
            except (ProtocolError, asyncio.IncompleteReadError, asyncio.LimitOverrunError):
                writer.write(b"-ERR invalid RESP request\r\n")
                await writer.drain()
                return
            writer.write(response)
            await writer.drain()
    finally:
        writer.close()
        with suppress(ConnectionError, OSError):
            await writer.wait_closed()


async def main_async(host: str, port: int, data: str, config: Config) -> None:
    engine = Engine(data, config)
    try:
        server = await asyncio.start_server(
            lambda r, w: serve_client(r, w, engine), host=host, port=port
        )
        async with server:
            await server.serve_forever()
    finally:
        engine.close(checkpoint=False)


def main() -> None:
    parser = argparse.ArgumentParser(description="AttoLang Redis clone reference server")
    parser.add_argument("--data-dir", default="./var/atto-redis")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=6379)
    parser.add_argument("--shards", type=int, default=64)
    parser.add_argument("--spill-mib", type=int, default=64)
    parser.add_argument("--fsync", choices=("always", "everysec"), default="always")
    args = parser.parse_args()
    config = Config(shards=args.shards, spill_bytes=args.spill_mib * 1024 * 1024,
                    fsync_policy=args.fsync)
    try:
        asyncio.run(main_async(args.host, args.port, args.data_dir, config))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
