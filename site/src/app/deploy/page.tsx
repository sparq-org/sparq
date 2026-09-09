import type { Metadata } from "next";
import {
  Boxes,
  Cloud,
  ExternalLink,
  HeartPulse,
  KeyRound,
  Layers3,
  LockKeyhole,
  Rocket,
  Server,
  ShieldCheck,
  ShipWheel,
  TriangleAlert,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card";

import { CopyCommand } from "./copy-command";
import {
  DEMO_ENVIRONMENT,
  DEPLOY_OPTIONS,
  OPEN_BY_DEFAULT_CAVEAT,
  SECURE_DEFAULTS,
  type DeployOption,
} from "./deploy-options";

// [GPT-5.6] sq-44ga1 — static /deploy route over the provider-owned templates.
export const metadata: Metadata = {
  title: "Deploy",
  description:
    "Deploy sparq-server or the Solid/LWS server with reviewed cloud, PaaS, Terraform, and Helm templates that keep auth and HTTPS wiring visible.",
};

const PROVIDER_ICONS = {
  aws: Cloud,
  azure: Layers3,
  gcp: Cloud,
  fly: Rocket,
  render: Server,
  railway: ShipWheel,
  terraform: Boxes,
  helm: ShipWheel,
} satisfies Record<DeployOption["id"], typeof Cloud>;

const SECURE_DEFAULT_ICONS = {
  "Auth on": KeyRound,
  "HTTPS only": LockKeyhole,
  "Secrets stay secret": ShieldCheck,
  "Server-specific health": HeartPulse,
} satisfies Record<(typeof SECURE_DEFAULTS)[number]["rule"], typeof KeyRound>;

export default function DeployPage() {
  return (
    <div className="space-y-14">
      <section aria-labelledby="deploy-heading" className="space-y-6 py-4">
        <Badge variant="warning">Native server deployments</Badge>
        <div className="max-w-3xl space-y-4">
          <h1
            id="deploy-heading"
            className="text-4xl font-semibold tracking-tight md:text-5xl"
          >
            Deploy sparq without hiding the security wiring.
          </h1>
          <p className="text-lg leading-relaxed text-muted-foreground">
            Pick a managed cloud, a fast PaaS, or portable infrastructure as
            code. Every link below points at the provider-owned template in this
            repository; the site does not generate or rewrite deployment assets.
          </p>
        </div>

        <div
          className="flex max-w-4xl gap-3 rounded-xl border border-warning/40 bg-warning/10 p-4"
          role="note"
          aria-label="sparq-server authentication caveat"
        >
          <TriangleAlert
            className="mt-0.5 size-5 shrink-0 text-warning"
            aria-hidden
          />
          <div className="space-y-1">
            <p className="font-semibold">Keep the token wiring.</p>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {OPEN_BY_DEFAULT_CAVEAT} Anyone who can reach an ungated image can
              read and write its dataset.
            </p>
          </div>
        </div>
      </section>

      <section aria-labelledby="providers-heading" className="space-y-6">
        <div className="space-y-2">
          <h2 id="providers-heading" className="text-2xl font-semibold">
            Choose a deployment path
          </h2>
          <p className="max-w-3xl text-muted-foreground">
            Buttons are shown only where the checked-in provider asset supports a
            real one-click flow. The Solid/LWS templates require a trusted OIDC
            issuer and public HTTPS base URL; confirm the selected container tag
            is published before launch.
          </p>
        </div>

        <div className="grid gap-5 lg:grid-cols-2">
          {DEPLOY_OPTIONS.map((option) => {
            const Icon = PROVIDER_ICONS[option.id];
            return (
              <Card
                key={option.id}
                id={`provider-${option.id}`}
                data-provider={option.id}
                className="transition-shadow hover:shadow-elevation-2"
              >
                <CardHeader>
                  <div className="flex items-start justify-between gap-4">
                    <div className="flex items-center gap-3">
                      <span className="rounded-lg bg-primary/10 p-2 text-primary">
                        <Icon className="size-5" aria-hidden />
                      </span>
                      <h3 className="font-semibold leading-none tracking-tight">
                        {option.name}
                      </h3>
                    </div>
                    <Badge variant={option.buttons.length ? "success" : "muted"}>
                      {option.mode}
                    </Badge>
                  </div>
                  <CardDescription>{option.summary}</CardDescription>
                </CardHeader>

                <CardContent className="flex flex-1 flex-col gap-4">
                  <div className="flex gap-2 text-sm leading-relaxed text-muted-foreground">
                    <ShieldCheck
                      className="mt-0.5 size-4 shrink-0 text-primary"
                      aria-hidden
                    />
                    <p>{option.security}</p>
                  </div>

                  {option.buttons.length > 0 && (
                    <div className="flex flex-wrap gap-2">
                      {option.buttons.map((button) => (
                        <Button key={button.href} size="sm" asChild>
                          <a
                            href={button.href}
                            target="_blank"
                            rel="noopener noreferrer"
                            data-deploy-target={button.target}
                          >
                            {button.label}
                            <ExternalLink aria-hidden />
                          </a>
                        </Button>
                      ))}
                    </div>
                  )}

                  {option.caveat && (
                    <p className="rounded-lg bg-muted/60 p-3 text-xs leading-relaxed text-muted-foreground">
                      {option.caveat}
                    </p>
                  )}

                  {option.command && option.commandLabel && (
                    <CopyCommand
                      command={option.command}
                      label={option.commandLabel}
                    />
                  )}

                  <Button variant="link" className="mt-auto w-fit px-0" asChild>
                    <a
                      href={option.docsHref}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      Read the reviewed {option.name} guide
                      <ExternalLink aria-hidden />
                    </a>
                  </Button>
                </CardContent>
              </Card>
            );
          })}
        </div>
      </section>

      <section aria-labelledby="demo-heading" className="space-y-6">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-3">
            <h2 id="demo-heading" className="text-2xl font-semibold">
              Ephemeral demo environment
            </h2>
            <Badge variant="warning">Throwaway, not production</Badge>
          </div>
          <p className="max-w-3xl text-muted-foreground">
            The demo templates stand up a deliberately different Cloud Run
            posture: two scale-to-zero services — the experimental Solid/LWS
            server plus a bundled Community Solid Server acting as a throwaway
            identity provider — that keep everything in memory, so an idle
            scale-down wipes the environment. This page links no sparq-hosted
            instance; the manifests deploy one into your own project.
          </p>
        </div>

        <div
          className="flex max-w-4xl gap-3 rounded-xl border border-warning/40 bg-warning/10 p-4"
          role="note"
          aria-label="Demo environment caveats"
        >
          <TriangleAlert
            className="mt-0.5 size-5 shrink-0 text-warning"
            aria-hidden
          />
          <div className="space-y-3">
            {DEMO_ENVIRONMENT.caveats.map((caveat) => (
              <div key={caveat.rule} className="space-y-1">
                <p className="font-semibold">{caveat.rule}</p>
                <p className="text-sm leading-relaxed text-muted-foreground">
                  {caveat.detail}
                </p>
              </div>
            ))}
            <p className="text-sm leading-relaxed text-muted-foreground">
              Sign in with throwaway identities and upload throwaway data only.
            </p>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-4">
          <Button variant="outline" asChild>
            <a
              href={DEMO_ENVIRONMENT.manifestsHref}
              target="_blank"
              rel="noopener noreferrer"
            >
              Demo manifests and deploy steps
              <ExternalLink aria-hidden />
            </a>
          </Button>
          <Button variant="link" className="w-fit px-0" asChild>
            <a
              href={DEMO_ENVIRONMENT.designHref}
              target="_blank"
              rel="noopener noreferrer"
            >
              Read the demo design record
              <ExternalLink aria-hidden />
            </a>
          </Button>
        </div>
      </section>

      <section aria-labelledby="defaults-heading" className="space-y-6">
        <div className="space-y-2">
          <h2 id="defaults-heading" className="text-2xl font-semibold">
            Secure defaults to preserve
          </h2>
          <p className="max-w-3xl text-muted-foreground">
            These are template-layer controls, not claims that the bare server
            images provide a production perimeter by themselves.
          </p>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          {SECURE_DEFAULTS.map((item) => {
            const Icon = SECURE_DEFAULT_ICONS[item.rule];
            return (
              <div
                key={item.rule}
                className="flex gap-3 rounded-xl border bg-card p-4 shadow-elevation-1"
              >
                <Icon className="mt-0.5 size-5 shrink-0 text-primary" aria-hidden />
                <div className="space-y-1">
                  <h3 className="font-semibold">{item.rule}</h3>
                  <p className="text-sm leading-relaxed text-muted-foreground">
                    {item.detail}
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      </section>

      <section
        aria-labelledby="operations-heading"
        className="grid gap-5 border-t pt-8 md:grid-cols-2"
      >
        <div className="space-y-3">
          <h2 id="operations-heading" className="text-xl font-semibold">
            Verify the deployment
          </h2>
          <p className="text-sm leading-relaxed text-muted-foreground">
            The deployment CI contract records which templates can be checked
            without cloud credentials and where a live boot, health probe, one
            request, and anonymous-write rejection are feasible.
          </p>
          <Button variant="outline" asChild>
            <a
              href="https://github.com/sparq-org/sparq/blob/main/docs/deploy-ci.md"
              target="_blank"
              rel="noopener noreferrer"
            >
              Deployment CI contract
              <ExternalLink aria-hidden />
            </a>
          </Button>
        </div>

        <div className="space-y-3 rounded-xl bg-muted/50 p-5">
          <h2 className="text-xl font-semibold">Local Solid development</h2>
          <p className="text-sm leading-relaxed text-muted-foreground">
            Need a loopback-only development pod instead of a cloud container?
            The npm host is in-memory local-development software, defaults to a
            fixed owner, and must not be exposed as a production server.
          </p>
          <CopyCommand
            label="Local-only npm host"
            command={[
              "npx @sparq-org/solid-server --port 3000 \\",
              "  --base-url http://127.0.0.1:3000 \\",
              "  --owner-webid https://id.example/alice#me",
            ].join("\n")}
          />
        </div>
      </section>
    </div>
  );
}
