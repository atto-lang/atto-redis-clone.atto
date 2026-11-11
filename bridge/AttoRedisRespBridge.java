import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Future;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Loopback-only RESP2 transport shim. Every SET/GET/DEL is executed by the
 * child AttoLang process; no fallback or host-side KV table exists.
 *
 * The child speaks a bounded, line-based hex wire format, which preserves
 * arbitrary RESP bulk bytes (including NUL, non-UTF8 and empty values).
 * This is a transitional host transport, NOT an Atto-native TCP backend.
 */
public final class AttoRedisRespBridge {
    private static final int MAX_CLIENTS = 32;
    private static final int MAX_ARGS = 3;
    private static final int MAX_KEY = 4096;
    private static final int MAX_VALUE = 1024 * 1024;
    private static final int MAX_HEADER = 64;
    private static final int MAX_FRAME = 2 * MAX_VALUE + MAX_KEY + 1024;
    private static final byte[] ERROR_FRAME = "-ERR malformed RESP2 request\r\n".getBytes(StandardCharsets.US_ASCII);
    private static final HexFormat HEX = HexFormat.of();

    private AttoRedisRespBridge() { }

    private static final class ClientFault extends IOException {
        ClientFault(String message) { super(message); }
    }

    /** One process-scoped owner. Its IPC is strictly serialized and fail-closed. */
    private static final class Guest implements AutoCloseable {
        final Process process;
        final BufferedReader output;
        final BufferedWriter input;
        final ExecutorService ordered;
        final AtomicBoolean closed = new AtomicBoolean();

        Guest(List<String> command) throws Exception {
            if (command.isEmpty()) throw new IllegalArgumentException("Atto guest command required");
            process = new ProcessBuilder(command).start();
            input = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.US_ASCII));
            output = new BufferedReader(new InputStreamReader(process.getInputStream(), StandardCharsets.US_ASCII));
            ordered = java.util.concurrent.Executors.newSingleThreadExecutor(r -> {
                Thread thread = new Thread(r, "atto-guest-ipc");
                thread.setDaemon(true);
                return thread;
            });
            Thread errors = new Thread(() -> {
                try (InputStream stream = process.getErrorStream()) {
                    byte[] buf = new byte[1024];
                    long seen = 0;
                    while (true) {
                        int count = stream.read(buf);
                        if (count < 0) break;
                        seen += count;
                        if (seen <= 16384) {
                            System.err.write(buf, 0, count);
                            System.err.flush();
                        }
                    }
                } catch (IOException ignored) { /* guest exit closes stderr */ }
            }, "atto-guest-stderr");
            errors.setDaemon(true);
            errors.start();
            try {
                Future<String> startup = ordered.submit(() -> line());
                if (!"READY".equals(startup.get(20, TimeUnit.SECONDS))) {
                    throw new IOException("Atto guest did not emit READY; compiler/runtime may have failed");
                }
                if (!process.isAlive()) throw new IOException("Atto guest exited at startup");
            } catch (Exception failure) {
                close();
                throw failure;
            }
        }

        private String line() throws IOException {
            StringBuilder text = new StringBuilder();
            while (text.length() < MAX_FRAME * 2) {
                int ch = output.read();
                if (ch < 0) throw new IOException("Atto guest exited or closed IPC");
                if (ch == '\n') return text.toString();
                if (ch < 32 || ch > 126) throw new IOException("unexpected guest IPC control byte");
                text.append((char) ch);
            }
            throw new IOException("Atto guest IPC response exceeds bound");
        }

        byte[] request(String opcode, byte[] key, byte[] value) throws IOException {
            if (closed.get() || !process.isAlive()) throw new IOException("Atto guest unavailable");
            Future<byte[]> pending = ordered.submit(() -> {
                input.write(opcode); input.write('\n');
                input.write(HEX.formatHex(key)); input.write('\n');
                if (value != null) { input.write(HEX.formatHex(value)); input.write('\n'); }
                input.flush();
                String result = line();
                return switch (result) {
                    case "OK" -> {
                        if (!opcode.equals("SET")) throw new IOException("unexpected guest OK");
                        yield "+OK\r\n".getBytes(StandardCharsets.US_ASCII);
                    }
                    case "NIL" -> {
                        if (!opcode.equals("GET")) throw new IOException("unexpected guest NIL");
                        yield "$-1\r\n".getBytes(StandardCharsets.US_ASCII);
                    }
                    case "VALUE" -> {
                        if (!opcode.equals("GET")) throw new IOException("unexpected guest VALUE");
                        String encoded = line();
                        if (encoded.length() > 2 * MAX_VALUE) throw new IOException("guest value exceeds cap");
                        byte[] bytes;
                        try { bytes = HEX.parseHex(encoded); }
                        catch (IllegalArgumentException failure) { throw new IOException("invalid guest value hex", failure); }
                        yield bulk(bytes);
                    }
                    case "ONE" -> {
                        if (!opcode.equals("DEL")) throw new IOException("unexpected guest ONE");
                        yield ":1\r\n".getBytes(StandardCharsets.US_ASCII);
                    }
                    case "ZERO" -> {
                        if (!opcode.equals("DEL")) throw new IOException("unexpected guest ZERO");
                        yield ":0\r\n".getBytes(StandardCharsets.US_ASCII);
                    }
                    default -> throw new IOException("unrecognized Atto guest IPC reply");
                };
            });
            try {
                return pending.get(15, TimeUnit.SECONDS);
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt(); close();
                throw new IOException("Atto guest request interrupted", interrupted);
            } catch (ExecutionException | TimeoutException failure) {
                close();
                throw new IOException("Atto guest IPC failure (fail closed)", failure);
            }
        }

        @Override public void close() {
            if (!closed.compareAndSet(false, true)) return;
            ordered.shutdownNow();
            process.destroyForcibly();
            try { input.close(); } catch (IOException ignored) { }
            try { output.close(); } catch (IOException ignored) { }
        }
    }

    /** Exact numeric ASCII, no signs, no whitespace, and no integer overflow. */
    private static int decimal(byte[] raw, int offset, int max) throws ClientFault {
        if (raw.length <= offset) throw new ClientFault("empty length");
        int count = 0;
        for (int i = offset; i < raw.length; i++) {
            int digit = raw[i] - '0';
            if (digit < 0 || digit > 9 || count > (max - digit) / 10) {
                throw new ClientFault("invalid or excessive length");
            }
            count = count * 10 + digit;
        }
        return count;
    }

    private static byte[] header(InputStream input) throws IOException {
        byte[] data = new byte[MAX_HEADER];
        for (int i = 0; i < data.length; i++) {
            int next = input.read();
            if (next == -1) {
                if (i == 0) return null;
                throw new ClientFault("partial header");
            }
            if (next == '\n') {
                if (i == 0 || data[i - 1] != '\r') throw new ClientFault("invalid CRLF");
                return Arrays.copyOf(data, i - 1);
            }
            data[i] = (byte) next;
        }
        throw new ClientFault("header exceeds cap");
    }

    private static List<byte[]> frame(InputStream input) throws IOException {
        byte[] header = header(input);
        if (header == null) return null;
        if (header.length < 2 || header[0] != '*') throw new ClientFault("expected RESP array");
        int argc = decimal(header, 1, MAX_ARGS);
        if (argc < 1) throw new ClientFault("empty command");
        int cumulative = 0;
        ArrayList<byte[]> parts = new ArrayList<>(argc);
        for (int i = 0; i < argc; i++) {
            byte[] bulkHeader = header(input);
            if (bulkHeader == null || bulkHeader.length < 2 || bulkHeader[0] != '$') {
                throw new ClientFault("expected bulk string");
            }
            int size = decimal(bulkHeader, 1, i == 0 ? 16 : MAX_VALUE);
            if (i == 0 && size == 0) throw new ClientFault("empty command name");
            cumulative += size;
            if (cumulative > MAX_FRAME) throw new ClientFault("request frame exceeds cap");
            byte[] body = input.readNBytes(size);
            if (body.length != size) throw new ClientFault("truncated bulk string");
            if (input.read() != '\r' || input.read() != '\n') throw new ClientFault("bad bulk trailer");
            parts.add(body);
        }
        return parts;
    }

    private static byte[] bulk(byte[] payload) {
        byte[] prefix = ("$" + payload.length + "\r\n").getBytes(StandardCharsets.US_ASCII);
        byte[] result = new byte[prefix.length + payload.length + 2];
        System.arraycopy(prefix, 0, result, 0, prefix.length);
        System.arraycopy(payload, 0, result, prefix.length, payload.length);
        result[result.length - 2] = '\r';
        result[result.length - 1] = '\n';
        return result;
    }

    private static String command(byte[] command) throws ClientFault {
        for (byte item : command) {
            if (item < 33 || item > 126) throw new ClientFault("non-ASCII command");
        }
        return new String(command, StandardCharsets.US_ASCII).toUpperCase(Locale.ROOT);
    }

    private static void connection(Socket socket, Guest guest) {
        try (socket) {
            socket.setSoTimeout(10000);
            socket.setTcpNoDelay(true);
            InputStream input = socket.getInputStream();
            OutputStream output = socket.getOutputStream();
            for (;;) {
                List<byte[]> args;
                try { args = frame(input); }
                catch (ClientFault bad) { output.write(ERROR_FRAME); return; }
                if (args == null) return;
                String op;
                try { op = command(args.get(0)); }
                catch (ClientFault bad) { output.write(ERROR_FRAME); return; }
                byte[] result;
                if (op.equals("PING") && args.size() == 1) {
                    result = "+PONG\r\n".getBytes(StandardCharsets.US_ASCII);
                } else if ((op.equals("PING") || op.equals("ECHO")) && args.size() == 2) {
                    result = bulk(args.get(1));
                } else if (op.equals("QUIT") && args.size() == 1) {
                    output.write("+OK\r\n".getBytes(StandardCharsets.US_ASCII));
                    return;
                } else if (op.equals("SET") && args.size() == 3 && args.get(1).length <= MAX_KEY) {
                    result = guest.request("SET", args.get(1), args.get(2));
                } else if ((op.equals("GET") || op.equals("DEL")) && args.size() == 2
                        && args.get(1).length <= MAX_KEY) {
                    result = guest.request(op, args.get(1), null);
                } else {
                    result = "-ERR unsupported command or argument count\r\n".getBytes(StandardCharsets.US_ASCII);
                }
                output.write(result);
                output.flush();
            }
        } catch (SocketTimeoutException ignored) {
            // Bound slow clients without affecting other connections.
        } catch (IOException failure) {
            try {
                if (!socket.isClosed()) socket.getOutputStream().write(
                    "-ERR Atto guest unavailable\r\n".getBytes(StandardCharsets.US_ASCII));
            } catch (IOException ignored) { }
        }
    }

    public static void main(String[] args) throws Exception {
        int port = 6380;
        int separator = -1;
        for (int i = 0; i < args.length; i++) if (args[i].equals("--")) { separator = i; break; }
        if (separator < 0 || separator == args.length - 1) {
            throw new IllegalArgumentException("usage: AttoRedisRespBridge [--port N] -- ATTO_GUEST_COMMAND [ARGS...]");
        }
        for (int i = 0; i < separator; i += 2) {
            if (i + 1 >= separator || !args[i].equals("--port")) throw new IllegalArgumentException("only --port N is supported");
            port = Integer.parseInt(args[i + 1]);
        }
        if (port < 1 || port > 65535) throw new IllegalArgumentException("port must be 1..65535");
        List<String> command = Arrays.asList(args).subList(separator + 1, args.length);
        try (Guest guest = new Guest(command); ServerSocket listener = new ServerSocket()) {
            listener.setReuseAddress(false);
            listener.bind(new InetSocketAddress(InetAddress.getByAddress(new byte[] {127,0,0,1}), port), MAX_CLIENTS);
            ThreadPoolExecutor clients = new ThreadPoolExecutor(MAX_CLIENTS, MAX_CLIENTS, 0, TimeUnit.SECONDS,
                new ArrayBlockingQueue<>(MAX_CLIENTS), r -> {
                    Thread t = new Thread(r, "atto-redis-client"); t.setDaemon(true); return t;
                }, new ThreadPoolExecutor.AbortPolicy());
            try {
                System.err.println("Atto RESP bridge ready on 127.0.0.1:" + port + " (ephemeral, no auth, Atto child owns KV)");
                for (;;) {
                    Socket socket = listener.accept();
                    try { clients.execute(() -> connection(socket, guest)); }
                    catch (java.util.concurrent.RejectedExecutionException limit) {
                        try (socket) { socket.getOutputStream().write("-ERR too many clients\r\n".getBytes(StandardCharsets.US_ASCII)); }
                    }
                }
            } finally {
                clients.shutdownNow();
            }
        }
    }
}
