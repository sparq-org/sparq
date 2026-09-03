#!/usr/bin/env python3
"""Extract a fail-closed EC2 on-demand quote and enforce the study budget."""

from __future__ import annotations

import argparse
import json
from decimal import Decimal
from pathlib import Path
from typing import Any


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


def quote(
    payload: dict[str, Any],
    hours: Decimal,
    ancillary_reserve: Decimal,
    prior_spend: Decimal,
    planned_ceiling: Decimal,
    study_ceiling: Decimal,
) -> dict[str, str]:
    if min(hours, ancillary_reserve, prior_spend) < 0:
        raise PricingError("hours, ancillary reserve, and prior spend must be non-negative")
    hourly = hourly_price(payload)
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pricing_json", type=Path)
    parser.add_argument("--hours", type=Decimal, default=Decimal("12"))
    parser.add_argument("--ancillary-reserve", type=Decimal, default=Decimal("5"))
    parser.add_argument("--prior-spend", type=Decimal, default=Decimal("0"))
    parser.add_argument("--planned-ceiling", type=Decimal, default=Decimal("80"))
    parser.add_argument("--study-ceiling", type=Decimal, default=Decimal("100"))
    args = parser.parse_args()
    try:
        payload = json.loads(args.pricing_json.read_text(encoding="utf-8"))
        result = quote(
            payload,
            args.hours,
            args.ancillary_reserve,
            args.prior_spend,
            args.planned_ceiling,
            args.study_ceiling,
        )
    except (OSError, json.JSONDecodeError, PricingError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
