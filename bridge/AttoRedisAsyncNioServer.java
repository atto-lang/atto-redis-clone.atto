import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.StandardSocketOptions;
import java.nio.ByteBuffer;
import java.nio.channels.AsynchronousServerSocketChannel;
import java.nio.channels.AsynchronousSocketChannel;
import java.nio.channels.CompletionHandler;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * NIO2 completion-driven RESP2 TCP admission for Atto actors.
 *
 * All socket accept/read/write operations use asynchronous NIO channels and
 * completion handlers. No thread waits on a Future, selector, or socket call.
 * The AsyncActorBackend MUST itself be async; it cannot wrap blocking stdio
 * or a synchronous Java/Rust KV fallback and still claim async-only Atto I/O.
 *
 * This is a transport building block, not a wired Atto-native TCP listener.
 */
public final class AttoRedisAsyncNioServer implements AutoCloseable {
    public interface AsyncActorBackend {
        CompletionStage<byte[]> request(String opcode, byte[] key, byte[] value);
    }

    private static final int MAX_CLIENTS = 32;
    private static final int MAX_PENDING = 32;
    private static final int MAX_ARGS = 3;
    private static final int MAX_KEY = 4096;
    private static final int MAX_VALUE = 1024 * 1024;
    private static final int MAX_HEADER = 64;
    private static final int MAX_FRAME = MAX_VALUE + MAX_KEY + 1024;
    private static final byte[] MALFORMED =
            "-ERR malformed RESP2 request\r\n".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] UNAVAILABLE =
            "-ERR Atto backend unavailable\r\n".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] OVERLOADED =
            "-ERR Atto backend overloaded\r\n".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] UNSUPPORTED =
            "-ERR unsupported command or argument count\r\n".getBytes(StandardCharsets.US_ASCII);

    private final AsyncActorBackend backend;
    private final AsynchronousServerSocketChannel listener;
    private final Set<Client> clients = ConcurrentHashMap.newKeySet();
    private final AtomicInteger pending = new AtomicInteger();
    private final AtomicBoolean stopped = new AtomicBoolean();

    private AttoRedisAsyncNioServer(AsyncActorBackend backend,
            AsynchronousServerSocketChannel listener) {
        this.backend = backend;
        this.listener = listener;
    }

    public static AttoRedisAsyncNioServer bind(
            InetSocketAddress address, AsyncActorBackend backend) throws IOException {
        if (backend == null || address == null
                || address.getAddress() == null
                || !address.getAddress().isLoopbackAddress()) {
            throw new SecurityException("async Atto RESP2 requires a numeric loopback address");
        }
        AsynchronousServerSocketChannel listener = AsynchronousServerSocketChannel.open();
        try {
            listener.bind(address, MAX_CLIENTS);
            AttoRedisAsyncNioServer server = new AttoRedisAsyncNioServer(backend, listener);
            server.accept();
            return server;
        } catch (Throwable failure) {
            listener.close();
            throw failure;
        }
    }

    public InetSocketAddress localAddress() throws IOException {
        return (InetSocketAddress) listener.getLocalAddress();
    }

    private void accept() {
        if (stopped.get()) return;
        listener.accept(null, new CompletionHandler<AsynchronousSocketChannel, Void>() {
            @Override public void completed(AsynchronousSocketChannel socket, Void attachment) {
                // Rearm before dispatch, independently of slow clients or actor operations.
                accept();
                if (stopped.get() || clients.size() >= MAX_CLIENTS) {
                    try { socket.close(); } catch (IOException ignored) { }
                    return;
                }
                try {
                    socket.setOption(StandardSocketOptions.TCP_NODELAY, true);
                    Client client = new Client(socket);
                    clients.add(client);
                    if (clients.size() > MAX_CLIENTS) client.close();
                    else client.read();
                } catch (IOException failure) {
                    try { socket.close(); } catch (IOException ignored) { }
                }
            }
            @Override public void failed(Throwable error, Void attachment) {
                // Listener retirement is terminal; no busy retry of a failed accept.
                if (!stopped.get()) AttoRedisAsyncNioServer.this.close();
            }
        });
    }

    private static final class ClientFault extends IOException {
        ClientFault(String message) { super(message); }
    }

    private record Frame(String op, List<byte[]> args, int end) { }

    /** Return -1 for an incomplete header, or the CR offset for a whole line. */
    private static int lineEnd(ByteBuffer source, int from) throws ClientFault {
        int until = Math.min(source.limit(), from + MAX_HEADER);
        for (int i = from; i < until; i++) {
            if (source.get(i) == '\n') {
                if (i == from || source.get(i - 1) != '\r') {
                    throw new ClientFault("noncanonical line ending");
                }
                return i - 1;
            }
        }
        if (source.limit() - from >= MAX_HEADER) {
            throw new ClientFault("header exceeds limit");
        }
        return -1;
    }

    private static int decimal(ByteBuffer source, int begin, int end, int max)
            throws ClientFault {
        if (begin >= end) throw new ClientFault("missing length");
        int count = 0;
        for (int i = begin; i < end; i++) {
            int n = (source.get(i) & 0xff) - '0';
            if (n < 0 || n > 9 || count > (max - n) / 10) {
                throw new ClientFault("length out of range");
            }
            count = count * 10 + n;
        }
        return count;
    }

    private static Frame parse(ByteBuffer source) throws ClientFault {
        int pos = source.position();
        int start = pos;
        int end = lineEnd(source, pos);
        if (end < 0) return null;
        if (source.get(pos) != '*') throw new ClientFault("expected RESP array");
        int argc = decimal(source, pos + 1, end, MAX_ARGS);
        if (argc == 0) throw new ClientFault("empty array");
        pos = end + 2;
        List<byte[]> args = new ArrayList<>(argc);
        String op = null;
        for (int i = 0; i < argc; i++) {
            end = lineEnd(source, pos);
            if (end < 0) return null;
            if (source.get(pos) != '$') throw new ClientFault("expected RESP bulk");
            int limit = (i == 0) ? 16
                    : (i == 1 && ("SET".equals(op) || "GET".equals(op) || "DEL".equals(op)))
                    ? MAX_KEY : MAX_VALUE;
            int size = decimal(source, pos + 1, end, limit);
            if (i == 0 && size == 0) throw new ClientFault("empty opcode");
            pos = end + 2;
            if ((long) pos - start + size + 2 > MAX_FRAME) {
                throw new ClientFault("frame exceeds bound");
            }
            if (source.limit() - pos < size + 2) return null;
            if (source.get(pos + size) != '\r' || source.get(pos + size + 1) != '\n') {
                throw new ClientFault("invalid bulk trailer");
            }
            byte[] payload = new byte[size];
            ByteBuffer view = source.duplicate();
            view.position(pos);
            view.get(payload);
            args.add(payload);
            if (i == 0) {
                for (byte character : payload) {
                    if (character < 33 || character > 126) {
                        throw new ClientFault("opcode must be visible ASCII");
                    }
                }
                op = new String(payload, StandardCharsets.US_ASCII).toUpperCase(Locale.ROOT);
            }
            pos += size + 2;
        }
        return new Frame(op, args, pos);
    }

    private static byte[] bulk(byte[] payload) {
        byte[] hdr = ("$" + payload.length + "\r\n").getBytes(StandardCharsets.US_ASCII);
        byte[] result = Arrays.copyOf(hdr, hdr.length + payload.length + 2);
        System.arraycopy(payload, 0, result, hdr.length, payload.length);
        result[result.length - 2] = '\r';
        result[result.length - 1] = '\n';
        return result;
    }

    private boolean reserve() {
        for (;;) {
            int n = pending.get();
            if (n >= MAX_PENDING) return false;
            if (pending.compareAndSet(n, n + 1)) return true;
        }
    }

    private final class Client implements AutoCloseable {
        private final AsynchronousSocketChannel channel;
        private final AtomicBoolean closed = new AtomicBoolean();
        private ByteBuffer input = ByteBuffer.allocate(4096);

        Client(AsynchronousSocketChannel channel) { this.channel = channel; }

        void read() {
            if (closed.get()) return;
            if (!input.hasRemaining()) {
                if (input.capacity() >= MAX_FRAME) {
                    send(MALFORMED, true);
                    return;
                }
                int next = Math.min(MAX_FRAME, input.capacity() * 2);
                ByteBuffer expanded = ByteBuffer.allocate(next);
                input.flip();
                expanded.put(input);
                input = expanded;
            }
            channel.read(input, 10, TimeUnit.SECONDS, null, new CompletionHandler<Integer, Void>() {
                @Override public void completed(Integer count, Void ignored) {
                    if (count == null || count < 0) { close(); return; }
                    if (count == 0) { read(); return; }
                    process();
                }
                @Override public void failed(Throwable failure, Void ignored) { close(); }
            });
        }

        void process() {
            if (closed.get()) return;
            input.flip();
            Frame parsed;
            try { parsed = parse(input); }
            catch (ClientFault invalid) {
                input.clear();
                send(MALFORMED, true);
                return;
            }
            if (parsed == null) {
                input.compact();
                read();
                return;
            }
            input.position(parsed.end());
            input.compact();
            dispatch(parsed);
        }

        void dispatch(Frame frame) {
            List<byte[]> args = frame.args();
            String op = frame.op();
            if (op.equals("PING") && args.size() == 1) {
                send("+PONG\r\n".getBytes(StandardCharsets.US_ASCII), false);
            } else if ((op.equals("PING") || op.equals("ECHO")) && args.size() == 2) {
                send(bulk(args.get(1)), false);
            } else if (op.equals("QUIT") && args.size() == 1) {
                send("+OK\r\n".getBytes(StandardCharsets.US_ASCII), true);
            } else if ((op.equals("GET") || op.equals("DEL")) && args.size() == 2
                    || op.equals("SET") && args.size() == 3) {
                if (!reserve()) {
                    send(OVERLOADED, true);
                    return;
                }
                try {
                    CompletionStage<byte[]> stage = backend.request(op, args.get(1),
                            args.size() == 3 ? args.get(2) : null);
                    if (stage == null) throw new IllegalStateException("null async response");
                    stage.whenComplete((response, failure) -> {
                        pending.decrementAndGet();
                        if (closed.get()) return;
                        if (failure != null || response == null || response.length > MAX_FRAME) {
                            send(UNAVAILABLE, true);
                        } else {
                            send(response, false);
                        }
                    });
                } catch (RuntimeException fail) {
                    pending.decrementAndGet();
                    send(UNAVAILABLE, true);
                }
            } else {
                send(UNSUPPORTED, false);
            }
        }

        void send(byte[] bytes, boolean retire) {
            if (closed.get()) return;
            ByteBuffer output = ByteBuffer.wrap(bytes);
            channel.write(output, 10, TimeUnit.SECONDS, null,
                    new CompletionHandler<Integer, Void>() {
                @Override public void completed(Integer count, Void unused) {
                    if (count == null || count < 0) { close(); return; }
                    if (output.hasRemaining()) {
                        channel.write(output, 10, TimeUnit.SECONDS, null, this);
                    } else if (retire) close();
                    else process();
                }
                @Override public void failed(Throwable error, Void unused) { close(); }
            });
        }

        @Override public void close() {
            if (!closed.compareAndSet(false, true)) return;
            clients.remove(this);
            try { channel.close(); } catch (IOException ignored) { }
        }
    }

    @Override public void close() {
        if (!stopped.compareAndSet(false, true)) return;
        try { listener.close(); } catch (IOException ignored) { }
        for (Client client : clients) client.close();
    }
}
