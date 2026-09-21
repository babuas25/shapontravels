#!/usr/bin/env python3
"""Audit the backend catalog; export every configured application group.

City groups are the pricing authority. Raw airport data and optional reviews
provide diagnostics, never an allowlist. No transitive or country-wide aliases.
Reads data/airports.json, data/city-airport-groups.json and review metadata
from this repository. Use --check to verify generated files without writing.
Neither catalog maintenance nor the build needs a frontend checkout.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re


def audit(rows, groups, reviews):
    by_code = collections.defaultdict(list)
    for row in rows:
        code = row.get('iata', '')
        if row.get('status') == 1 and isinstance(code, str) and re.fullmatch('[A-Z]{3}', code):
            by_code[code].append(row)
    duplicates = {code: len(items) for code, items in sorted(by_code.items()) if len(items) > 1}
    conflicts = {
        code: sorted({(r.get('city', ''), r.get('iso', '')) for r in items})
        for code, items in sorted(by_code.items())
        if len({(r.get('city', ''), r.get('iso', '')) for r in items}) > 1
    }
    membership = collections.defaultdict(list)
    configured = []
    country_mismatches = {}
    non_airports = {}
    stale_reviews = []
    for key, group in sorted(groups.items()):
        country = group.get('countryCode', '')
        airports = group.get('airports')
        if (not isinstance(key, str) or not key.strip()
                or not isinstance(group.get('city'), str) or not group['city'].strip()
                or not isinstance(country, str) or not re.fullmatch('[A-Z]{2}', country)
                or not isinstance(airports, list) or not airports
                or any(not isinstance(code, str) or not re.fullmatch('[A-Z]{3}', code)
                       for code in airports)
                or len(set(airports)) != len(airports)):
            raise ValueError(f'Invalid city group: {key}')
        for code in airports:
            membership[code].append(key)
            countries = sorted({r.get('iso', '') for r in by_code[code]})
            if countries and countries != [country]:
                country_mismatches[f'{key}/{code}'] = countries
            types = sorted({r.get('type', '') for r in by_code[code]})
            if types and types != ['airport']:
                non_airports[f'{key}/{code}'] = types
        configured.append({'id': key, 'city': group['city'], 'country': country,
                           'airports': sorted(airports)})
    overlap = {code: sorted(keys) for code, keys in sorted(membership.items()) if len(keys) > 1}
    for code, keys in overlap.items():
        if len({groups[key]['countryCode'] for key in keys}) > 1:
            raise ValueError(f'Airport assigned to conflicting group countries: {code}')
    # Reviews are historical provenance only. Adding/editing a configured group
    # never requires a second entry here, and stale metadata grants no approval.
    for key, review in sorted(reviews.items()):
        group = groups.get(key)
        if (group is None or group['countryCode'] != review.get('countryCode')
                or sorted(group['airports']) != sorted(review.get('airports', []))):
            stale_reviews.append(key)
    report = {'rows': len(rows), 'activeRows': sum(map(len, by_code.values())),
              'distinctActiveCodes': sum(bool(items) for items in by_code.values()),
              'duplicateCodes': duplicates, 'conflictingLocations': conflicts,
              'overlappingGroups': overlap,
              'missingGroupAirports': sorted(code for code in membership if not by_code[code]),
              'groupCountryMismatches': country_mismatches,
              'nonAirportGroupMembers': non_airports,
              'enabledGroups': [g['id'] for g in configured],
              'groupsWithoutReview': sorted(set(groups) - set(reviews)),
              'staleReviews': stale_reviews,
              'matchingPolicy': 'exact airport or direct shared group; no transitive merge'}
    return configured, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    airport_path = root / 'data/airports.json'
    group_path = root / 'data/city-airport-groups.json'
    review_path = root / 'data/airport-group-reviews.json'
    configured, report = audit(json.loads(airport_path.read_text()), json.loads(group_path.read_text()),
                             json.loads(review_path.read_text()) if review_path.exists() else {})
    report['sha256'] = {name: hashlib.sha256(p.read_bytes()).hexdigest() for name, p in
                       [('airports', airport_path), ('groups', group_path), ('reviews', review_path)] if p.exists()}
    for path, value in [(root / 'src/airport_groups.json', configured),
                        (root / 'docs/evidence/AIRPORT_MAPPING_AUDIT_2026-09-21.json', report)]:
        text = json.dumps(value, indent=2, ensure_ascii=False) + '\n'
        if args.check:
            if path.read_text() != text:
                raise SystemExit(f'Stale generated catalog/report: {path.name}')
        else:
            path.write_text(text)
    print(f"Audited {report['rows']} rows; {len(report['duplicateCodes'])} duplicate codes; "
          f"{len(report['conflictingLocations'])} conflicting locations; {len(configured)} configured groups enabled.")


if __name__ == '__main__':
    main()
