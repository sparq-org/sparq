"""[GPT-6] Validate retained-stock rates and projection versus inventory scope."""
from decimal import Decimal
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("paper_inputs", Path(__file__).with_name("derive-paper-inputs.py"))
deriver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deriver)


class StockRateTests(unittest.TestCase):
    def test_stock_uses_actual_horizon_and_balanced_ingestion_expiry(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            calibration = root / "calibration.json"
            calibration.write_text(json.dumps({"history_months": {"value": 60}}))
            inventory = root / "inventory.jsonl"
            inventory.write_text(json.dumps({"pod_id": 0, "records_by_service": {"communication": 60, "contacts": 365}}) + "\n")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"pods": 1, "config": {"history_months": 2}}))
            execution = {"retention_days_per_month": 30, "contacts_modifications_per_retained_record_year": 1,
                         "days_per_year": 365, "mutation_batches": [{"service": "communication", "ingest_records": 1,
                         "expire_records": 1, "modify_records": 1, "modifications_per_ingested_record": Decimal("0.5")}]}
            result = deriver.stock_based_writes({"execution_v2": execution}, calibration, inventory, manifest)
            self.assertEqual(result["retention_days"], 60)
            self.assertEqual(result["per_hosted_person_daily_content_requests"],
                             {"ingest": 1, "expire": 1, "modify": Decimal("1.5")})
            self.assertEqual(result["content_write_requests_per_hosted_day"], Decimal("3.5"))
            self.assertIn("observed retained inventory", result["basis"])
            manifest.write_text(json.dumps({"pods": 2, "config": {"history_months": 2}}))
            with self.assertRaises(ValueError):
                deriver.stock_based_writes({"execution_v2": execution}, calibration, inventory, manifest)

    def test_population_projection_does_not_change_request_composition(self):
        workload = {"offered_multiplier_scenarios": {"mean": 1, "busy": 4}}
        rates = deriver.population_rates({"query": Decimal("43.2"), "content-write": Decimal("21.6")}, workload)
        million = next(x for x in rates if x["pods"] == 1000000 and x["scenario"] == "busy")
        self.assertEqual(million["components_rps"], {"query": Decimal(2000), "content-write": Decimal(1000)})
        self.assertEqual(million["total_rps"], 3000)


if __name__ == "__main__":
    unittest.main()
