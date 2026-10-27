# Collaboration client interoperability fixture

<!-- internal-stub -->

This private test package supplies real JavaScript peers for the Rust router
fixture. It checks editing, awareness forwarding, viewer departure and continued
editing in both server modes. The dependencies are test-only.
Peers communicate over sockets with local broadcast channels disabled.

From the repository root:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix crates/sparq-lws-core/tests/fixtures/crdt-interop
npm test --prefix crates/sparq-lws-core/tests/fixtures/crdt-interop
```

The ordinary Rust suite skips this optional test because its dependencies need
separate installation. The dedicated command runs it explicitly. All servers
and clients use the local Rust test fixture; no deployed service is needed.
