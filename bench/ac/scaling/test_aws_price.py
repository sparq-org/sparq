#!/usr/bin/env python3
"""Regression tests for the EC2 quote and budget guard."""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from decimal import Decimal
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ac_aws_price", HERE / "aws_price.py")
assert SPEC and SPEC.loader
PRICE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = PRICE
SPEC.loader.exec_module(PRICE)


def payload(*prices: str) -> dict[str, object]:
    products = []
    for index, value in enumerate(prices):
        products.append(
            json.dumps(
                {
                    "terms": {
                        "OnDemand": {
                            f"term-{index}": {
                                "priceDimensions": {
                                    f"dimension-{index}": {
                                        "unit": "Hrs",
                                        "pricePerUnit": {"USD": value},
                                    }
                                }
                            }
                        }
                    }
                }
            )
        )
    return {"PriceList": products}


class AwsPriceTests(unittest.TestCase):
    def test_one_hourly_price_produces_conservative_quote(self) -> None:
        result = PRICE.quote(
            payload("0.25"),
            Decimal("12"),
            Decimal("5"),
            Decimal("1"),
            Decimal("80"),
            Decimal("100"),
        )
        self.assertEqual(result["maximum_compute"], "3.00")
        self.assertEqual(result["maximum_accumulated_study_spend"], "9.00")

    def test_ambiguous_prices_fail_closed(self) -> None:
        with self.assertRaises(PRICE.PricingError):
            PRICE.hourly_price(payload("0.25", "0.30"))

    def test_study_ceiling_is_enforced(self) -> None:
        with self.assertRaises(PRICE.PricingError):
            PRICE.quote(
                payload("1"),
                Decimal("12"),
                Decimal("5"),
                Decimal("90"),
                Decimal("80"),
                Decimal("100"),
            )


if __name__ == "__main__":
    unittest.main()
