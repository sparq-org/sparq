#!/usr/bin/env python3
"""[GPT-6] Current-price reservation for the constrained disposable build host."""
import csv
from decimal import Decimal
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("aws_price", Path(__file__).parents[1] / "scaling/aws_price.py")
price = importlib.util.module_from_spec(spec)
spec.loader.exec_module(price)


def gp3_price(path):
    with Path(path).open(newline="") as stream:
        rows = csv.reader(stream)
        header = next(row for row in rows if row and row[0] == "SKU")
        matches = [row for row in csv.DictReader(stream, fieldnames=header)
                   if row.get("Region Code") == "eu-west-2" and row.get("Volume API Name") == "gp3"
                   and row.get("Product Family") == "Storage" and row.get("TermType") == "OnDemand"
                   and row.get("Unit") == "GB-Mo" and row.get("Currency") == "USD"]
    if len(matches) != 1:
        raise ValueError("expected exactly one London gp3 storage price")
    storage = Decimal(matches[0]["PricePerUnit"])
    if not storage.is_finite() or storage <= 0:
        raise ValueError("nonfinite or nonpositive price input")
    return storage, matches[0]["SKU"]


def reservation(path, prior):
    with Path(path).open(newline="") as stream:
        hourly, evidence = price.bulk_csv_hourly_price(stream, "c7g.4xlarge", "EU (London)", "eu-west-2")
    storage, storage_sku = gp3_price(path)
    if not prior.is_finite():
        raise ValueError("nonfinite prior allocation")
    hours = Decimal("4.25")
    # Shortest calendar month gives a conservative bound; no new contingency.
    ancillary = hours * (Decimal(200) * storage / Decimal(672) + Decimal("0.005"))
    result = price.quote_at_hourly_price(hourly, hours, ancillary, prior, Decimal("3.10"), Decimal(100))
    result.update(evidence)
    result.update(scope="build-and-functional-tests-only", invoice_verified=False,
                  gp3_gib=200, gp3_monthly_usd=str(storage), gp3_price_sku=storage_sku,
                  storage_month_hours_bound=672, watchdog_seconds=14400,
                  ipv4_hourly_usd="0.005", ipv4_price_source="https://aws.amazon.com/vpc/pricing/",
                  billing_cushion_seconds=900, prior_includes_existing_reserve=True)
    return result


if __name__ == "__main__":
    print(json.dumps(reservation(sys.argv[1], Decimal(sys.argv[2])), indent=2, sort_keys=True))
