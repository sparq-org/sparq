# Async events extension

Implemented behind the default-off `proposed-async-events` feature in
`proposed::async_events`.

`AsyncObservableStore` wraps an `AsyncStore` and awaits each subscribed
listener's future, in subscription order, before an effective `add` / `delete`
resolves. Presence is checked first, so a duplicate add or an absent delete
performs no write and notifies nobody. Events reuse `proposed::observe`'s
`ChangeEvent`, `ChangeKind` and `SubscriptionId`. The feature implies
`proposed-async-store` and `proposed-observe`. Source: rdfjs/wrapper draft
PR #99. <!-- sq-1rg2q.10 -->
