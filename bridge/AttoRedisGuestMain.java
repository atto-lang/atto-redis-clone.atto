import dev.oreslang.OresLanguage;
import dev.oreslang.compiler.OresCompiler;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Source;

/**
 * Narrow trusted CLI adapter: explicitly connect System.in to the polyglot
 * guest. LinkedProgramRunner currently does not forward CLI stdin by default.
 * This class does not process commands or store Redis values on the host.
 *
 * It is not an untrusted-source launcher; pass only the reviewed local .atto
 * fixture and deny host reflection, host file/network operations and interop.
 */
public final class AttoRedisGuestMain {
    private AttoRedisGuestMain() { }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("one Atto fixture path required");
        Path path = Path.of(args[0]).toAbsolutePath().normalize();
        if (!path.toString().endsWith(".atto")
                || !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)
                || Files.isSymbolicLink(path)) {
            throw new SecurityException("regular non-symlink .atto source required");
        }
        byte[] bytes = Files.readAllBytes(path);
        if (bytes.length < 100 || bytes.length > 8192) {
            throw new SecurityException("Atto guest source byte cap violated");
        }
        String code = new String(bytes, StandardCharsets.UTF_8);
        OresCompiler.parseAndTypeCheck(code);
        try (Context context = Context.newBuilder(OresLanguage.ID)
                .allowAllAccess(false)
                .in(System.in)
                .out(System.out)
                .err(System.err)
                .build()) {
            context.eval(Source.newBuilder(OresLanguage.ID, code,
                    path.getFileName().toString())
                    .mimeType(OresLanguage.MIME_TYPE).build());
        }
    }
}
