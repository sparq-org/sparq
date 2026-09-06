#!/usr/bin/env python3
"""[GPT-6] Guard against lost arrivals, hidden network delay, and false mutation success."""
import importlib.util
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('campaign',Path(__file__).with_name('run-campaign.py'))
campaign=importlib.util.module_from_spec(spec);spec.loader.exec_module(campaign)

class SummaryTests(unittest.TestCase):
    def summarize(self,rows):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'requests.jsonl'
            path.write_text(''.join(json.dumps(row)+'\n' for row in rows))
            return campaign.summary(path,200)

    def test_scheduled_latency_and_every_offered_failure(self):
        rows=[{'record_type':'request','sequence':0,'scheduled_us':0,'outcome':'ok','http_latency_us':190000,'scheduled_latency_us':210000,'queue_us':10},
              {'record_type':'request','sequence':1,'scheduled_us':1000,'outcome':'client-admission-drop','scheduled_latency_us':100},
              {'record_type':'load-complete','offered':2}]
        result=self.summarize(rows)
        self.assertTrue(result['scheduled_coverage_complete'])
        self.assertEqual(result['within_local_deadline'],0)
        self.assertEqual(result['offered'],2)
        self.assertEqual(result['client_dropped'],1)
        self.assertEqual(result['p95_http_us'],190000)
        self.assertEqual(result['p95_scheduled_us'],210000)
        rows[1]['sequence']=0
        self.assertFalse(self.summarize(rows)['scheduled_coverage_complete'])

    def test_mutation_noops_are_distinct_from_successful_changes(self):
        base={'record_type':'request','scheduled_us':0,'outcome':'ok','http_latency_us':10,'scheduled_latency_us':20,'planned_records':1,'mutation_receipt_present':True,'mutation_matches_plan':False}
        absent={**base,'sequence':0,'operation':'modify','mutation_receipt':{'inserted_triples':0,'deleted_triples':0,'poststate_triples':[0]}}
        lost={**base,'sequence':1,'operation':'ingest','mutation_receipt':{'inserted_triples':0,'deleted_triples':0,'poststate_triples':[0]}}
        result=self.summarize([absent,lost,{'record_type':'load-complete','offered':2}])
        self.assertEqual(result['acknowledged_absent_record_modification_noops'],1)
        self.assertEqual(result['unexpected_mutation_count_mismatches'],1)
        self.assertEqual(result['acknowledged_net_records'],0)

    def test_acknowledged_data_and_policy_receipts_must_exist_in_journal(self):
        with tempfile.TemporaryDirectory() as directory:
            raw=Path(directory)/'raw.jsonl';audit=Path(directory)/'audit.jsonl'
            receipt={'id':'1-0','policy_triple_delta':-1}
            raw.write_text(json.dumps({'record_type':'request','sequence':0,'mutation_id':'1-0','pod':0,'operation':'policy-attempt','outcome':'ok','mutation_receipt':receipt})+'\n')
            audit.write_text('')
            self.assertFalse(campaign.reconcile(raw,audit)['passed'])
            audit.write_text(json.dumps({'record_type':'committed-mutation','pod':0,'receipt':receipt})+'\n')
            self.assertTrue(campaign.reconcile(raw,audit)['passed'])
            row=json.loads(raw.read_text());row.pop('mutation_receipt');raw.write_text(json.dumps(row)+'\n')
            self.assertFalse(campaign.reconcile(raw,audit)['passed'])
            row['outcome']='transport-error';raw.write_text(json.dumps(row)+'\n')
            resolved=campaign.reconcile(raw,audit)
            self.assertTrue(resolved['passed']);self.assertEqual(resolved['unknown_outcomes_resolved'],[{'id':'1-0','committed':True}])

    def test_representatives_cover_late_heavy_class_and_deterministic_ties(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'inventory.jsonl'
            rows=[{'pod_id':0,'bytes':100,'intensity_numerator':1,'intensity_denominator':2},
                  {'pod_id':1,'bytes':110,'intensity_numerator':1,'intensity_denominator':2},
                  {'pod_id':2,'bytes':110,'intensity_numerator':1,'intensity_denominator':2},
                  {'pod_id':999,'bytes':2000,'intensity_numerator':20,'intensity_denominator':1},
                  {'pod_id':1000,'bytes':3000,'intensity_numerator':20,'intensity_denominator':1}]
            path.write_text(''.join(json.dumps(row)+'\n' for row in rows))
            self.assertEqual([row['pod'] for row in campaign.representatives(path)],[0,1,999,1000])

    def test_stopping_owned_wrapper_also_kills_term_ignoring_child(self):
        with tempfile.TemporaryDirectory() as directory:
            ready=Path(directory)/'ready';heartbeat=Path(directory)/'heartbeat'
            child="import signal,sys,time;from pathlib import Path;signal.signal(signal.SIGTERM,signal.SIG_IGN);Path(sys.argv[1]).write_text('ready');\nwhile True: Path(sys.argv[2]).write_text(str(time.monotonic_ns()));time.sleep(.01)"
            wrapper="import subprocess,sys,time;subprocess.Popen([sys.executable,'-c',"+repr(child)+",sys.argv[1],sys.argv[2]]);time.sleep(60)"
            process=subprocess.Popen([sys.executable,'-c',wrapper,str(ready),str(heartbeat)],start_new_session=True)
            try:
                deadline=time.monotonic()+5
                while not heartbeat.exists() and time.monotonic()<deadline:time.sleep(.01)
                self.assertTrue(heartbeat.exists(),'child did not start')
                campaign.stop_process_group(process,grace_seconds=.1)
                self.assertIsNotNone(process.poll())
                time.sleep(.1);last=heartbeat.read_text();time.sleep(.15)
                self.assertEqual(heartbeat.read_text(),last,'wrapper child kept writing after cancellation')
            finally:
                try:os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                process.wait(timeout=5)

if __name__=='__main__':unittest.main()
