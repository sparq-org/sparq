# Canonicalisation regression guest

This unpublished, independent guest executes the standard canonicaliser for
native-versus-guest byte comparisons. The methods crate builds it only with
the existing `phase-cycles` measurement feature. It is never accepted by an
authenticated evaluator verifier. Its input is an N-Quads string and a flag
selecting the SHA-384 profile. Its journal contains the standard canonical
output, the bounded canonical output and the output relabelled by the ordered bounded issuer, with
each result retaining its error independently.
