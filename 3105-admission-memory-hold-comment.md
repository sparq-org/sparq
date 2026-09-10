> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Holding this exact head for a measured memory regression found during the remaining validation. The allocation-counted full-miss query reaches every RHS and retains several relations at once. The eight-pattern case exceeds the experiment's predeclared memory screen; the same requested-heap metrics repeat across all three samples per variant. This is requested heap, not process RSS, and does not establish a latency result.

Generated diagnostic summary:

```json
{
  "base_commit": "e53464c73f31f7aca800f3867ac054c36408e346",
  "candidate_commit": "19763bfab1dce196a654c899b172e7b24d70bc59",
  "metric": "peak requested live heap growth during query, bytes",
  "samples_per_variant": 3,
  "seed_rows": 70000,
  "results": [
    {
      "total_patterns": 5,
      "main_bytes": 10811556,
      "candidate_bytes": 15435090,
      "additional_bytes": 4623534,
      "screen_no_go": false
    },
    {
      "total_patterns": 8,
      "main_bytes": 10813029,
      "candidate_bytes": 22157124,
      "additional_bytes": 11344095,
      "screen_no_go": true
    }
  ]
}
```

I am applying `review:changes` and revising retention before merge admission. Existing CI and review evidence remain intact. The initial independent source approval was for CI validation; it did not approve merging this memory tradeoff. #3105 remains open.
