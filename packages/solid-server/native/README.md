# Native npm package assembly

The existing release binary matrix builds `sparq-lws-core` into its versioned
archives. Assemble all platform packages from those downloaded archives:

```sh
node packages/solid-server/native/assemble.mjs artifacts packages/solid-server/native/packages
```

An optional third argument selects one platform for local validation. The input
archive names must match the host package version. Each generated manifest has
an exact matching version, npm `os`/`cpu` selectors, a binary, a README and a
license; there are no lifecycle scripts. The host resolves only installed
packages and never downloads executables at runtime.

`release.yml` uploads the assembled tarballs. `publish.yml` publishes them only
when explicitly dispatched with `publish_solid_server_native=true`, at the
verified release tag. Each platform package needs its own registry bootstrap
and trusted-publisher registration for `publish.yml` before that dispatch.
The maintainer controls publication.
