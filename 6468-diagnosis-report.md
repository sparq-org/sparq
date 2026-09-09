[GPT-6 Astra] Read-only #6468 diagnosis.

Two captured current rows reproduce the missing label through T2: #5016 via its
explicit generator title scope, and #6468 via the generator token mentioned by
the classifier diagnostic itself. #5016 is a strong pre-existing candidate for
the original failure; original row attribution remains unproven because the run
recorded no input/evidence row. #6468 was created after that failure.

The classifier repair belongs to existing area:ci. A narrow title-scoped rule and
fixture can correct #6468; an executed red fixture demonstrates today's wrong
result. However this alone leaves #5016 as an unsupported generator partition.
Historical #2658's area:wrapper is not a safe alias: current readiness resolves
wrapper, sparq-wrapper and sparq-wrapper-gen as disjoint roots. PR path attribution
also uses the real generator crate. No supported existing replacement for actual
generator work was established. Do not mask this by renaming its area.

The smallest defensible proposal is the narrow classifier-owner rule plus better
prewrite row/tier diagnostics, retaining the global unknown-label zero-write guard.
Actual generator-label/partition reconciliation needs an explicit policy decision
consistent across consumers. No implementation or remote mutation was made.
