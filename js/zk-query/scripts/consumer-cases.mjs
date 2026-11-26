// Research grade and not externally audited. Issuer-scoped fixtures for the Solid consumer expressions.
const prefixes = `PREFIX vcq: <https://w3id.org/sparq/vcq#>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
`;
export const degreeQuery = `${prefixes}SELECT DISTINCT ?value WHERE {
  ?g vcq:verifiedIssuer <urn:fictional:degree-recognition:university> .
  GRAPH ?g { ?subject <urn:claim:degree.level> ?value . FILTER(?value IN ("BSc", "MSc")) }
}`;
const envelope = { holder: true, status: true, validity: true, freshness: true, openingProfile: 'long-objects' };
export const consumerCases = [
  {
    name: 'consumer-insurance-count',
    query: `${prefixes}SELECT DISTINCT ?result WHERE {
  ?lic vcq:verifiedIssuer <urn:fictional:insurance-quote:licensing> .
  ?reg vcq:verifiedIssuer <urn:fictional:insurance-quote:claims-register> .
  GRAPH ?lic { ?subject <urn:claim:licence.years> ?v0 . } BIND((?v0 >= 5) AS ?test0)
  GRAPH ?lic { ?subject <urn:claim:licence.points> ?v1 . } BIND((?v1 <= 3) AS ?test1)
  GRAPH ?reg { ?subject <urn:claim:claims.five-year-count> ?v2 . } BIND((?v2 = 0) AS ?test2)
  GRAPH ?reg { ?subject <urn:claim:claims.five-year-complete> ?v3 . } BIND((?v3 = true) AS ?test3)
  BIND((?test0 && ?test1 && ?test2 && ?test3) AS ?result)
}`,
    bounds: { credentials: 2, quadsPerCredential: 64, rowsPerTable: 4 }, credentialQuadBounds: [16, 64], envelope,
  },
  {
    name: 'consumer-insurance-absence',
    query: `${prefixes}SELECT DISTINCT ?result WHERE {
  ?lic vcq:verifiedIssuer <urn:fictional:insurance-quote:licensing> .
  ?reg vcq:verifiedIssuer <urn:fictional:insurance-quote:claims-register> .
  GRAPH ?lic { ?subject <urn:claim:licence.years> ?v0 . ?subject <urn:claim:licence.points> ?v1 . }
  BIND((?v0 >= 5 && ?v1 <= 3 &&
        NOT EXISTS { GRAPH ?reg { ?subject <urn:claim:claims.claim> ?c . ?c <urn:claim:claims.date> ?d .
                     FILTER(?d >= "2021-11-13"^^xsd:date) } }) AS ?result)
}`,
    bounds: { credentials: 2, quadsPerCredential: 64, rowsPerTable: 16 }, credentialQuadBounds: [16, 64],
    envelope: { ...envelope, tableRowBounds: [4, 4, 4, 4, 4, 4, 4, 16, 16, 16, 16, 4, 4, 4] },
  },
  {
    name: 'consumer-trial',
    query: `${prefixes}SELECT DISTINCT ?result WHERE {
  ?prac vcq:verifiedIssuer <urn:fictional:trial-eligibility:practice> .
  GRAPH ?prac { ?subject <urn:claim:trial.age> ?v0 . } BIND((?v0 >= 18) AS ?test0)
  GRAPH ?prac { ?subject <urn:claim:trial.age> ?v1 . } BIND((?v1 <= 65) AS ?test1)
  GRAPH ?prac { ?subject <urn:claim:trial.diagnosis> ?v2 . } BIND((?v2 = "condition-a") AS ?test2)
  BIND(NOT EXISTS { GRAPH ?prac { ?subject <urn:claim:trial.medications> ?excluded3 .
                    FILTER(?excluded3 IN ("excluded-x", "excluded-y")) } } AS ?test3)
  GRAPH ?prac { ?subject <urn:claim:trial.medication-complete> ?v4 . } BIND((?v4 = true) AS ?test4)
  BIND((?test0 && ?test1 && ?test2 && ?test3 && ?test4) AS ?result)
}`,
    bounds: { credentials: 1, quadsPerCredential: 64, rowsPerTable: 32 }, credentialQuadBounds: [64],
    envelope: { ...envelope, tableRowBounds: [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 32, 32, 2, 2, 2, 2, 2, 2, 2] },
  },
  {
    name: 'consumer-recycled-min',
    query: `${prefixes}SELECT DISTINCT ((MIN(?value) >= 0.3) AS ?result) WHERE {
  ?subject <urn:claim:material.recycled-fraction> ?value .
}`,
    bounds: { credentials: 4, quadsPerCredential: 16, rowsPerTable: 4 }, credentialQuadBounds: [16, 16, 16, 16],
    envelope: { status: true, validity: true, freshness: true },
  },
  {
    name: 'consumer-degree', query: degreeQuery,
    bounds: { credentials: 1, quadsPerCredential: 16, rowsPerTable: 4 },
  },
  {
    name: 'consumer-degree-hidden', query: degreeQuery.replace('DISTINCT ?value', 'DISTINCT ?subject'),
    bounds: { credentials: 1, quadsPerCredential: 16, rowsPerTable: 4 },
  },
];
