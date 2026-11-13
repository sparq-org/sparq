# Noir query expression core

Research grade and not externally audited. This library contains isolated
expression gadgets; it does not establish a credential or query proof.

Use nargo `1.0.0-beta.21` and bb `5.0.0-nightly.20260324`.
The compiler rejects prerelease requirements in `compiler_version`, so the
Rust live test checks the exact versions before running Noir.

Each expression returns `(Lane<T>, is_error)`. `Lane<T>` carries a separate
`capacity` bit. Constrain that bit to false at the selected expression root.
A capacity refusal must survive COALESCE; IF selects its result branch.

Exact numbers use signed magnitude scaled by `10^18`, with magnitude below
`2^112`. Excess range or fractional precision produces a capacity refusal.
Integer division produces decimal. Float and double follow IEEE semantics,
including infinity and NaN for division by zero. Invalid numeric or boolean
lexicals have false EBV, while an unbound variable has an error EBV.

`expr_term_contract::CheckedOpening` is a temporary compile-time seam for the
term module. Its implementation must bind every scalar field to the lexical
opening and recomputed comparison key. A bare `TermCell` cannot implement this
contract merely by carrying a witness flag. Computed term encoding belongs to
the term module; unary plus retains the original term identity.

Run `nargo check --deny-warnings` and `nargo test` here. The Rust differential
harness also runs generated Noir samples through the shared toolchain gate.
