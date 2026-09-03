#!/usr/bin/env python3
"""Extract a fail-closed EC2 on-demand quote and enforce the study budget."""

from __future__ import annotations

import argparse
import csv
import json
from decimal import Decimal
from pathlib import Path
from typing import Any, TextIO


class PricingError(RuntimeError):
    """The AWS price document does not identify one hourly price."""


def hourly_prices(payload: dict[str, Any]) -> set[Decimal]:
    prices: set[Decimal] = set()
    price_list = payload.get("PriceList")
    if not isinstance(price_list, list) or not price_list:
        raise PricingError("AWS PriceList is absent or empty")
    for encoded_product in price_list:
        product = json.loads(encoded_product) if isinstance(encoded_product, str) else encoded_product
        terms = product.get("terms", {}).get("OnDemand", {})
        for term in terms.values():
            for dimension in term.get("priceDimensions", {}).values():
                if dimension.get("unit") != "Hrs":
                    continue
                value = dimension.get("pricePerUnit", {}).get("USD")
                if value is not None:
                    prices.add(Decimal(str(value)))
    return prices


def hourly_price(payload: dict[str, Any]) -> Decimal:
    prices = hourly_prices(payload)
    if len(prices) != 1:
        raise PricingError(f"expected exactly one hourly USD price, found {sorted(prices)}")
    price = next(iter(prices))
    if price <= 0:
        raise PricingError(f"hourly price must be positive, found {price}")
    return price


def bulk_csv_hourly_price(
    stream: TextIO,
    instance_type: str,
    location: str,
    region_code: str,
) -> tuple[Decimal, dict[str, str]]:
    """Extract one regional Linux shared-tenancy on-demand instance price."""

    rows = csv.reader(stream)
    metadata: dict[str, str] = {}
    header: list[str] | None = None
    for row in rows:
        if row and row[0] == "SKU":
            header = row
            break
        if len(row) >= 2:
            metadata[row[0]] = row[1]
    if header is None:
        raise PricingError("AWS bulk CSV has no SKU header")
    if metadata.get("FormatVersion") != "v1.0":
        raise PricingError(
            f"unexpected AWS bulk CSV format {metadata.get('FormatVersion')!r}"
        )
    if metadata.get("OfferCode") != "AmazonEC2":
        raise PricingError(
            f"unexpected AWS bulk CSV offer {metadata.get('OfferCode')!r}"
        )

    filters = {
        "OfferTermCode": "JRTCKXETXF",
        "TermType": "OnDemand",
        "Unit": "Hrs",
        "Currency": "USD",
        "Product Family": "Compute Instance",
        "serviceCode": "AmazonEC2",
        "Location": location,
        "Location Type": "AWS Region",
        "Instance Type": instance_type,
        "Tenancy": "Shared",
        "Operating System": "Linux",
        "License Model": "No License required",
        "usageType": f"EUW2-BoxUsage:{instance_type}",
        "operation": "RunInstances",
        "AvailabilityZone": "NA",
        "CapacityStatus": "Used",
        "MarketOption": "OnDemand",
        "Pre Installed S/W": "NA",
        "Region Code": region_code,
    }
    missing = sorted(set(filters) - set(header))
    if missing:
        raise PricingError(f"AWS bulk CSV lacks required columns {missing}")

    matches = [
        row
        for row in csv.DictReader(stream, fieldnames=header)
        if all(row.get(key) == expected for key, expected in filters.items())
    ]
    if len(matches) != 1:
        raise PricingError(
            f"expected exactly one AWS bulk CSV row for {instance_type}, found {len(matches)}"
        )
    match = matches[0]
    try:
        price = Decimal(match["PricePerUnit"])
    except (KeyError, ArithmeticError) as error:
        raise PricingError("AWS bulk CSV match has an invalid PricePerUnit") from error
    if price <= 0:
        raise PricingError(f"hourly price must be positive, found {price}")
    evidence = {
        "price_source": "aws-price-list-bulk-csv",
        "price_list_format": metadata["FormatVersion"],
        "price_list_publication_date": metadata.get("Publication Date", ""),
        "price_list_version": metadata.get("Version", ""),
        "price_list_offer_code": metadata["OfferCode"],
        "price_sku": match.get("SKU", ""),
        "price_rate_code": match.get("RateCode", ""),
        "price_description": match.get("PriceDescription", ""),
    }
    return price, evidence


def quote_at_hourly_price(
    hourly: Decimal,
    hours: Decimal,
    ancillary_reserve: Decimal,
    prior_spend: Decimal,
    planned_ceiling: Decimal,
    study_ceiling: Decimal,
) -> dict[str, str]:
    """Apply the study's conservative ceilings to one validated hourly price."""

    if hourly <= 0:
        raise PricingError(f"hourly price must be positive, found {hourly}")
    if min(hours, ancillary_reserve, prior_spend) < 0:
        raise PricingError("hours, ancillary reserve, and prior spend must be non-negative")
    compute = hourly * hours
    planned = compute + ancillary_reserve
    accumulated = prior_spend + planned
    if planned > planned_ceiling:
        raise PricingError(f"planned run ${planned} exceeds ${planned_ceiling} operating ceiling")
    if accumulated > study_ceiling:
        raise PricingError(f"study total ${accumulated} exceeds ${study_ceiling} ceiling")
    return {
        "currency": "USD",
        "instance_hourly": str(hourly),
        "maximum_instance_hours": str(hours),
        "maximum_compute": str(compute),
        "ancillary_ebs_ipv4_reserve": str(ancillary_reserve),
        "maximum_planned_run": str(planned),
        "prior_study_spend": str(prior_spend),
        "maximum_accumulated_study_spend": str(accumulated),
        "planned_ceiling": str(planned_ceiling),
        "study_ceiling": str(study_ceiling),
    }


def quote(
    payload: dict[str, Any],
    hours: Decimal,
    ancillary_reserve: Decimal,
    prior_spend: Decimal,
    planned_ceiling: Decimal,
    study_ceiling: Decimal,
) -> dict[str, str]:
    hourly = hourly_price(payload)
    return quote_at_hourly_price(
        hourly,
        hours,
        ancillary_reserve,
        prior_spend,
        planned_ceiling,
        study_ceiling,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pricing_json", type=Path)
    parser.add_argument("--hours", type=Decimal, default=Decimal("12"))
    parser.add_argument("--ancillary-reserve", type=Decimal, default=Decimal("5"))
    parser.add_argument("--prior-spend", type=Decimal, default=Decimal("0"))
    parser.add_argument("--planned-ceiling", type=Decimal, default=Decimal("80"))
    parser.add_argument("--study-ceiling", type=Decimal, default=Decimal("100"))
    parser.add_argument("--bulk-csv", action="store_true")
    parser.add_argument("--instance-type")
    parser.add_argument("--location", default="EU (London)")
    parser.add_argument("--region-code", default="eu-west-2")
    args = parser.parse_args()
    try:
        if args.bulk_csv:
            if not args.instance_type:
                raise PricingError("--instance-type is required with --bulk-csv")
            with args.pricing_json.open(encoding="utf-8", newline="") as stream:
                hourly, evidence = bulk_csv_hourly_price(
                    stream,
                    args.instance_type,
                    args.location,
                    args.region_code,
                )
            result = quote_at_hourly_price(
                hourly,
                args.hours,
                args.ancillary_reserve,
                args.prior_spend,
                args.planned_ceiling,
                args.study_ceiling,
            )
            result.update(evidence)
        else:
            payload = json.loads(args.pricing_json.read_text(encoding="utf-8"))
            result = quote(
                payload,
                args.hours,
                args.ancillary_reserve,
                args.prior_spend,
                args.planned_ceiling,
                args.study_ceiling,
            )
            result["price_source"] = "aws-price-list-query-api"
    except (OSError, json.JSONDecodeError, PricingError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
