import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.List;

/**
 * Executes the actual Java bridge RESP2 parser without a surrogate KV store.
 *
 * This tests transport admission/bounds only. It does NOT execute the Atto
 * compiler/guest; the real Atto loopback and actor tests are independent gates.
 */
public final class AttoRedisRespBridgeBoundsTest {
    private static final Method FRAME;

    static {
        try {
            FRAME = AttoRedisRespBridge.class.getDeclaredMethod("frame", InputStream.class);
            FRAME.setAccessible(true);
        } catch (ReflectiveOperationException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    private AttoRedisRespBridgeBoundsTest() { }

    @SuppressWarnings("unchecked")
    private static List<byte[]> next(InputStream input) throws Exception {
        try {
            return (List<byte[]>) FRAME.invoke(null, input);
        } catch (InvocationTargetException e) {
            Throwable cause = e.getCause();
            if (cause instanceof IOException io) throw io;
            if (cause instanceof RuntimeException runtime) throw runtime;
            throw e;
        }
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }

    private static void reject(String name, byte[] data) throws Exception {
        try {
            next(new ByteArrayInputStream(data));
            throw new AssertionError("Parser accepted malformed request: " + name);
        } catch (IOException expected) {
            // Bridge must reject by a bounded IOException before dispatch.
        }
    }

    private static byte[] ascii(String s) {
        return s.getBytes(StandardCharsets.US_ASCII);
    }

    private static byte[] bulk(byte[] payload) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(ascii("$" + payload.length + "\r\n"));
        out.write(payload);
        out.write(ascii("\r\n"));
        return out.toByteArray();
    }

    private static byte[] request(byte[]... args) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(ascii("*" + args.length + "\r\n"));
        for (byte[] arg : args) out.write(bulk(arg));
        return out.toByteArray();
    }

    public static void main(String[] args) throws Exception {
        int cases = 0;
        check(next(new ByteArrayInputStream(new byte[0])) == null, "EOF should be non-request");
        cases++;

        byte[] binaryKey = new byte[] {0, (byte) 0xff, 10, 13};
        byte[] binaryValue = new byte[] {(byte) 0x80, 0, 1, 2, (byte) 0xfe};
        byte[] valid = request(ascii("SET"), binaryKey, binaryValue);
        List<byte[]> parts = next(new ByteArrayInputStream(valid));
        check(parts.size() == 3 && Arrays.equals(parts.get(1), binaryKey)
                && Arrays.equals(parts.get(2), binaryValue), "binary bulk changed bytes");
        cases++;

        ByteArrayInputStream pipelined = new ByteArrayInputStream(
                concat(request(ascii("PING")), request(ascii("GET"), binaryKey)));
        check(Arrays.equals(next(pipelined).get(0), ascii("PING")), "first pipeline");
        check(Arrays.equals(next(pipelined).get(1), binaryKey), "second pipeline");
        check(next(pipelined) == null, "pipeline EOF");
        cases++;

        reject("empty array", ascii("*0\r\n")); cases++;
        reject("array too large", ascii("*4\r\n")); cases++;
        reject("non-array", ascii("+PING\r\n")); cases++;
        reject("partial header", ascii("*1\r")); cases++;
        reject("header no CR", ascii("*1\n")); cases++;
        reject("negative length", ascii("*-1\r\n")); cases++;
        reject("decimal overflow", ascii("*2147483648\r\n")); cases++;
        reject("empty bulk command", ascii("*1\r\n$0\r\n\r\n")); cases++;
        reject("oversize command", ascii("*1\r\n$17\r\n")); cases++;
        reject("oversize bulk", ascii("*2\r\n$4\r\nECHO\r\n$1048577\r\n")); cases++;
        reject("body truncated", ascii("*1\r\n$4\r\nPI")); cases++;
        reject("wrong bulk trailer", ascii("*1\r\n$4\r\nPING\n\n")); cases++;
        reject("missing bulk header", ascii("*1\r\n$")); cases++;

        byte[] longHeader = new byte[80];
        Arrays.fill(longHeader, (byte) '9');
        reject("excessive header", concat(ascii("*"), longHeader));
        cases++;

        System.out.println("ATTO_REDIS_RESP_BRIDGE_BOUNDS=PASS cases=" + cases
                + " (Java parser only; Atto guest not executed by this test)");
    }

    private static byte[] concat(byte[]... arrays) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] item : arrays) out.write(item);
        return out.toByteArray();
    }
}
