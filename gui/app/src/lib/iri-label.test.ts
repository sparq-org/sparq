// Unit tests for the shared IRI display-shortening helper (#6058).
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { abbreviateIri } from "./iri-label.js";

test("abbreviateIri: a common prefix wins", () => {
  assert.equal(abbreviateIri("http://xmlns.com/foaf/0.1/name"), "foaf:name");
  assert.equal(abbreviateIri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"), "rdf:type");
});

test("abbreviateIri: falls back to the fragment or last path segment", () => {
  assert.equal(abbreviateIri("http://data.test/people#alice"), "alice");
  assert.equal(abbreviateIri("http://data.test/people/bob"), "bob");
});

test("abbreviateIri: a trailing separator or no separator keeps the IRI whole", () => {
  assert.equal(abbreviateIri("http://data.test/people/"), "http://data.test/people/");
  assert.equal(abbreviateIri("urn:isbn:123"), "urn:isbn:123");
});
