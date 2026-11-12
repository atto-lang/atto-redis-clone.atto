import dev.oreslang.OresLanguage;
import dev.oreslang.compiler.OresCompiler;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Source;

/**
 * Launches the reviewed Atto-owned Redis actor guest on an NIO2 loopback port.
 * The host reads/compiles the source once during bootstrapping ONLY. All
 * request I/O and Redis map operations execute inside the compiled Atto guest:
 * AsynchronousServerSocketChannel/AsynchronousSocketChannel and actor mail.
 * No stdin/stdout bridge, host Java hashmap or host RESP dispatch.
 */
public final class AttoRedisNioGuestMain {
    private AttoRedisNioGuestMain() { }

    public static void main(String[] args) throws Exception {
        if (args.length != 2 && !(args.length == 3 && "--trace-truffle".equals(args[2]))) {
            throw new IllegalArgumentException(
                    "usage: AttoRedisNioGuestMain path/to/redis_actor_nio.atto numeric-loopback-port [--trace-truffle]");
        }
        Path path = Path.of(args[0]).toAbsolutePath().normalize();
        if (!path.toString().endsWith(".atto") || Files.isSymbolicLink(path)
                || !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw new SecurityException("non-symlink .atto guest fixture required");
        }
        int port = Integer.parseInt(args[1]);
        if (port < 1024 || port > 65535) throw new SecurityException("invalid listener port");
        byte[] bytes = Files.readAllBytes(path);
        if (bytes.length < 100 || bytes.length > 8192) {
            throw new SecurityException("Atto guest source exceeds bootstrap bound");
        }
        String source = new String(bytes, StandardCharsets.UTF_8);
        if (!source.contains("__PORT__")) {
            throw new IllegalArgumentException("expected reviewed __PORT__ placeholder");
        }
        source = source.replace("__PORT__", Integer.toString(port));
        OresCompiler.parseAndTypeCheck(source);
        // Explicit experimental diagnostic mode for CI. Not enabled in normal
        // production launches. A successful JVM version check alone does not
        // establish that an Oreslang guest call target has been compiled.
        Context.Builder builder = Context.newBuilder(OresLanguage.ID)
                .allowAllAccess(false)
                .arguments(OresLanguage.ID,
                        new String[] {"--ores-capabilities=NETWORK,ACTOR_SPAWN,SHARED_MEMORY"});
        if (args.length == 3) {
            builder.allowExperimentalOptions(true);
            builder.option("engine.TraceCompilation", "true");
            builder.option("engine.CompilationFailureAction", "Print");
        }
        try (Context context = builder.build()) {
            context.eval(Source.newBuilder(OresLanguage.ID, source, "redis_actor_nio.atto")
                    .mimeType(OresLanguage.MIME_TYPE).build());
        }
    }
}
