#!/usr/bin/env python3
"""Regression tests for the EC2 quote and budget guard."""

from __future__ import annotations

import importlib.util
import csv
import io
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

    def test_bulk_csv_selects_exact_linux_shared_on_demand_row(self) -> None:
        columns = [
            "SKU",
            "OfferTermCode",
            "RateCode",
            "TermType",
            "PriceDescription",
            "Unit",
            "PricePerUnit",
            "Currency",
            "Product Family",
            "serviceCode",
            "Location",
            "Location Type",
            "Instance Type",
            "Tenancy",
            "Operating System",
            "License Model",
            "usageType",
            "operation",
            "AvailabilityZone",
            "CapacityStatus",
            "MarketOption",
            "Pre Installed S/W",
            "Region Code",
        ]
        selected = [
            "sku-1",
            "JRTCKXETXF",
            "rate-1",
            "OnDemand",
            "$0.42 per On Demand Linux r7g.xlarge Instance Hour",
            "Hrs",
            "0.4200000000",
            "USD",
            "Compute Instance",
            "AmazonEC2",
            "EU (London)",
            "AWS Region",
            "r7g.xlarge",
            "Shared",
            "Linux",
            "No License required",
            "EUW2-BoxUsage:r7g.xlarge",
            "RunInstances",
            "NA",
            "Used",
            "OnDemand",
            "NA",
            "eu-west-2",
        ]
        output = io.StringIO()
        writer = csv.writer(output, lineterminator="\n")
        writer.writerows(
            [
                ["FormatVersion", "v1.0"],
                ["Publication Date", "2026-09-03T00:00:00Z"],
                ["Version", "20260903000000"],
                ["OfferCode", "AmazonEC2"],
                columns,
                selected,
                [*selected[:13], "Dedicated", *selected[14:]],
            ]
        )
        output.seek(0)

        hourly, evidence = PRICE.bulk_csv_hourly_price(
            output, "r7g.xlarge", "EU (London)", "eu-west-2"
        )

        self.assertEqual(hourly, Decimal("0.4200000000"))
        self.assertEqual(evidence["price_sku"], "sku-1")
        self.assertEqual(evidence["price_list_version"], "20260903000000")

    def test_bulk_csv_ambiguous_exact_rows_fail_closed(self) -> None:
        text = """\
\"FormatVersion\",\"v1.0\"
\"OfferCode\",\"AmazonEC2\"
\"SKU\",\"OfferTermCode\",\"TermType\",\"Unit\",\"PricePerUnit\",\"Currency\",\"Product Family\",\"serviceCode\",\"Location\",\"Location Type\",\"Instance Type\",\"Tenancy\",\"Operating System\",\"License Model\",\"usageType\",\"operation\",\"AvailabilityZone\",\"CapacityStatus\",\"MarketOption\",\"Pre Installed S/W\",\"Region Code\"
\"a\",\"JRTCKXETXF\",\"OnDemand\",\"Hrs\",\"0.4\",\"USD\",\"Compute Instance\",\"AmazonEC2\",\"EU (London)\",\"AWS Region\",\"r7g.xlarge\",\"Shared\",\"Linux\",\"No License required\",\"EUW2-BoxUsage:r7g.xlarge\",\"RunInstances\",\"NA\",\"Used\",\"OnDemand\",\"NA\",\"eu-west-2\"
\"b\",\"JRTCKXETXF\",\"OnDemand\",\"Hrs\",\"0.4\",\"USD\",\"Compute Instance\",\"AmazonEC2\",\"EU (London)\",\"AWS Region\",\"r7g.xlarge\",\"Shared\",\"Linux\",\"No License required\",\"EUW2-BoxUsage:r7g.xlarge\",\"RunInstances\",\"NA\",\"Used\",\"OnDemand\",\"NA\",\"eu-west-2\"
"""
        with self.assertRaises(PRICE.PricingError):
            PRICE.bulk_csv_hourly_price(
                io.StringIO(text), "r7g.xlarge", "EU (London)", "eu-west-2"
            )


if __name__ == "__main__":
    unittest.main()
