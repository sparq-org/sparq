# Native collaboration string adapter

<!-- internal-stub -->

This unpublished compatibility package provides the `SmallString` surface used by
`yrs`, backed by the standard library's owned UTF-8 `String`. Its array type
parameter preserves the consumer's type signatures. It has no dependencies and
does not implement a Yjs codec or an inline storage optimization.
The default `union` compatibility feature accepts the consumer's manifest setting;
it does not change this implementation.

The root workspace patch selects this implementation for the native collaboration
dependency. Behavioral validation goes through the real collaboration routers,
including Unicode edits, persistence, and reload. This is a consumer-specific
interface rather than a general replacement for the registry package.

License: MIT.
