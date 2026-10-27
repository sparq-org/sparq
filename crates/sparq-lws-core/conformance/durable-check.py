#!/usr/bin/env python3
"""Capture live conformance state, then GET it without writing after a restart.

The manifest contains response bytes and validators, never credentials. Solid's test
trees include the surviving suite resources and their ACLs. Touchstone's run.json
names its PUT targets, including resources its per-test cleanup has removed.
"""

import argparse
import base64
import hashlib
import http.client
import json
from pathlib import Path
import ssl
import subprocess
import tempfile
import time
from urllib.parse import urlencode, urljoin, urlsplit
import uuid


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64(data):
    return base64.urlsafe_b64decode(data + "=" * (-len(data) % 4))


class Signer:
    """P-256 fixture signing, using the same local crypto prerequisite as the drivers."""

    def __init__(self, directory, jwk=None):
        self.path = Path(directory) / (uuid.uuid4().hex + ".der")
        if jwk:
            # SEC1 ECPrivateKey: version, private scalar, named P-256 curve.
            scalar = unb64(jwk["d"])
            if jwk["kty"] != "EC" or jwk["crv"] != "P-256" or len(scalar) != 32:
                raise ValueError("expected a private P-256 fixture key")
            self.path.write_bytes(bytes.fromhex("30310201010420") + scalar
                                  + bytes.fromhex("a00a06082a8648ce3d030107"))
        else:
            self.path.write_bytes(subprocess.check_output(
                ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-outform", "DER"],
                stderr=subprocess.DEVNULL))
        self.path.chmod(0o600)
        point = subprocess.check_output(
            ["openssl", "pkey", "-inform", "DER", "-in", str(self.path), "-pubout", "-outform", "DER"],
            stderr=subprocess.DEVNULL)[-65:]
        self.public = {"kty": "EC", "crv": "P-256", "x": b64(point[1:33]), "y": b64(point[33:])}
        self.kid = jwk.get("kid") if jwk else None

    def jwt(self, header, claims):
        encode = lambda value: b64(json.dumps(value, separators=(",", ":")).encode())
        message = (encode({"alg": "ES256", **header}) + "." + encode(claims)).encode()
        signature = subprocess.check_output(
            ["openssl", "dgst", "-sha256", "-keyform", "DER", "-sign", str(self.path)], input=message)
        # ECDSA-Sig-Value's two DER INTEGERs become the fixed-width JWS R || S.
        rlen = signature[3]
        r = int.from_bytes(signature[4:4 + rlen], "big")
        slen = signature[5 + rlen]
        s = int.from_bytes(signature[6 + rlen:6 + rlen + slen], "big")
        return message.decode() + "." + b64(r.to_bytes(32, "big") + s.to_bytes(32, "big"))


def request(method, url, headers=None, body=None):
    parsed = urlsplit(url)
    if parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise ValueError("conformance requests must stay on loopback")
    if parsed.scheme == "https":
        # Conformance uses a generated self-signed certificate on loopback.
        conn = http.client.HTTPSConnection(parsed.hostname, parsed.port, timeout=15,
                                          context=ssl._create_unverified_context())
    elif parsed.scheme == "http":
        conn = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=15)
    else:
        raise ValueError("expected an HTTP conformance URL")
    try:
        conn.request(method, parsed.path + ("?" + parsed.query if parsed.query else ""), body, headers or {})
        response = conn.getresponse()
        return response.status, {k.lower(): v for k, v in response.getheaders()}, response.read()
    finally:
        conn.close()


class Client:
    def __init__(self, args, directory):
        self.args = args
        self.directory = directory
        self.tokens = {}
        self.signers = {}
        self.env = {}
        if args.protocol == "solid":
            for line in Path(args.env_file).read_text().splitlines():
                if line and not line.startswith("#"):
                    key, value = line.split("=", 1)
                    self.env[key] = value
        else:
            self.signers["alice"] = Signer(directory, json.loads(Path(args.key).read_text()))
            status, _, data = request("GET", args.base + "/.well-known/lws-configuration")
            if status != 200:
                raise ValueError("cannot discover the LWS authorization server")
            metadata = json.loads(data)
            self.issuer = metadata["issuer"]
            self.realm = args.base + "/"

    def proof(self, signer, method, url, token=None):
        claims = {"jti": uuid.uuid4().hex, "htm": method, "htu": url, "iat": int(time.time())}
        if token:
            claims["ath"] = b64(hashlib.sha256(token.encode()).digest())
        return signer.jwt({"typ": "dpop+jwt", "jwk": signer.public}, claims)

    def headers(self, identity, method, url):
        if identity == "anonymous":
            return {}
        if self.args.protocol == "lws":
            signer = self.signers["alice"]
            now = int(time.time())
            header = {"typ": "at+jwt"}
            if signer.kid:
                header["kid"] = signer.kid
            token = signer.jwt(header, {"iss": self.issuer, "aud": self.realm, "sub": self.args.owner,
                                        "iat": now, "exp": now + 300, "jti": uuid.uuid4().hex})
            return {"Authorization": "Bearer " + token}
        if identity not in self.tokens:
            signer = self.signers[identity] = Signer(self.directory)
            issuer = self.env["USERS_" + identity.upper() + "_IDP"].rstrip("/")
            status, _, data = request("GET", issuer + "/.well-known/openid-configuration")
            if status != 200:
                raise ValueError("cannot discover the fixture identity provider")
            endpoint = json.loads(data)["token_endpoint"]
            prefix = "USERS_" + identity.upper() + "_"
            credentials = urlencode({"x": self.env[prefix + "CLIENTID"]})[2:] + ":" \
                + urlencode({"x": self.env[prefix + "CLIENTSECRET"]})[2:]
            status, _, data = request("POST", endpoint,
                                     {"Authorization": "Basic " + base64.b64encode(credentials.encode()).decode(),
                                      "DPoP": self.proof(signer, "POST", endpoint),
                                      "Content-Type": "application/x-www-form-urlencoded"},
                                     b"grant_type=client_credentials")
            if status != 200:
                raise ValueError("fixture token exchange failed")
            self.tokens[identity] = json.loads(data)["access_token"]
        token = self.tokens[identity]
        return {"Authorization": "DPoP " + token,
                "DPoP": self.proof(self.signers[identity], method, url, token)}

    def send(self, method, url, identity="alice", headers=None, body=None):
        target, base = urlsplit(url), urlsplit(self.args.base)
        if (target.scheme, target.netloc) != (base.scheme, base.netloc):
            raise ValueError("resource URL escaped the conformance origin")
        return request(method, url, {**self.headers(identity, method, url), **(headers or {})}, body)

    def read(self, url, accept="*/*"):
        identities = ("alice", "bob", "anonymous") if self.args.protocol == "solid" else ("alice",)
        for identity in identities:
            status, headers, body = self.send("GET", url, identity, {"Accept": accept})
            if status not in (401, 403):
                return identity, status, headers, body
        raise ValueError("no fixture identity can read " + url)


def record(client, url, optional=False):
    identity, status, headers, body = client.read(url)
    if optional and status == 404:
        return {"url": url, "status": 404, "identity": identity}
    if status != 200:
        raise ValueError(f"GET {url} returned {status}")
    etag = headers.get("etag")
    if not etag:
        raise ValueError("GET lacks ETag: " + url)
    return {"url": url, "status": status, "identity": identity, "body": b64(body), "etag": etag}


def solid_tree(client, roots):
    pending, seen = list(roots), {}
    while pending:
        url = pending.pop()
        if url in seen:
            continue
        if len(seen) >= 10000:
            raise ValueError("conformance resource walk exceeded its bound")
        seen[url] = record(client, url)
        acl = url + ".acl"
        seen[acl] = record(client, acl, optional=True)
        if url.endswith("/"):
            _, status, _, data = client.read(url, "application/ld+json")
            if status != 200:
                raise ValueError("cannot enumerate conformance container " + url)
            # Native JSON-LD serialization uses expanded predicates and @id objects.
            def children(node):
                if isinstance(node, list):
                    for item in node:
                        yield from children(item)
                elif isinstance(node, dict):
                    for key, value in node.items():
                        if key == "http://www.w3.org/ns/ldp#contains":
                            for child in value if isinstance(value, list) else [value]:
                                yield urljoin(url, child["@id"])
                        else:
                            yield from children(value)
            pending.extend(children(json.loads(data)))
    return seen


def touchstone_puts(runs, base):
    reports = sorted(Path(runs).glob("*/run.json"))
    if not reports:
        raise ValueError("no fresh Touchstone evidence")
    urls = set()
    origin = urlsplit(base)[:2]

    def visit(node):
        if isinstance(node, dict):
            if node.get("method") == "PUT" and isinstance(node.get("status"), int) and 200 <= node["status"] < 300:
                # Reports also contain writes to the identity and webhook fixture host.
                if urlsplit(node["uri"])[:2] == origin:
                    urls.add(node["uri"])
            for value in node.values():
                visit(value)
        elif isinstance(node, list):
            for value in node:
                visit(value)
    visit(json.loads(reports[-1].read_text()))
    if not urls:
        raise ValueError("Touchstone evidence contains no successful PUT")
    return urls


def capture(client, args):
    # Keep these outside the suites' cleanup trees. Exercise RDF, arbitrary binary
    # bytes and overwrite, and retain the PUT target list across both restart phases.
    manifest = Path(args.manifest)
    previous = json.loads(manifest.read_text()) if manifest.exists() else {"probes": []}
    parent = "/alice/durable-recheck/" if args.protocol == "solid" else "/durable-recheck/"
    probes = list(previous["probes"])
    for name, content_type, payload in (
        ("graph.ttl", "text/turtle", b'<urn:durable:s> <urn:durable:p> "persisted" .\n'),
        ("nested/blob.bin", "application/octet-stream", bytes(range(256)) + b"\x00\xff\r\n"),
    ):
        url = args.base + parent + args.phase + "/" + name
        if args.protocol == "lws":
            # LWS creates through POST; PUT replaces a resource that already exists.
            status, headers, _ = client.send("POST", args.base + "/",
                                             headers={"Content-Type": content_type,
                                                      "Slug": "durable-recheck-" + args.phase + "-" + name.replace("/", "-")},
                                             body=payload)
            if status != 201 or not headers.get("location"):
                raise ValueError(f"probe POST returned {status}; expected 201 with Location")
            url = urljoin(args.base + "/", headers["location"])
        for body in (b"replaced", payload) if name.endswith(".bin") else (payload,):
            status, _, _ = client.send("PUT", url, headers={"Content-Type": content_type}, body=body)
            if status not in (200, 201, 204):
                raise ValueError(f"PUT {url} returned {status}")
        probes.append(url)
    if args.protocol == "solid":
        roots = [args.base + "/" + user + "/" + args.test_container for user in ("alice", "bob")]
        records = solid_tree(client, roots)
    else:
        records = {url: record(client, url, optional=True) for url in touchstone_puts(args.runs, args.base)}
    for url in probes:
        records[url] = record(client, url)
    manifest.write_text(json.dumps({"probes": probes, "resources": list(records.values())}, indent=2) + "\n")
    print("Captured conformance resources and persistence PUTs")


def check(client, args):
    manifest = json.loads(Path(args.manifest).read_text())
    if not manifest["probes"] or not manifest["resources"]:
        raise ValueError("empty persistence manifest")
    for saved in manifest["resources"]:
        status, headers, body = client.send("GET", saved["url"], saved["identity"], {"Accept": "*/*"})
        if status != saved["status"]:
            raise ValueError("post-restart status changed: " + saved["url"])
        if status == 200:
            if body != unb64(saved["body"]):
                raise ValueError("post-restart bytes changed: " + saved["url"])
            if headers.get("etag") != saved["etag"]:
                raise ValueError("post-restart ETag changed: " + saved["url"])
    print("Post-restart bytes, ETags and deletions match")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("capture", "check"))
    parser.add_argument("--protocol", choices=("solid", "lws"), required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--phase", choices=("term", "kill"), default="term")
    parser.add_argument("--env-file")
    parser.add_argument("--test-container", default="test/")
    parser.add_argument("--key")
    parser.add_argument("--owner")
    parser.add_argument("--runs")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="sparq-conformance-auth-") as directory:
        client = Client(args, directory)
        (capture if args.command == "capture" else check)(client, args)


if __name__ == "__main__":
    main()
