// [OPUS-4.8] sq-vw3ax.11 — the SPLIT homepage hero: text on the left, the LIVE in-browser runner
// on the right. This makes the in-tab runner the hero artifact (Run is now in-fold, ≤3
// interactions to a live result), replacing the old full-REPL "moment" that was duplicated with
// /try. Built on the merged dark-first foundation (.bg-atmos / .bg-grid / .display-1 /
// .text-gradient / .kicker) — NO new hue, NO globals.css edit. The atmospheric ground is mounted
// here, page-scoped (fixed/inset:0/z-index:-1, behind all content).
//
// The old "Run a query now" scroll-CTA is gone: Run itself is in-fold in the runner card, so the
// two hero CTAs are just "Open the full workbench →" (/app) and "GitHub". The four-cell stat strip
// moved to a slim band under the hero (StatBand). No performance/timing claim anywhere here.
//
// [OPUS-4.8] sq-4hiqe — the workbench CTA targets /app (the single workbench; /try was removed).
// /app is a SEPARATE Next.js app overlaid at /app/, so this is a HARD full-page anchor
// (withBasePath + trailing slash), NOT a next/link soft nav (which would fetch /app/index.txt).

import { ArrowRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Github } from "@/components/github-icon";
import { HeroQueryRunnerLazy } from "@/components/home/hero-runner-lazy";
import { withBasePath } from "@/lib/base-path";

const REPO_URL = "https://github.com/sparq-org/sparq";

export function Hero() {
  return (
    <section className="relative -mt-8 pb-2 pt-8 md:-mt-10 md:pt-12">
      {/* Atmospheric ground — mounted page-scoped. Fixed + inert, behind all content. */}
      <div className="bg-atmos" aria-hidden />
      <div className="bg-grid" aria-hidden />

      <div className="grid items-center gap-8 lg:grid-cols-12 lg:gap-10">
        {/* LEFT (~5/12): kicker, gradient headline, one-line sub-lede, exactly two CTAs. */}
        <div className="lg:col-span-5">
          <p className="kicker text-primary">In-browser SPARQL</p>

          <h1 className="display-1 mt-4 max-w-[15ch] font-semibold">
            A full SPARQL engine.{" "}
            <span className="text-gradient">In this tab.</span>
          </h1>

          <p className="mt-5 max-w-[46ch] text-lg leading-relaxed text-muted-foreground">
            A state-of-the-art Rust triplestore and SPARQL 1.1/1.2 engine, compiled to
            WebAssembly — it answers your query right here, on the real engine, with nothing
            sent to a server.
          </p>

          <div className="mt-7 flex flex-wrap gap-3">
            <Button asChild size="lg">
              <a href={withBasePath("/app/")}>
                Open the full workbench
                <ArrowRight className="size-4" aria-hidden />
              </a>
            </Button>
            <Button asChild size="lg" variant="ghost">
              <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
                <Github className="size-4" aria-hidden />
                GitHub
              </a>
            </Button>
          </div>
        </div>

        {/* RIGHT (~7/12): the live in-browser runner (code-split, ssr:false) on the atmos ground. */}
        <div className="lg:col-span-7">
          <HeroQueryRunnerLazy />
        </div>
      </div>
    </section>
  );
}
