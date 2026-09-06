# Verification — the 1249 no-AXIS projected EPSG entries in `epsg_full`

<!-- [SONNET-4.6] Verification record for issue #4284, split out of #3752 (group 3).
     No behaviour change: the codes verified here stay fail-closed. 🤖 SPARQ agent. -->

> 🤖 **SPARQ agent** — verification record for @jeswr's review.

**Status:** VERIFIED. **Issue:** #4284 (split from #3752, group 3).
**Subject:** `crates/sparq-geo/src/reproject.rs`, the `epsg_full` projected axis-order gate.

**Verdict in one line:** the inference #4284 was asked to check is **CONFIRMED with one
correction** — of the 1249 projected no-AXIS registry entries, **1241 are officially
northing/easting with zero exceptions**, but the other **8 are officially easting/northing**,
so **"no AXIS node ⇒ northing-first ⇒ swap (x, y)" is NOT a safe blanket rule**. Those 8 are
exactly the entries whose `wkt` string is **empty**, which is mechanically detectable. The
current fail-closed refusal of the whole bucket stays correct and is unchanged by this record.

---

## 0. What #4284 asked

Issue #3752 landed the axis-order gate but deliberately did not act on the no-AXIS bucket. The
premise recorded there was an **inference about an upstream exporter's behaviour**: the WKT1
export "appears to omit AXIS nodes exactly when the official EPSG order is not
easting/northing". Acting on it means emitting silently-transposed coordinates for 1249 codes
if it is wrong, so #4284 required verification against the authoritative EPSG registry — the
coordinate-system axis table — before any swap is trusted.

## 1. Correction to the premise: the upstream is PostGIS, not crs-csv

Issue #4284 (and the module docs) attributed the data to "crs-csv". That is wrong.
`crs-definitions` 0.5.0 generates `src/defs.rs` from **PostGIS `spatial_ref_sys`** —
`scripts/regenerate.py` spins up `postgis/postgis:16-3.4` and dumps

```sql
SELECT ... proj4text ..., srtext ... FROM spatial_ref_sys WHERE auth_name = 'EPSG' ORDER BY srid;
```

So the `wkt` column is PostGIS's `srtext` (a GDAL/PROJ WKT1 export) and `proj4` is
`proj4text`. The module docs are corrected accordingly in this change. This matters because it
identifies *which* exporter's behaviour the inference is about, and it is the reason the
empty-WKT rows in §3 exist at all.

## 2. Authoritative source used

The **EPSG Geodetic Parameter Dataset v11.022 (2024-11-05)**, as redistributed in PROJ 9.5.1's
`proj.db` (extracted from the `pyproj` 3.7.2 manylinux wheel; PROJ redistributes EPSG under the
EPSG terms of use). The relevant tables are exactly the coordinate-system axis table #4284
names:

- `projected_crs(auth_name, code, coordinate_system_auth_name, coordinate_system_code)`
- `axis(coordinate_system_auth_name, coordinate_system_code, coordinate_system_order, name,
  abbrev, orientation)`

**Key methodological point.** Classify on the axis **`name`** column (`Easting` / `Northing`),
NOT on `orientation` and NOT on `abbrev`:

- `orientation` is non-cardinal for polar CRSs (`"north along 90°E"`, `"south along 45°E"`), so
  a naive cardinal comparison reports 47 spurious disagreements — every one of them a polar
  stereographic / LAEA / UPS / SCAR / NSIDC grid where the WKT's cardinal `EAST`/`NORTH`
  keywords are the conventional approximation of the same axis. All 47 resolve correctly on
  `name`.
- `abbrev` is ambiguous: EPSG uses `X`/`Y` for both orders — `X = Northing` for the
  Gauss-Krüger families, `X = Easting` for e.g. `NAD27 / Michigan Central`. Classifying on
  `abbrev` manufactures a false counterexample set.

## 3. Result

Over all 6184 entries in `crs-definitions` 0.5.0 (763 geographic, 5421 projected):

| bucket | count | official EPSG axis order |
| --- | --- | --- |
| projected, no `AXIS[` node, **non-empty** WKT | **1241** | `(Northing, Easting)` — **1241/1241, zero exceptions** |
| projected, no `AXIS[` node, **empty** WKT | **8** | `(Easting, Northing)` — **8/8** |
| projected, `AXIS[…,EAST],AXIS[…,NORTH]` accepted by the gate | 3687 | `(Easting, Northing)` — confirmed |
| projected, `+axis=wsu` / `+axis=swu` accepted by the gate | 29 | westing/southing grids — confirmed |
| projected, accepted by the gate, absent from EPSG v11.022 | 185 | no record (retired codes) — see §5 |

**The converse also closes.** EPSG v11.022 marks **1243** of the crate's projected entries
`(Northing, Easting)`. 1241 of them are the no-AXIS bucket; the other **2 are EPSG:8433 (Macao
Grid) and EPSG:8441 (Tananarive / Laborde Grid)** — precisely the two transposed
`AXIS[…,NORTH],AXIS[…,EAST]` entries the module docs already name and refuse. 1241 + 2 = 1243,
with nothing left over. There is no projected entry that is officially northing-first and
silently accepted.

**Control on the accepted bucket: zero mismatches.** No entry the shipped gate accepts is
officially northing-first. The gate is not currently emitting transposed coordinates.

### The 8 empty-WKT entries

| EPSG | name | why it matters |
| --- | --- | --- |
| 3993 | Guam 1963 / Guam SPCS | |
| 6200 | NAD27 / Michigan North (deprecated) | |
| 6201 | NAD27 / Michigan Central | |
| 6202 | NAD27 / Michigan South | |
| 6966 | NAD27 / Michigan North | |
| 8857 | WGS 84 / Equal Earth Greenwich | plain `+datum=WGS84` |
| 8858 | WGS 84 / Equal Earth Americas | plain `+datum=WGS84` |
| 8859 | WGS 84 / Equal Earth Asia-Pacific | plain `+datum=WGS84` |

All eight are officially **easting/northing**, and all eight are refused by the **axis rule
alone** — the grid-shift and datum-less filters pass every one of them. So a blanket swap over
the no-AXIS bucket would newly admit these eight AND transpose them, with no other honesty
filter standing behind it. The three Equal Earth codes are the sharpest: WGS84 datum, no grid
shift, nothing else to object to.

This is pinned by `registry_entries_with_an_empty_wkt_stay_refused` in
`crates/sparq-geo/tests/reproject.rs`.

## 4. What this does and does not license

**Licensed:** the discriminator for a future swap is **"non-empty WKT carrying no `AXIS` node"**,
not "no `AXIS` node". On that predicate the registry evidence is 1241/1241 with the converse
closed — i.e. the *registry* half of the module's policy is now satisfied for those 1241 codes.

**Not licensed:** the swap itself. The module's policy requires the registry check **plus an
independent worked example per family**, and this record does **not** provide worked examples.
The families in the bucket are wide (Pulkovo 1942/1995 Gauss-Krüger 436, ETRS89 108, the Chinese
Beijing/Xian/CGCS2000/New Beijing set 267, NZGD2000/NZGD49 64, JGD/Tokyo 57, Korean 21, plus 103
distinct datum prefixes in all). Until those land, the bucket stays
fail-closed `GeoError::Unsupported`, which remains the correct default. **No behaviour changes
in this record.**

## 5. Residuals (not addressed here)

- **185 codes the gate accepts that EPSG v11.022 has no record of.** These are retired/renumbered
  codes still carried by the PostGIS snapshot. They are accepted today — this is pre-existing, not
  introduced by #3752 — but they are unverifiable against the current registry by construction.
  Worth a separate look; not in scope for #4284.
- **PostGIS snapshot vs EPSG v11.022 skew.** The crate's data is whatever
  `postgis/postgis:16-3.4` ships, which is an older EPSG snapshot than v11.022. The 1241/1241
  agreement is strong evidence the two do not disagree on axis order for these codes, but a
  future `crs-definitions` bump should re-run §6 rather than assume it still holds.

## 6. Reproduction

Self-contained; needs network and Python 3 stdlib only.

```python
import re, sqlite3, urllib.request, tarfile, zipfile, io, collections

# 1. crs-definitions 0.5.0 — the data the crate embeds.
b = urllib.request.urlopen(
    "https://static.crates.io/crates/crs-definitions/crs-definitions-0.5.0.crate").read()
t = tarfile.open(fileobj=io.BytesIO(b))
src = t.extractfile("crs-definitions-0.5.0/src/defs.rs").read().decode()
defs = {int(c): (p, w) for c, _, p, w in
        re.findall(r'^EPSG_(\d+)\|(\d+)\|"(.*?)"\|r#"(.*?)"#\|$', src, re.M | re.S)}

# 2. EPSG v11.022 via PROJ 9.5.1 proj.db (pyproj wheel).
u = ("https://files.pythonhosted.org/packages/b8/be/"
     "212882c450bba74fc8d7d35cbd57e4af84792f0a56194819d98106b075af/"
     "pyproj-3.7.2-cp312-cp312-manylinux_2_28_x86_64.whl")
z = zipfile.ZipFile(io.BytesIO(urllib.request.urlopen(u).read()))
open("proj.db", "wb").write(z.read("pyproj/proj_dir/share/proj/proj.db"))
db = sqlite3.connect("proj.db")
print(db.execute("select value from metadata where key='EPSG.VERSION'").fetchone())  # v11.022

def axes(code):  # authoritative (name, name) for axis order 1 and 2
    r = db.execute("select coordinate_system_auth_name, coordinate_system_code "
                   "from projected_crs where auth_name='EPSG' and code=?", (str(code),)).fetchone()
    if not r: return None
    a = db.execute("select name from axis where coordinate_system_auth_name=? and "
                   "coordinate_system_code=? order by coordinate_system_order", r).fetchall()
    return (a[0][0], a[1][0]) if len(a) >= 2 else None

proj = [c for c, (p, w) in defs.items() if not p.startswith("+proj=longlat")]
zero = [c for c in proj if "AXIS[" not in defs[c][1]]
empty = [c for c in zero if not defs[c][1].strip()]
real  = [c for c in zero if defs[c][1].strip()]
print(len(zero), len(empty), len(real))                       # 1249 8 1241
print(collections.Counter(axes(c) for c in real))             # {('Northing','Easting'): 1241}
print(collections.Counter(axes(c) for c in empty))            # {('Easting','Northing'): 8}
# converse: officially northing-first entries that DO carry AXIS nodes
print([c for c in proj if axes(c) == ("Northing", "Easting") and "AXIS[" in defs[c][1]])  # [8433, 8441]
```
