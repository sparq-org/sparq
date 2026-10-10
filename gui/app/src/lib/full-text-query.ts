// The SPARQL query the Full-text tool sends to the W-text bundle (#6719). The search term goes
// through the shared sparq-client literal writer, so `"`, `\`, CR, LF and other control
// characters are escaped; a raw line break would otherwise end the string and fail to parse.
import { termToNTriples } from "@sparq/client";

/** The BM25 search query for `term`, matched with `text:matches` and ranked by `text:score`. */
export function buildSearchQuery(term: string): string {
  const literal = termToNTriples({ type: "literal", value: term });
  return `PREFIX text: <http://sparq.dev/text#>
SELECT ?s ?prop ?lit ?score WHERE {
  ?s ?prop ?lit .
  ?lit text:matches ${literal} ; text:score ?score .
} ORDER BY DESC(?score) LIMIT 50`;
}
