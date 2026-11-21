# Atto Redis: multi-client actor handoff plan and acceptance gates

Status: **design and first NIO transport proof; not a completed concurrent Redis server**.

## Evidence already executed

- Exact-source real Atto compiler/Truffle, 33/33 tests passing:
  https://github.com/voxletra/voxletra-e2e/actions/runs/43148815864
- Native immutable `Bytes` async NIO2 read and write both execute.
- An Atto guest accepts **two simultaneously open sockets** from the same
  listener, exchanges independent binary payloads, and closes both.
- The current Redis guest still accepts **only one client**, and its
  `BinaryKvActor` output stream remains a single consumer. **Do not run
  multiple handlers independently calling worker.outputs.next(): replies
  could be consumed by the wrong connection.**

## Target topology

1. One Atto accept loop creates a bounded connection owner for each accepted
   socket, without moving an `NioConnection` across actor/task boundaries
   unless compiler ownership and Send capability explicitly authorize it.
2. Each connection owner independently maintains the incremental RESP2
   parser, ordered pipelined command queue, and ordered outbound queue.
3. The single Atto `BinaryKvActor` retains exclusive ownership of the
   `Map<Bytes,Bytes>`, and processes one mutation at a time.
4. A command message carries a monotonically assigned bounded request ID
   and an explicit reply destination/actor capability, **not** a shared
   `worker.outputs.next()` stream in every connection.
5. A single reply router (or a checked typed reply actor reference) ensures
   the reply reaches exactly the originating connection, with per-connection
   command sequencing and bounded inflight accounting. Reuse of request IDs
   across connection generations must never route stale responses.
6. Network reads and writes remain NIO2 CompletionHandler futures.
   Backpressure waits suspend cooperatively and never park the scheduler
   carrier. A full mailbox must fail nonblocking admission or produce a
   cancellable Future rather than a blocking call.
7. The listener and connection owners observe supervisor CONTROL priority,
   typed cancellation, and graceful shutdown. Client disconnect cancels
   outstanding requests and retires the socket, without deleting or moving
   any actor-owned key/value entries.

## Compiler contracts to prove before exposure

- NioConnection is an affine owner. A socket may transfer to a source task
  only through an explicit checked move; it is not implicitly Send/Share.
- No borrow of live socket or mutable parser storage crosses task await
  unless pinned-frame and cancellation rules prove lifetime safety.
- Actor mail carrying a reply route is a nominally typed capability, not an
  unverified `Channel` alias or ambient shared host queue.
- No Java host-side RESP parser, KV hashmap, sync socket or reply mux.
- A connection cannot observe or mutate another connection's parser state.

## Executed acceptance gate for the next implementation

- Run real compiled Atto against **2, 16, and 100 simultaneous clients**
  using binary GET/SET/DEL and pipelined commands.
- Interleave client A's and B's requests and validate every exact reply
  across different binary payloads. Deliberately race identical request IDs
  in different connection epochs; never cross-wire a response.
- Saturate bounded reply queues, pause one reader, cancel a client during
  a pending write, shut down while accept is outstanding, and verify no
  lost/duplicate replies, memory growth, or scheduler-carrier blocking.
- Keep a single KV actor for these correctness tests; sharding is a separate
  performance experiment after routing is correct.
- Require exact source SHA pinning, no skipped tests, executable Atto guest
  evidence, and optimizing GraalVM compatibility for performance runs.
  Correctness tests alone are not a throughput benchmark.

## Remaining byte allocation work

`read_owned_bytes_async` avoids boxing bytes on ingress and
`write_owned_bytes_async` uses a read-only ByteBuffer over frozen Bytes
without payload copying at the NIO writer. The RESP2 state machine currently
materializes `List<int>` for each bulk argument and the response encoder
builds another `List<int>`. Direct bounded Bytes slices / builders and
checked ownership transfers are the next data-representation optimization.
