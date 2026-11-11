import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HexFormat;
import java.util.Map;

/**
 * Standalone JDK21 transport contract tests. MockTransportPeer is NOT Atto;
 * run actual compiler/Truffle integration separately before claiming Atto proof.
 */
public final class AttoRedisRespBridgeContractTest {
    private static final byte[] CRLF = "\r\n".getBytes(StandardCharsets.US_ASCII);
    private static final HexFormat HEX = HexFormat.of();
    private static int checks;

    private AttoRedisRespBridgeContractTest() { }

    private static void verify(boolean condition, String label) {
        checks++;
        if (!condition) throw new AssertionError(label);
    }

    private static byte[] bytes(String value) { return value.getBytes(StandardCharsets.US_ASCII); }

    private static byte[] resp(byte[]... args) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(bytes("*" + args.length));
        out.write(CRLF);
        for (byte[] arg : args) {
            out.write(bytes("$" + arg.length));
            out.write(CRLF);
            out.write(arg);
            out.write(CRLF);
        }
        return out.toByteArray();
    }

    private static byte[] resp(String... args) throws IOException {
        byte[][] raw = new byte[args.length][];
        for (int i = 0; i < args.length; i++) raw[i] = bytes(args[i]);
        return resp(raw);
    }

    private static byte[] bulk(byte[] data) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(bytes("$" + data.length));
        out.write(CRLF);
        out.write(data);
        out.write(CRLF);
        return out.toByteArray();
    }

    private static void exchange(Socket socket, byte[] command, byte[] expected, String label) throws IOException {
        socket.getOutputStream().write(command);
        socket.getOutputStream().flush();
        byte[] got = socket.getInputStream().readNBytes(expected.length);
        verify(Arrays.equals(got, expected), label + ": expected " + HEX.formatHex(expected)
                + " got " + HEX.formatHex(got));
    }

    private static final class Server implements AutoCloseable {
        private final Process process;
        private final int port;

        Server(String peerMode) throws Exception {
            try (ServerSocket reserved = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
                port = reserved.getLocalPort();
            }
            String java = Path.of(System.getProperty("java.home"), "bin", "java").toString();
            String cp = System.getProperty("java.class.path");
            process = new ProcessBuilder(java, "-cp", cp, "AttoRedisRespBridge", "--port",
                    Integer.toString(port), "--", java, "-cp", cp,
                    "AttoRedisRespBridgeContractTest$MockTransportPeer", peerMode)
                    .redirectError(ProcessBuilder.Redirect.DISCARD).start();
            boolean started = false;
            for (int attempt = 0; attempt < 160; attempt++) {
                if (!process.isAlive()) break;
                try (Socket ignored = connect()) {
                    verify(ignored.isConnected(), "bridge loopback readiness");
                    started = true;
                    break;
                } catch (IOException retry) { Thread.sleep(25); }
            }
            if (!started) { close(); throw new AssertionError("bridge failed to start on loopback"); }
        }

        Socket connect() throws IOException {
            Socket socket = new Socket("127.0.0.1", port);
            socket.setSoTimeout(2500);
            return socket;
        }

        @Override public void close() {
            process.destroyForcibly();
            try { process.waitFor(); }
            catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
        }
    }

    /** Deliberately synthetic backend: tests framing, not the Atto compiler. */
    public static final class MockTransportPeer {
        public static void main(String[] args) throws Exception {
            Map<String, String> values = new HashMap<>();
            BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.US_ASCII));
            PrintWriter out = new PrintWriter(System.out, true, StandardCharsets.US_ASCII);
            out.println("READY");
            for (;;) {
                String opcode = in.readLine();
                if (opcode == null) return;
                String key = in.readLine();
                if (key == null) return;
                if (opcode.equals("SET")) {
                    String value = in.readLine();
                    if (value == null || args[0].equals("die-on-set")) return;
                    values.put(key, value);
                    out.println("OK");
                } else if (opcode.equals("GET")) {
                    if (values.containsKey(key)) { out.println("VALUE"); out.println(values.get(key)); }
                    else out.println("NIL");
                } else if (opcode.equals("DEL")) {
                    out.println(values.remove(key) == null ? "ZERO" : "ONE");
                } else {
                    return;
                }
            }
        }
    }

    private static void liveProtocol() throws Exception {
        try (Server server = new Server("normal"); Socket socket = server.connect()) {
            byte[] rawKey = {0, (byte) 0xff, '\r', '\n'};
            byte[] rawValue = {0, (byte) 0xfe, '\r', '\n', 42};
            exchange(socket, resp("PING"), bytes("+PONG\r\n"), "PING");
            exchange(socket, resp(new byte[][] {bytes("ECHO"), rawValue}), bulk(rawValue), "binary ECHO");
            exchange(socket, resp(new byte[][] {bytes("GET"), rawKey}), bytes("$-1\r\n"), "missing GET");
            exchange(socket, resp(new byte[][] {bytes("SET"), rawKey, rawValue}), bytes("+OK\r\n"), "binary SET");
            exchange(socket, resp(new byte[][] {bytes("GET"), rawKey}), bulk(rawValue), "binary GET");
            exchange(socket, resp("SET", "empty", ""), bytes("+OK\r\n"), "empty SET");
            exchange(socket, resp("GET", "empty"), bulk(new byte[0]), "empty is not nil");
            exchange(socket, resp(new byte[][] {bytes("DEL"), rawKey}), bytes(":1\r\n"), "present DEL");
            exchange(socket, resp(new byte[][] {bytes("DEL"), rawKey}), bytes(":0\r\n"), "absent DEL");
            exchange(socket, resp(new byte[][] {bytes("GET"), rawKey}), bytes("$-1\r\n"), "deleted GET");
            exchange(socket, resp("INCR", "x"), bytes("-ERR unsupported command or argument count\r\n"), "unknown op");
            exchange(socket, resp("PING"), bytes("+PONG\r\n"), "unknown op does not desynchronize");
            exchange(socket, resp("QUIT"), bytes("+OK\r\n"), "QUIT");
            verify(socket.getInputStream().read() < 0, "QUIT disconnect");
        }
    }

    private static void malformedRequest(byte[] attack, String label) throws Exception {
        try (Server server = new Server("normal"); Socket socket = server.connect()) {
            exchange(socket, attack, bytes("-ERR malformed RESP2 request\r\n"), label);
            verify(socket.getInputStream().read() < 0, label + " closes connection");
        }
    }

    private static void backendCrashIsFailClosed() throws Exception {
        try (Server server = new Server("die-on-set"); Socket socket = server.connect()) {
            socket.getOutputStream().write(resp("SET", "key", "value"));
            socket.getOutputStream().flush();
            byte[] reply = socket.getInputStream().readNBytes(30);
            verify(!new String(reply, StandardCharsets.US_ASCII).startsWith("+OK"),
                    "failed guest must not acknowledge SET");
        }
    }

    public static void main(String[] args) throws Exception {
        liveProtocol();
        malformedRequest(bytes("*2\r\n$3\r\nGET\r\n$4097\r\n"), "reject key too long before body read");
        malformedRequest(bytes("*2\r\n$3\r\nGET\r\n$-1\r\n"), "reject null bulk");
        malformedRequest(bytes("*2\r\n$3\r\nGET\r\n$1048577\r\n"), "reject oversized bulk before read");
        malformedRequest(bytes("*1\n$4\r\nPING\r\n"), "reject LF-only header");
        backendCrashIsFailClosed();
        System.out.println("ATTO_REDIS_RESP_TRANSPORT_CONTRACT=PASS checks=" + checks
                + " (mock backend only; native Atto guest NOT_EXECUTED by this test)");
    }
}
