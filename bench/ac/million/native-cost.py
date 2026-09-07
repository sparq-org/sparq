#!/usr/bin/env python3
"""[GPT-6] Current-price reservation for the ready-only native evaluation host."""
from decimal import Decimal
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location('build_cost', Path(__file__).with_name('build-cost.py'))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


def reservation(path, prior):
    with Path(path).open(newline='') as stream:
        hourly, evidence = build.price.bulk_csv_hourly_price(stream, 'r7gd.12xlarge', 'EU (London)', 'eu-west-2')
    storage, sku = build.gp3_price(path)
    if not prior.is_finite():
        raise ValueError("nonfinite prior allocation")
    hours = Decimal('12.25')
    ancillary = hours * (Decimal(80) * storage / Decimal(672) + Decimal('0.005'))
    result = build.price.quote_at_hourly_price(hourly, hours, ancillary, prior, Decimal(100), Decimal(100))
    result.update(evidence)
    result.update(scope='native-evaluation-host-reservation; campaign not yet dispatched', invoice_verified=False,
                  gp3_gib=80, gp3_monthly_usd=str(storage), gp3_price_sku=sku,
                  storage_month_hours_bound=672, watchdog_seconds=43200,
                  ipv4_hourly_usd='0.005', ipv4_price_source='https://aws.amazon.com/vpc/pricing/',
                  billing_cushion_seconds=900, prior_includes_existing_reserve=True,
                  instance_store_included=True, host_physical_vcpus=48, host_memory_gib=384)
    return result


if __name__ == '__main__':
    print(json.dumps(reservation(sys.argv[1], Decimal(sys.argv[2])), indent=2, sort_keys=True))
