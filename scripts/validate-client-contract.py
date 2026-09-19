"""Validate private audit captures without printing their values.

Requires jsonschema (Draft 2020-12). Run against the OpenAPI response saved by
the UAT audit, or an exported /openapi.json, and one or more evidence folders.
Raw supplier captures and request fixtures are deliberately excluded.
"""
import argparse
import json
from decimal import Decimal
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker


NAMES = {
    "search": "SearchResponse", "rules": "FareRulesResponse",
    "reprice": "RepriceResponse", "accept": "AcceptanceResponse",
    "pricing": "PricingSnapshot", "book": "BookingResponse",
    "book-replay": "BookingResponse", "booking-saved": "BookingResponse",
    "issue": "TicketResponse", "issue-replay": "TicketResponse",
    "ticket-saved": "TicketResponse", "ticket-report": "TicketReport",
    "pnr": "PnrResponse", "identity": "Machine",
    "wallet-before": "WalletBalance", "wallet-after": "WalletBalance",
    "wallet-statement": "WalletStatement",
}
MONEY = ("baseFare", "taxes", "ait", "serviceCharge", "discount",
         "tierAdjustment", "gross", "payable")


def breakdowns(value):
    if isinstance(value, dict):
        if all(k in value for k in MONEY) and "passengers" in value:
            yield value
        for child in value.values():
            yield from breakdowns(child)
    elif isinstance(value, list):
        for child in value:
            yield from breakdowns(child)


def reconcile(b):
    def equation(v):
        return (Decimal(v["baseFare"]) + Decimal(v["taxes"]) + Decimal(v["ait"])
                + Decimal(v["serviceCharge"]) - Decimal(v["discount"])
                + Decimal(v["tierAdjustment"]) == Decimal(v["payable"]))
    assert equation(b), "aggregate payable arithmetic"
    for p in b["passengers"].values():
        assert equation(p), "passenger payable arithmetic"
    for key in MONEY:
        assert sum(Decimal(p[key]) * p["count"] for p in b["passengers"].values()) == Decimal(b[key]), "count-weighted " + key


def main():
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("openapi", type=Path)
    args.add_argument("folders", type=Path, nargs="+")
    args = args.parse_args()
    doc = json.loads(args.openapi.read_text())
    doc = doc.get("body", doc)
    components = doc["components"]
    for schema in components["schemas"].values():
        Draft202012Validator.check_schema(schema)
    responses = fares = 0
    failures = []
    for folder in args.folders:
        for path in sorted(folder.glob("*.json")):
            stem = path.stem
            if stem.startswith("supplier-") or stem.endswith("-request"):
                continue
            name = NAMES.get(stem) or next((v for k, v in NAMES.items() if stem.endswith("-" + k)), None)
            if name is None:
                continue
            capture = json.loads(path.read_text(), parse_float=Decimal)
            if capture["status"] >= 400:
                name = "ErrorResponse"
            elif capture["status"] == 202 and name == "BookingResponse":
                name = "PendingBooking"
            schema = {"$ref": "#/components/schemas/" + name, "components": components}
            validator = Draft202012Validator(schema, format_checker=FormatChecker())
            errors = list(validator.iter_errors(capture["body"]))
            responses += 1
            for error in errors:
                # No error.message or instance: they can contain passenger data.
                failures.append(f"{folder.name}/{path.name}: {list(error.absolute_path)} {error.validator}")
            for b in breakdowns(capture["body"]):
                try:
                    reconcile(b)
                    fares += 1
                except (AssertionError, KeyError, TypeError, ValueError):
                    failures.append(f"{folder.name}/{path.name}: fare arithmetic")
    print(json.dumps({"responses": responses, "reconciled_fare_breakdowns": fares,
                      "failures": failures}, indent=2))
    return 1 if failures or not responses else 0


if __name__ == "__main__":
    raise SystemExit(main())
