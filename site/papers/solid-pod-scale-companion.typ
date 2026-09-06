// [GPT-6] Exhaustive source-bound tables accompanying the concise manuscript.
#import "solid-pod-scale-results.typ": main-state, main-tables
#let data = json("../../research/solid-pod-scale-main.json")
#assert(main-state(data) == "finalized and reviewed", message: "Companion requires final reviewed evidence")
#set document(title: "Solid Pod Query Study — Companion Tables")
#set page(paper: "a4", margin: (x: 20mm, y: 16mm), numbering: "1")
#set text(font: "Libertinus Serif", size: 10pt)
#set par(justify: false, leading: 0.5em)
#set table(stroke: (x: none, y: 0.35pt + luma(75%)))
#align(center)[#text(size: 15pt, weight: "bold")[Solid Pod Query Study: Companion Tables]]
These tables accompany the local review manuscript. Source: #raw(data.source_commit).
The #link("../../research/solid-pod-scale-main.json")[analysis JSON] records raw-input
hashes, complete request accounting, source review and per-cell validity. CPU usage
can include outstanding warm-up work; cache headers cover only observed replies.
This bundle has no public archival URL.
#main-tables(data)
