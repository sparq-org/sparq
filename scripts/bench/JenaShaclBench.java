// sq-7d3dj.33 — in-process Apache Jena SHACL driver for the same-box
// SHACL comparison harness (scripts/bench/shacl-same-box.sh).
//
// WHY IN-PROCESS (not `bin/shacl validate` via report_cli_adapter.py): the CLI's
// wall clock bundles JVM start-up + data parse + validate + report
// serialisation, which is NOT comparable to sparq's validate-only `validate_us`
// (bench_shacl loads once, times validate best-of-N). This driver mirrors
// bench_shacl's methodology: parse the data graph ONCE (timed, advisory), parse
// the shapes once, then time `ShaclValidator.get().validate` best-of-N.
//
// BUILD + RUN (the harness does this; JENA_HOME = an apache-jena distribution):
//   javac -cp "$JENA_HOME/lib/*" -d <outdir> scripts/bench/JenaShaclBench.java
//   java  -cp "$JENA_HOME/lib/*:<outdir>" JenaShaclBench <data> <shapes.ttl> <iters>
//
// OUTPUT (stdout) — ONE bench_shacl-style 6-column TSV line for the given
// shapes file (`focus_nodes` = `na`; Jena does not expose a target-selection
// count):
//   <workload>\t<violations>\t<validate_us>\t<conforms 0/1>\t<na>\t<load_us>
//
// The harness invokes this once per (data, shapes) pair under `timeout`, so a
// hung workload degrades to an honest ERROR row in the envelope; JVM start-up
// stays OUTSIDE the timed section either way.
//
// stderr carries a one-line meta block: jena=<version>.
import org.apache.jena.graph.Graph;
import org.apache.jena.riot.RDFDataMgr;
import org.apache.jena.shacl.ShaclValidator;
import org.apache.jena.shacl.Shapes;
import org.apache.jena.shacl.ValidationReport;

public final class JenaShaclBench {
    public static void main(String[] args) {
        if (args.length < 3) {
            System.err.println("usage: JenaShaclBench <data> <shapes.ttl> <iters>");
            System.exit(2);
        }
        String dataPath = args[0];
        String shapesPath = args[1];
        int iters = Math.max(1, Integer.parseInt(args[2]));

        System.err.println("jena=" + org.apache.jena.Jena.VERSION);

        // ---- load the data graph once (timed: the ADVISORY load metric) ----
        long t0 = System.nanoTime();
        Graph data = RDFDataMgr.loadGraph(dataPath);
        double loadUs = (System.nanoTime() - t0) / 1e3;

        // Shapes parsed once, mirroring sparq's validate_with_model
        // (amortised shapes parse) and the pySHACL driver.
        Graph shapesGraph = RDFDataMgr.loadGraph(shapesPath);
        Shapes shapes = Shapes.parse(shapesGraph);

        String name = shapesPath.replaceAll(".*/", "").replaceAll("\\.ttl$", "");
        double bestUs = Double.POSITIVE_INFINITY;
        long violations = 0;
        boolean conforms = true;
        for (int i = 0; i < iters; i++) {
            long t = System.nanoTime();
            ValidationReport report = ShaclValidator.get().validate(shapes, data);
            bestUs = Math.min(bestUs, (System.nanoTime() - t) / 1e3);
            violations = report.getEntries().size();
            conforms = report.conforms();
        }
        System.out.printf(
            "%s\t%d\t%.1f\t%d\tna\t%.1f%n",
            name, violations, bestUs, conforms ? 1 : 0, loadUs);
    }
}
