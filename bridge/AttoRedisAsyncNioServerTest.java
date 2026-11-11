import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

/** Pure JDK21 async NIO transport tests. The backend is a MOCK, not Atto. */
public final class AttoRedisAsyncNioServerTest {
    private static int checks;
    private static final byte[] EMPTY = new byte[0];

    private static void ok(boolean good, String message) {
        if (!good) throw new AssertionError(message);
        checks++;
    }
    private static byte[] ascii(String s) { return s.getBytes(StandardCharsets.US_ASCII); }
    private static byte[] frame(byte[]... args) throws IOException {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        buf.write(ascii("*" + args.length + "\r\n"));
        for (byte[] a : args) {
            buf.write(ascii("$" + a.length + "\r\n"));
            buf.write(a);
            buf.write(ascii("\r\n"));
        }
        return buf.toByteArray();
    }
    private static byte[] concat(byte[]... args) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] a : args) out.write(a);
        return out.toByteArray();
    }
    private static byte[] read(Socket socket, int n) throws IOException {
        byte[] value = socket.getInputStream().readNBytes(n);
        if (value.length != n) throw new IOException("incomplete response " + value.length + "/" + n);
        return value;
    }
    private static Socket connect(AttoRedisAsyncNioServer server) throws IOException {
        Socket s = new Socket();
        s.connect(server.localAddress());
        s.setSoTimeout(3000);
        return s;
    }
    private static final class MockActor implements AttoRedisAsyncNioServer.AsyncActorBackend {
        final Map<String, byte[]> state = new ConcurrentHashMap<>();
        final AtomicInteger submissions = new AtomicInteger();
        final HexFormat hex = HexFormat.of();
        @Override public CompletionStage<byte[]> request(String op, byte[] key, byte[] value) {
            submissions.incrementAndGet();
            // The AsyncActorBackend contract yields a future, never a synchronous
            // blocking result. This deterministic mock is NOT a real Atto guest.
            return CompletableFuture.supplyAsync(() -> {
                String id = hex.formatHex(key);
                return switch (op) {
                    case "SET" -> {
                        state.put(id, value.clone());
                        yield ascii("+OK\r\n");
                    }
                    case "GET" -> {
                        byte[] found = state.get(id);
                        if (found == null) yield ascii("$-1\r\n");
                        try {
                            yield concat(ascii("$" + found.length + "\r\n"),
                                    found, ascii("\r\n"));
                        } catch (IOException error) { throw new IllegalStateException(error); }
                    }
                    case "DEL" -> ascii(state.remove(id) == null ? ":0\r\n" : ":1\r\n");
                    default -> throw new IllegalArgumentException(op);
                };
            });
        }
    }

    private static void binaryAndPipeline() throws Exception {
        MockActor actor = new MockActor();
        try (AttoRedisAsyncNioServer server = AttoRedisAsyncNioServer.bind(
                new InetSocketAddress(InetAddress.getLoopbackAddress(), 0), actor);
             Socket client = connect(server)) {
            byte[] key = {(byte) 0xff, 0, 10, 13};
            byte[] value = {(byte) 0xfd, 0, 13, 10};
            byte[] input = concat(
                    frame(ascii("PING")),
                    frame(ascii("GET"), key),
                    frame(ascii("SET"), key, value),
                    frame(ascii("GET"), key),
                    frame(ascii("DEL"), key),
                    frame(ascii("GET"), key),
                    frame(ascii("SET"), ascii("empty"), EMPTY),
                    frame(ascii("GET"), ascii("empty")),
                    frame(ascii("WHAT")),
                    frame(ascii("QUIT")));
            byte[] expected = concat(
                    ascii("+PONG\r\n"), ascii("$-1\r\n"), ascii("+OK\r\n"),
                    ascii("$4\r\n"), value, ascii("\r\n"),
                    ascii(":1\r\n"), ascii("$-1\r\n"),
                    ascii("+OK\r\n"), ascii("$0\r\n\r\n"),
                    ascii("-ERR unsupported command or argument count\r\n"),
                    ascii("+OK\r\n"));
            client.getOutputStream().write(input);
            byte[] output = read(client, expected.length);
            ok(Arrays.equals(output, expected), "binary pipeline output mismatch");
            ok(actor.submissions.get() == 7, "only SET/GET/DEL must reach actor");
        }
    }

    private static void partialHeadersAndFairness() throws Exception {
        MockActor actor = new MockActor();
        try (AttoRedisAsyncNioServer server = AttoRedisAsyncNioServer.bind(
                new InetSocketAddress(InetAddress.getLoopbackAddress(), 0), actor);
             Socket slow = connect(server);
             Socket fast = connect(server)) {
            slow.getOutputStream().write(ascii("*2\r\n$3\r\nGET\r\n$"));
            fast.getOutputStream().write(frame(ascii("PING")));
            ok(Arrays.equals(read(fast, 7), ascii("+PONG\r\n")),
                    "slow fragmented client blocked unrelated NIO client");
            slow.getOutputStream().write(ascii("1\r\na\r\n"));
            ok(Arrays.equals(read(slow, 5), ascii("$-1\r\n")),
                    "fragmented request was not reassembled");
            slow.getOutputStream().write(ascii("*2\r\n$3\r\nGET\r\n$4097\r\n"));
            ok(Arrays.equals(read(slow, ascii("-ERR malformed RESP2 request\r\n").length),
                    ascii("-ERR malformed RESP2 request\r\n")),
                    "oversized key must reject before reading key body");
        }
    }

    private static void asynchronousBackendFailure() throws Exception {
        AttoRedisAsyncNioServer.AsyncActorBackend backend = (op, key, value) ->
                CompletableFuture.failedFuture(new IOException("actor died"));
        try (AttoRedisAsyncNioServer server = AttoRedisAsyncNioServer.bind(
                new InetSocketAddress(InetAddress.getLoopbackAddress(), 0), backend);
             Socket client = connect(server)) {
            client.getOutputStream().write(frame(ascii("GET"), ascii("k")));
            byte[] error = ascii("-ERR Atto backend unavailable\r\n");
            ok(Arrays.equals(read(client, error.length), error),
                    "actor exception must fail closed without stale reply");
        }
    }

    private static void refusesRemoteBind() throws Exception {
        MockActor mock = new MockActor();
        try {
            AttoRedisAsyncNioServer.bind(
                    new InetSocketAddress(InetAddress.getByAddress(new byte[] {0, 0, 0, 0}), 0),
                    mock);
            throw new AssertionError("wildcard remote bind admitted");
        } catch (SecurityException expected) {
            checks++;
        }
    }

    public static void main(String[] args) throws Exception {
        binaryAndPipeline();
        partialHeadersAndFairness();
        asynchronousBackendFailure();
        refusesRemoteBind();
        System.out.println("ATTO_REDIS_ASYNC_NIO_CONTRACT=PASS checks=" + checks
                + " backend=MOCK real_Atto_guest=NOT_EXECUTED");
    }
}
