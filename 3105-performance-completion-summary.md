Local advisory screening; timing uses original unchanged System binaries (five samples, three queries each). Allocation columns use the amended complete matrix (three samples, one query each).

| Case / view | Baseline ms/query range | Candidate ms/query range | C/B median | Timing CV B/C % | Requested bytes B/C | Peak requested live growth B/C | Allocations B/C | Reallocations B/C |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| first / base | 1.7761–1.8458 | 1.7668–1.8288 | 0.9948 | 1.67 / 1.45 | 21,370,986 / 21,371,162 | 8,712,554 / 8,712,730 | 1,242 / 1,243 | 200 / 200 |
| first / overlay | 1.7977–1.8576 | 1.8147–1.8524 | 1.0019 | 1.43 / 0.88 | 21,370,986 / 21,371,162 | 8,712,554 / 8,712,730 | 1,242 / 1,243 | 200 / 200 |
| second / base | 11.2952–11.5878 | 10.4790–10.6593 | 0.9326 | 0.98 / 0.69 | 38,767,905 / 28,139,966 | 10,777,001 / 8,712,742 | 65,828 / 65,805 | 390 / 294 |
| second / overlay | 11.3322–11.4750 | 10.3481–10.4684 | 0.9172 | 0.49 / 0.54 | 38,767,905 / 28,139,966 | 10,777,001 / 8,712,742 | 65,828 / 65,805 | 390 / 294 |
| miss / base | 12.2525–12.6864 | 10.6167–11.4521 | 0.8549 | 1.60 / 3.10 | 50,090,494 / 28,834,440 | 10,776,989 / 8,712,730 | 70,347 / 70,300 | 497 / 305 |
| miss / overlay | 12.4730–12.7045 | 10.5019–11.1092 | 0.8435 | 0.66 / 2.26 | 50,090,494 / 28,834,440 | 10,776,989 / 8,712,730 | 70,347 / 70,300 | 497 / 305 |
| late / base | 22.6154–23.2352 | 20.7066–21.2647 | 0.9279 | 1.06 / 0.99 | 70,275,472 / 49,019,418 | 15,851,195 / 15,851,195 | 332,467 / 332,420 | 497 / 305 |
| late / overlay | 22.7286–23.4327 | 20.8689–21.3586 | 0.9129 | 1.21 / 0.95 | 70,275,472 / 49,019,418 | 15,851,195 / 15,851,195 | 332,467 / 332,420 | 497 / 305 |
| multi / base | 15.8299–16.3201 | 12.6078–12.8911 | 0.7888 | 1.17 / 0.84 | 86,762,344 / 44,250,148 | 10,810,085 / 10,953,245 | 70,499 / 70,404 | 818 / 434 |
| multi / overlay | 16.1594–16.4652 | 12.5823–12.8839 | 0.7713 | 0.79 / 0.96 | 86,762,344 / 44,250,148 | 10,810,085 / 10,953,245 | 70,499 / 70,404 | 818 / 434 |

Full min/max/median/mean/sample-standard-deviation/CV and cumulative setup/before/after RSS ranges are in summary.json. Count metrics are identical across the three completed amended repetitions of every case. live_after is total process requested live bytes after the query, not query-retained bytes: the harness does not emit the begin baseline. Peak live growth subtracts that actual baseline internally. RSS is cumulative process high-water; setup includes the full oracle, and cannot isolate query physical memory.
