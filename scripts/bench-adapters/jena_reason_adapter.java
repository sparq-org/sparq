// sq-hmd7l.7 — in-process Apache Jena MATERIALIZATION driver for the
// same-box reasoning comparison harness (scripts/bench/materialize-same-box.sh).
//
// 🤖 SPARQ agent. Mirrors the shape of scripts/bench/JenaShaclBench.java: load the
// (ABox + TBox) graph ONCE (timed, advisory), then time the FORWARD MATERIALIZATION
// (compute the full deductive closure) best-of-N. The materialized-triple count is
// the correctness oracle: it is cross-checked against `sparq-cli reason ... owl/rdfs`
// (bench/lubm expected closure) BEFORE any timing is trusted.
//
// PROFILE FIDELITY — READ THIS (recorded per-column in the envelope, never absorbed):
//   Jena has NO full W3C OWL 2 RL/RDF rule reasoner. Its built-in rule reasoners are:
//     * RDFS       : Jena's RDFS rule set (rdfs2/3/5/7/9/11 + a few). Comparable to
//                    sparq `reason ... rdfs` MODULO Jena's axiomatic/entailment choices.
//     * OWL_MICRO  : a small, fast OWL-subset (subClassOf/subPropertyOf/domain/range/
//                    intersectionOf/some restriction handling) — NOT full OWL 2 RL.
//     * OWL_MINI   : OWL_MICRO + someValuesFrom + a bit more; still NOT full OWL 2 RL
//                    (drops the hardest complete-cardinality/oneOf rules).
//     * OWL (full) : the maximal Jena OWL rule reasoner (slow, still incomplete vs OWL 2 RL).
//   sparq `reason ... owl` implements the FULL OWL 2 RL/RDF rule table (cls-*/cax-*/scm-*/
//   prp-* incl. prp-trp/prp-inv/cls-svf/cls-int — the exact rules LUBM Q6/Q9/Q11/Q12/Q13
//   depend on). So Jena OWL_MICRO/OWL_MINI closures will DIFFER in size from sparq's
//   OWL-RL closure; this is a documented PROFILE difference, not a bug. The envelope
//   records the Jena profile used per row so the count delta is attributable, not hidden.
//
// We count the DISTINCT triples of the materialized (inferred + asserted) model. Jena's
// InfModel.size() counts the deductive closure; we materialize into a plain in-memory
// model so the count is the concrete closure size (comparable to `wc -l` of sparq's
// closure NT, which is de-duplicated).
//
// BUILD + RUN (the harness does this; JENA_HOME = an apache-jena distribution):
//   javac -cp "$JENA_HOME/lib/*" -d <outdir> scripts/bench-adapters/jena_reason_adapter.java
//   java  -cp "$JENA_HOME/lib/*:<outdir>" JenaReasonAdapter <data.nt> <profile> <iters>
//     <profile> in {rdfs, owl-micro, owl-mini, owl}
//
// OUTPUT (stdout) — ONE 3-column TSV line:
//   <profile>\t<closure_triples|ERROR>\t<materialize_best_us|reason>
// stderr carries: jena=<version>, load_us=<..>, base_triples=<..>.
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import org.apache.jena.rdf.model.InfModel;
import org.apache.jena.rdf.model.Model;
import org.apache.jena.rdf.model.ModelFactory;
import org.apache.jena.reasoner.Reasoner;
import org.apache.jena.reasoner.ReasonerRegistry;
import org.apache.jena.riot.RDFDataMgr;
import org.apache.jena.riot.Lang;

// NOTE: package-private (not `public`) so the class name (JenaReasonAdapter) may
// legally live in the bead-spec'd file name jena_reason_adapter.java — javac only
// enforces filename==classname for PUBLIC top-level classes. The harness compiles
// with `javac jena_reason_adapter.java` and runs `java JenaReasonAdapter`.
final class JenaReasonAdapter {
    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            System.err.println("usage: JenaReasonAdapter <data.nt> <rdfs|owl-micro|owl-mini|owl> <iters>");
            System.exit(2);
        }
        String dataPath = args[0];
        String profile = args[1];
        int iters = Math.max(1, Integer.parseInt(args[2]));

        System.err.println("jena=" + org.apache.jena.Jena.VERSION);

        // ---- load the combined (ABox + TBox) graph once (timed: ADVISORY load) ----
        long t0 = System.nanoTime();
        Model base = ModelFactory.createDefaultModel();
        try (InputStream in = Files.newInputStream(Paths.get(dataPath))) {
            RDFDataMgr.read(base, in, Lang.NTRIPLES);
        }
        double loadUs = (System.nanoTime() - t0) / 1e3;
        System.err.println("load_us=" + String.format("%.1f", loadUs));
        System.err.println("base_triples=" + base.size());

        // Select the Jena reasoner for the requested profile. These are Jena's
        // OWN rule reasoners; the profile note in the harness records the
        // fidelity gap vs sparq's full OWL 2 RL.
        double bestUs = Double.POSITIVE_INFINITY;
        long closureTriples = -1;
        for (int i = 0; i < iters; i++) {
            Reasoner reasoner;
            switch (profile) {
                case "rdfs":
                    reasoner = ReasonerRegistry.getRDFSReasoner();
                    break;
                case "owl-micro":
                    reasoner = ReasonerRegistry.getOWLMicroReasoner();
                    break;
                case "owl-mini":
                    reasoner = ReasonerRegistry.getOWLMiniReasoner();
                    break;
                case "owl":
                    reasoner = ReasonerRegistry.getOWLReasoner();
                    break;
                default:
                    System.out.printf("%s\tERROR\tunknown-profile%n", profile);
                    System.exit(2);
                    return;
            }
            long t = System.nanoTime();
            InfModel inf = ModelFactory.createInfModel(reasoner, base);
            // Materialize: force the full closure by draining the InfModel into a
            // concrete model (InfModel is lazy; size() over the deductions model
            // realises the closure). We copy so the count is the concrete de-dup'd size.
            Model materialized = ModelFactory.createDefaultModel();
            materialized.add(inf);
            long triples = materialized.size();
            bestUs = Math.min(bestUs, (System.nanoTime() - t) / 1e3);
            closureTriples = triples;
        }
        System.out.printf("%s\t%d\t%.1f%n", profile, closureTriples, bestUs);
    }
}
