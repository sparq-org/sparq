> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

I am taking up this existing issue as the next direct SPARQL performance task. I will first verify and measure the current `Overlay::count_correction` path on fresh main; the historical issue timings are context, not current evidence.

The intended narrow change is a lazy sorted deletion projection for range counting, retaining the hash set used for membership. Validation will cover cache invalidation, mixed changes, snapshots/forks, cold and warm use, and the added memory cost, followed by actual independent Opus review before admission. The stopped indexed top-k experiment is tracked separately in #6465.
