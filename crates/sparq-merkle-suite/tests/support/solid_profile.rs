pub fn configuration() -> String {
    String::from("<urn:solid:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n<urn:solid:proof> <https://w3id.org/security#verificationMethod> <urn:solid:key> .\n<urn:solid:proof> <http://purl.org/dc/terms/created> \"2026-11-13T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n")
}

pub fn document() -> String {
    let mut doc = String::new();
    for i in 0..32 {
        let value = match i % 4 {
            0 => format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                (1u64 << 48) - i
            ),
            1 => format!(
                "\"0.{:02}\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                30 + i
            ),
            2 => String::from("\"2026-11-13\"^^<http://www.w3.org/2001/XMLSchema#date>"),
            _ => format!("\"medicine-{i}\""),
        };
        doc.push_str(&format!(
            "<urn:solid:subject> <urn:solid:fact:{i}> {value} .\n"
        ));
    }
    for triple in [
        "<urn:solid:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential>",
        "<urn:solid:credential> <https://www.w3.org/2018/credentials#credentialSubject> <urn:solid:subject>",
        "<urn:solid:credential> <https://www.w3.org/2018/credentials#issuer> <urn:solid:issuer>",
        "<urn:solid:credential> <https://www.w3.org/2018/credentials#validFrom> \"2026-11-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
        "<urn:solid:credential> <https://www.w3.org/2018/credentials#validUntil> \"2026-12-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
        "<urn:solid:credential> <https://www.w3.org/2018/credentials#credentialStatus> _:status",
        "_:status <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/ns/credentials/status#BitstringStatusListEntry>",
        "_:status <https://www.w3.org/ns/credentials/status#statusPurpose> \"revocation\"",
        "_:status <https://www.w3.org/ns/credentials/status#statusListIndex> \"0007\"",
        "_:status <https://www.w3.org/ns/credentials/status#statusListCredential> <urn:solid:status:list>",
    ] {
        doc.push_str(triple);
        doc.push_str(" .\n");
    }
    doc
}
