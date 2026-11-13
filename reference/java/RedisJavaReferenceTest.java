import java.io.ByteArrayOutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/** Java-only independent RESP2 reference test; no Atto guest is executed. */
public final class RedisJavaReferenceTest {
    private static int checks;
    private static byte[] ascii(String text) { return text.getBytes(StandardCharsets.US_ASCII); }
    private static void ok(boolean condition, String name) {
        if (!condition) throw new AssertionError(name);
        checks++;
    }
    private static byte[] frame(byte[]... args) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.writeBytes(ascii("*" + args.length + "\r\n"));
        for (byte[] part : args) {
            out.writeBytes(ascii("$" + part.length + "\r\n"));
            out.writeBytes(part);
            out.writeBytes(ascii("\r\n"));
        }
        return out.toByteArray();
    }
    private static byte[] concat(byte[]... parts) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] part : parts) out.writeBytes(part);
        return out.toByteArray();
    }
    public static void main(String[] args) throws Exception {
        RedisJavaReference.MemoryBackend backend = new RedisJavaReference.MemoryBackend();
        byte[] key = {(byte) 0xff, 0, 13, 10};
        byte[] value = {(byte) 0xfe, 0, 13, 10};
        try (AttoRedisAsyncNioServer server = AttoRedisAsyncNioServer.bind(
                new InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0), backend);
             Socket client = new Socket()) {
            client.connect(server.localAddress());
            client.setSoTimeout(3000);
            byte[] request = concat(
                    frame(ascii("GET"), key),
                    frame(ascii("SET"), key, value),
                    frame(ascii("GET"), key),
                    frame(ascii("DEL"), key),
                    frame(ascii("GET"), key),
                    frame(ascii("SET"), ascii("empty"), new byte[0]),
                    frame(ascii("GET"), ascii("empty")),
                    frame(ascii("PING")),
                    frame(ascii("QUIT")));
            byte[] expected = concat(ascii("$-1\r\n"), ascii("+OK\r\n"),
                    ascii("$4\r\n"), value, ascii("\r\n"), ascii(":1\r\n"),
                    ascii("$-1\r\n"), ascii("+OK\r\n"), ascii("$0\r\n\r\n"),
                    ascii("+PONG\r\n"), ascii("+OK\r\n"));
            client.getOutputStream().write(request);
            byte[] actual = client.getInputStream().readNBytes(expected.length);
            ok(Arrays.equals(actual, expected), "binary pipeline matches RESP2 subset");
            ok(backend.size() == 1, "deletion and empty-value retained");
            ok(backend.retainedBytes() == 5, "byte budget counts key and empty value");
        }
        byte[] owned = {1, 2};
        backend.request("SET", key, owned).toCompletableFuture().join();
        owned[0] = 99;
        byte[] response = backend.request("GET", key, null).toCompletableFuture().join();
        ok(response[4] == 1, "caller mutation must not change owner");
        ok(backend.retainedBytes() == 11, "binary key and value charge");
        backend.request("DEL", key, null).toCompletableFuture().join();
        ok(backend.retainedBytes() == 5, "delete returns storage credit");
        System.out.println("JAVA_REDIS_REFERENCE_TEST=PASS checks=" + checks
                + " durability=NONE atto_guest=NOT_EXECUTED");
    }
}
