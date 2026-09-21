# Backend airport catalog

These files are owned and maintained by this backend. `airports.json` and
`city-airport-groups.json` were initially copied unchanged from the frontend on
2026-09-21. No frontend checkout, automatic sync or frontend request is needed.

`city-airport-groups.json` is the pricing geography authority. Every valid group
is exported, currently all 133 groups with 275 distinct airport codes, including
single-airport groups. Adding or editing a group does not require an allowlist
or an entry in `airport-group-reviews.json`.

- `city-airport-groups.json`: configured city/metro memberships and country codes.
  Edit this file to change matching. These are application-defined groups, not
  a claim that every airport is physically in the named city or IATA-certified.
- `airports.json`: supplementary airport records for audit diagnostics. Missing,
  inactive, conflicting or non-airport records are reported; they do not silently
  remove explicit memberships from the authoritative group file.
- `airport-group-reviews.json`: optional historical source/review metadata only.
  It neither enables nor disables a group; stale reviews are reported.

Matching accepts an exact airport code or two airports directly sharing a group.
Country or similar names alone never establish a match. All direct memberships
are retained: TTN belongs to both NYC and PHL_CITY, so JFK/TTN and TTN/PHL match,
but JFK/PHL do not. Groups are not transitively merged. Conflicting country codes
for the same airport across group definitions are rejected.

Malformed/empty groups, invalid airport/country codes and duplicate members in
one group are rejected. Raw-catalog anomalies and overlaps remain in the audit
report for correction. A wrong configured membership affects pricing matching;
the generator validates structure and consistency, not real-world geography.

After editing backend data, run from the repository root:

```sh
python3 scripts/audit-airport-mapping.py
python3 scripts/test-airport-mapping.py
python3 scripts/audit-airport-mapping.py --check
cargo test --locked --lib locations::tests
```

The generator writes `src/airport_groups.json` and
`docs/evidence/AIRPORT_MAPPING_AUDIT_2026-09-21.json`. Do not edit these generated
files by hand. `--check` reads only and fails if they are stale. Script paths are
resolved relative to the repository, independent of the current directory.

Rust embeds all generated groups at build time and builds a lookup index once
on first use. No JSON file read, frontend call or database lookup occurs per
request. Rebuild/deploy through the normal release workflow after data changes;
editing a file alone does not change a running service. Search and RePrice use
the same mapping, while RePrice still requires exact selected flight airports.
These groups do not establish that a ground transfer is supplied or feasible.
