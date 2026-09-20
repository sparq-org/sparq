import { Card, CardContent, CardHeader } from "@/components/ui/card";

// [GPT-6] Registry versions are independent of the desktop release metadata.
const PACKAGES = [
  {
    title: "Rust · crates.io",
    description: "Install the CLI or add the engine libraries to a Rust project.",
    command: "cargo install sparq-cli\ncargo add sparq-core sparq-engine",
    links: [
      ["sparq-cli on crates.io", "https://crates.io/crates/sparq-cli"],
      ["Browse the sparq crate family", "https://crates.io/search?q=sparq-"],
    ],
  },
  {
    title: "Python · PyPI",
    description: "Install sparq-rdf; use import sparq in Python.",
    command: "python -m pip install sparq-rdf",
    links: [["sparq-rdf on PyPI", "https://pypi.org/project/sparq-rdf/"]],
  },
  {
    title: "JavaScript · npm",
    description: "The WebAssembly engine with an RDF/JS API.",
    command: "npm install @sparq-org/sparq",
    links: [["@sparq-org/sparq on npm", "https://www.npmjs.com/package/@sparq-org/sparq"]],
  },
  {
    title: "Solid development server · npm",
    description: "A local, in-memory development host; not a production server.",
    command: "npm install @sparq-org/solid-server",
    links: [["@sparq-org/solid-server on npm", "https://www.npmjs.com/package/@sparq-org/solid-server"]],
  },
  {
    title: "EYE compatibility · npm",
    description: "The eyereasoner-compatible JavaScript adapter.",
    command: "npm install @sparq-org/eyereasoner-compat",
    links: [["@sparq-org/eyereasoner-compat on npm", "https://www.npmjs.com/package/@sparq-org/eyereasoner-compat"]],
  },
];

export function PackageInstalls() {
  return (
    <section aria-labelledby="package-installs" className="space-y-4">
      <h2 id="package-installs" className="text-xl font-semibold">
        Package managers
      </h2>
      <p className="measure text-sm text-muted-foreground">
        Choose the package for your environment. Each registry lists its available
        versions and requirements; package availability is separate from the desktop
        downloads below.
      </p>
      <div className="grid gap-4 md:grid-cols-2">
        {PACKAGES.map(({ title, description, command, links }) => (
          <Card key={title} className="min-w-0">
            <CardHeader>
              <h3 className="font-semibold">{title}</h3>
              <p className="text-sm text-muted-foreground">{description}</p>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              <pre
                tabIndex={0}
                aria-label={`${title} install commands`}
                className="overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs"
              >
                <code>{command}</code>
              </pre>
              <ul className="space-y-1">
                {links.map(([label, href]) => (
                  <li key={href}>
                    <a
                      href={href}
                      className="break-words text-primary underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-offset-4"
                    >
                      {label}
                    </a>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}
