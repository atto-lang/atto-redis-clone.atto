import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.CountDownLatch;

/**
 * Standalone, independent Java RESP2 reference with an in-memory KV engine.
 * The NIO2 transport is reused, but this backend NEVER calls an Atto guest.
 *
 * Limitations: volatile data, SET/GET/DEL/PING/ECHO/QUIT only. No snapshots,
 * persistence or replication. Do NOT compare it with the durable Rust engine
 * as though durability and fsync settings were equal.
 */
public final class RedisJavaReference {
    private static final int MAX_ENTRIES = 65_536;
    private static final long MAX_RETAINED_BYTES = 32L * 1024 * 1024;
    private static byte[] ascii(String s) { return s.getBytes(StandardCharsets.US_ASCII); }

    private record Key(byte[] value) {
        Key {
            value = value.clone();
        }
        @Override public boolean equals(Object other) {
            return other instanceof Key key && Arrays.equals(value, key.value);
        }
        @Override public int hashCode() { return Arrays.hashCode(value); }
    }

    /** Synchronized linearizable operations and bounded owned byte storage. */
    public static final class MemoryBackend implements AttoRedisAsyncNioServer.AsyncActorBackend {
        private final Map<Key, byte[]> data = new HashMap<>();
        private long retained;

        @Override public synchronized CompletionStage<byte[]> request(
                String opcode, byte[] key, byte[] value) {
            Key id = new Key(key);
            byte[] reply;
            switch (opcode) {
                case "SET" -> {
                    if (value == null) throw new IllegalArgumentException("value");
                    byte[] previous = data.get(id);
                    long next = retained - (previous == null ? 0 : previous.length)
                            + value.length + (previous == null ? key.length : 0);
                    if (next > MAX_RETAINED_BYTES
                            || previous == null && data.size() >= MAX_ENTRIES) {
                        reply = ascii("-ERR reference capacity exceeded\r\n");
                    } else {
                        data.put(id, value.clone());
                        retained = next;
                        reply = ascii("+OK\r\n");
                    }
                }
                case "GET" -> {
                    byte[] found = data.get(id);
                    if (found == null) reply = ascii("$-1\r\n");
                    else {
                        byte[] header = ascii("$" + found.length + "\r\n");
                        reply = Arrays.copyOf(header, header.length + found.length + 2);
                        System.arraycopy(found, 0, reply, header.length, found.length);
                        reply[reply.length - 2] = '\r';
                        reply[reply.length - 1] = '\n';
                    }
                }
                case "DEL" -> {
                    byte[] previous = data.remove(id);
                    if (previous != null) retained -= key.length + previous.length;
                    reply = ascii(previous == null ? ":0\r\n" : ":1\r\n");
                }
                default -> throw new IllegalArgumentException("unsupported opcode");
            }
            return CompletableFuture.completedFuture(reply);
        }
        public synchronized int size() { return data.size(); }
        public synchronized long retainedBytes() { return retained; }
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1 || !args[0].matches("[0-9]{1,5}")) {
            throw new IllegalArgumentException("usage: RedisJavaReference PORT (loopback only)");
        }
        int port = Integer.parseInt(args[0]);
        try (AttoRedisAsyncNioServer server = AttoRedisAsyncNioServer.bind(
                new InetSocketAddress(InetAddress.getByName("127.0.0.1"), port),
                new MemoryBackend())) {
            Runtime.getRuntime().addShutdownHook(new Thread(server::close, "java-redis-reference-shutdown"));
            System.out.println("JAVA_REDIS_REFERENCE=VOLATILE_NIO2 ADDR=" + server.localAddress()
                    + " ATTO_GUEST=NOT_EXECUTED");
            new CountDownLatch(1).await();
        }
    }
}
