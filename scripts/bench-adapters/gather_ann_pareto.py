#!/usr/bin/env python3
# sq-hmd7l.19: ANN Pareto gather script — SIFT1M + GloVe-100-angular
# Measures: recall-controlled QPS, tail latency (p99), build time, peak RSS, index bytes
# Engines: hnswlib (primary HNSW peer), FAISS (IndexFlatL2 exact oracle + IVFFlat + IVFSq8)
# This work-box run is NON-CANONICAL (aarch64 EC2, noisy) — flagged in the output.
import json
import os
import resource
import struct
import sys
import time
import traceback

sys.path.insert(0, '/home/ubuntu/sparq-wt/ann-pareto/scripts/bench-adapters')
import vector_lib_adapter as vla

import numpy as np


def peak_rss_mb():
    """Peak RSS in MB (Linux: getrusage MAXRSS is in kilobytes)."""
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024.0


def query_latencies(index_query_fn, queries, k, n_reps=3):
    """Run multiple repetitions, return per-query latencies (us) across all reps."""
    all_us = []
    for _ in range(n_reps):
        t0 = time.perf_counter()
        index_query_fn(queries, k)
        elapsed_us = (time.perf_counter() - t0) * 1e6
        all_us.append(elapsed_us / max(1, len(queries)))
    return all_us


def p99(latencies_us):
    """p99 of a list of per-query latencies."""
    import statistics
    s = sorted(latencies_us)
    idx = int(0.99 * len(s))
    return s[min(idx, len(s) - 1)]


def run_hnswlib_sweep_full(data, queries, k, space, ef_values, ground_truth, m=16, ef_construction=200, n_reps=3):
    """Build hnswlib index, measure build time + peak RSS, sweep ef values.
    Returns: build_s, build_peak_rss_mb, index_bytes, list of ef-point dicts."""
    import hnswlib

    rss_before = peak_rss_mb()
    t_build = time.perf_counter()
    idx = hnswlib.Index(space=space, dim=data.shape[1])
    idx.init_index(max_elements=len(data), ef_construction=ef_construction, M=m)
    idx.add_items(data, list(range(len(data))))
    build_s = time.perf_counter() - t_build
    rss_after = peak_rss_mb()
    build_rss_delta_mb = rss_after - rss_before

    # Persist to tmp to measure index bytes
    idx_path = '/tmp/ann/_hnswlib_tmp.bin'
    idx.save_index(idx_path)
    persisted_bytes = os.path.getsize(idx_path)
    os.unlink(idx_path)

    points = []
    for ef in ef_values:
        idx.set_ef(int(ef))
        # Collect latencies across reps
        latencies = []
        for _ in range(n_reps):
            t0 = time.perf_counter()
            labels, _ = idx.knn_query(queries, k=k)
            elapsed_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
            latencies.append(elapsed_us)
        # Score vs ground truth
        approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(labels)}
        recall, _n = vla.recall_at_k(approx, ground_truth, k)
        mean_us = sum(latencies) / len(latencies)
        qps = vla.qps_from_query_us(mean_us)
        points.append({
            'ef': ef,
            'recall': recall,
            'deficit_milli': vla.recall_deficit_milli(recall),
            'mean_query_us': mean_us,
            'p99_query_us': p99(latencies),
            'qps': qps,
        })
        print(f"  hnswlib ef={ef}: recall@{k}={recall:.4f} deficit={vla.recall_deficit_milli(recall)} mean_us={mean_us:.1f} qps={qps:.1f} p99_us={p99(latencies):.1f}", flush=True)

    return {
        'engine': 'hnswlib',
        'config': {'M': m, 'ef_construction': ef_construction},
        'build_s': build_s,
        'build_rss_delta_mb': build_rss_delta_mb,
        'persisted_bytes': persisted_bytes,
        'points': points,
    }


def run_faiss_exact(data, queries, k, n_reps=3):
    """FAISS IndexFlatL2 brute-force exact search — oracle + baseline."""
    import faiss

    rss_before = peak_rss_mb()
    t_build = time.perf_counter()
    idx = faiss.IndexFlatL2(data.shape[1])
    idx.add(data)
    build_s = time.perf_counter() - t_build
    rss_after = peak_rss_mb()

    latencies = []
    all_labels = None
    for _ in range(n_reps):
        t0 = time.perf_counter()
        _, labels = idx.search(queries, k)
        elapsed_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
        latencies.append(elapsed_us)
        all_labels = labels

    exact_gt = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(all_labels)}
    mean_us = sum(latencies) / len(latencies)
    print(f"  faiss-flat: build={build_s:.2f}s mean_us={mean_us:.1f} qps={vla.qps_from_query_us(mean_us):.1f}", flush=True)
    return {
        'engine': 'faiss-flat-l2',
        'build_s': build_s,
        'build_rss_delta_mb': rss_after - rss_before,
        'mean_query_us': mean_us,
        'p99_query_us': p99(latencies),
        'qps': vla.qps_from_query_us(mean_us),
        'exact_gt': exact_gt,
    }


def run_faiss_ivfflat_sweep(data, queries, k, nlist, nprobe_values, ground_truth, n_reps=3):
    """FAISS IVFFlat: cluster-indexed ANN — sweep nprobe values."""
    import faiss

    rss_before = peak_rss_mb()
    t_build = time.perf_counter()
    quantizer = faiss.IndexFlatL2(data.shape[1])
    idx = faiss.IndexIVFFlat(quantizer, data.shape[1], nlist)
    idx.train(data)
    idx.add(data)
    build_s = time.perf_counter() - t_build
    rss_after = peak_rss_mb()

    points = []
    for nprobe in nprobe_values:
        idx.nprobe = nprobe
        latencies = []
        all_labels = None
        for _ in range(n_reps):
            t0 = time.perf_counter()
            _, labels = idx.search(queries, k)
            elapsed_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
            latencies.append(elapsed_us)
            all_labels = labels
        approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(all_labels)}
        recall, _ = vla.recall_at_k(approx, ground_truth, k)
        mean_us = sum(latencies) / len(latencies)
        qps = vla.qps_from_query_us(mean_us)
        points.append({
            'nprobe': nprobe,
            'recall': recall,
            'deficit_milli': vla.recall_deficit_milli(recall),
            'mean_query_us': mean_us,
            'p99_query_us': p99(latencies),
            'qps': qps,
        })
        print(f"  faiss-ivfflat nprobe={nprobe}: recall@{k}={recall:.4f} deficit={vla.recall_deficit_milli(recall)} mean_us={mean_us:.1f} qps={qps:.1f}", flush=True)

    return {
        'engine': 'faiss-ivfflat',
        'config': {'nlist': nlist},
        'build_s': build_s,
        'build_rss_delta_mb': rss_after - rss_before,
        'points': points,
    }


def run_faiss_ivfsq8_sweep(data, queries, k, nlist, nprobe_values, ground_truth, n_reps=3):
    """FAISS IVFScalarQuantizer (SQ8): 8-bit scalar quantization — sweep nprobe."""
    import faiss

    rss_before = peak_rss_mb()
    t_build = time.perf_counter()
    quantizer = faiss.IndexFlatL2(data.shape[1])
    idx = faiss.IndexIVFScalarQuantizer(quantizer, data.shape[1], nlist, faiss.ScalarQuantizer.QT_8bit)
    idx.train(data)
    idx.add(data)
    build_s = time.perf_counter() - t_build
    rss_after = peak_rss_mb()

    points = []
    for nprobe in nprobe_values:
        idx.nprobe = nprobe
        latencies = []
        all_labels = None
        for _ in range(n_reps):
            t0 = time.perf_counter()
            _, labels = idx.search(queries, k)
            elapsed_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
            latencies.append(elapsed_us)
            all_labels = labels
        approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(all_labels)}
        recall, _ = vla.recall_at_k(approx, ground_truth, k)
        mean_us = sum(latencies) / len(latencies)
        qps = vla.qps_from_query_us(mean_us)
        points.append({
            'nprobe': nprobe,
            'recall': recall,
            'deficit_milli': vla.recall_deficit_milli(recall),
            'mean_query_us': mean_us,
            'p99_query_us': p99(latencies),
            'qps': qps,
        })
        print(f"  faiss-ivfsq8 nprobe={nprobe}: recall@{k}={recall:.4f} deficit={vla.recall_deficit_milli(recall)} mean_us={mean_us:.1f} qps={qps:.1f}", flush=True)

    return {
        'engine': 'faiss-ivfsq8',
        'config': {'nlist': nlist},
        'build_s': build_s,
        'build_rss_delta_mb': rss_after - rss_before,
        'points': points,
    }


def pareto_at_targets(points, targets, recall_key='recall', qps_key='qps'):
    """For each target recall, find the best QPS (matched_recall_qps)."""
    pts = [(p[recall_key], p[qps_key]) for p in points]
    return {str(t): vla.matched_recall_qps(pts, t) for t in targets}


if __name__ == '__main__':
    dataset = sys.argv[1] if len(sys.argv) > 1 else 'sift'
    out_file = sys.argv[2] if len(sys.argv) > 2 else '/tmp/ann/results_{}.json'.format(dataset)
    print(f"[gather] Dataset: {dataset}, output: {out_file}", flush=True)
    print(f"[gather] NON-CANONICAL work-box timing (aarch64 EC2)", flush=True)

    k = 10
    RECALL_TARGETS = [0.80, 0.90, 0.95, 0.99]
    results = {'dataset': dataset, 'k': k, 'non_canonical': True, 'engines': {}}

    if dataset in ('sift', 'sift1m', 'sift-128-euclidean'):
        root = '/tmp/ann'
        data, queries, space, gt_raw_from_loader = vla.load_dataset('sift-128-euclidean', root)
        # SIFT has precomputed ground truth from the dataset
        # gt_raw_from_loader is {qid:[str(id),...]} from the ivecs file
        ground_truth = gt_raw_from_loader  # already {str(qi): [str(i),...]}
        print(f"[gather] SIFT: {data.shape}, queries={queries.shape}", flush=True)

        # FAISS exact (oracle)
        print("[gather] FAISS exact...", flush=True)
        faiss_exact = run_faiss_exact(data, queries, k)
        results['engines']['faiss-flat-l2'] = {
            'build_s': faiss_exact['build_s'],
            'build_rss_delta_mb': faiss_exact['build_rss_delta_mb'],
            'mean_query_us': faiss_exact['mean_query_us'],
            'p99_query_us': faiss_exact['p99_query_us'],
            'qps': faiss_exact['qps'],
            'role': 'exact oracle',
        }

        # hnswlib sweep
        print("[gather] hnswlib sweep...", flush=True)
        hnsw = run_hnswlib_sweep_full(data, queries, k, 'l2', [16, 32, 64, 128, 256, 512], ground_truth)
        results['engines']['hnswlib'] = hnsw
        results['engines']['hnswlib']['matched_recall_qps'] = pareto_at_targets(hnsw['points'], RECALL_TARGETS)

        # FAISS IVFFlat sweep (nlist=1024 for 1M vectors)
        print("[gather] FAISS IVFFlat sweep...", flush=True)
        ivfflat = run_faiss_ivfflat_sweep(data, queries, k, 1024, [1, 4, 8, 16, 32, 64, 128, 256], ground_truth)
        results['engines']['faiss-ivfflat'] = ivfflat
        results['engines']['faiss-ivfflat']['matched_recall_qps'] = pareto_at_targets(ivfflat['points'], RECALL_TARGETS)

        # FAISS IVFSQ8 sweep
        print("[gather] FAISS IVFSQ8 sweep...", flush=True)
        ivfsq8 = run_faiss_ivfsq8_sweep(data, queries, k, 1024, [1, 4, 8, 16, 32, 64, 128, 256], ground_truth)
        results['engines']['faiss-ivfsq8'] = ivfsq8
        results['engines']['faiss-ivfsq8']['matched_recall_qps'] = pareto_at_targets(ivfsq8['points'], RECALL_TARGETS)

    elif dataset in ('glove', 'glove100', 'glove-100-angular'):
        root = '/tmp/ann'
        data, queries, space, ground_truth = vla.load_dataset('glove-100-angular', root)
        print(f"[gather] GloVe: {data.shape}, queries={queries.shape}, space={space}", flush=True)

        # FAISS for angular needs IP, not L2 (cosine = normalize + inner product)
        # Normalize vectors for cosine similarity (FAISS IndexFlatIP on normalized = cosine)
        data_norm = data / (np.linalg.norm(data, axis=1, keepdims=True) + 1e-10)
        queries_norm = queries / (np.linalg.norm(queries, axis=1, keepdims=True) + 1e-10)

        # hnswlib sweep (cosine space - hnswlib handles normalization internally)
        print("[gather] hnswlib sweep...", flush=True)
        hnsw = run_hnswlib_sweep_full(data, queries, k, 'cosine', [16, 32, 64, 128, 256, 512], ground_truth)
        results['engines']['hnswlib'] = hnsw
        results['engines']['hnswlib']['matched_recall_qps'] = pareto_at_targets(hnsw['points'], RECALL_TARGETS)

        # FAISS exact oracle (inner product on normalized)
        print("[gather] FAISS exact (IP on normalized)...", flush=True)
        import faiss
        idx_flat = faiss.IndexFlatIP(data.shape[1])
        idx_flat.add(data_norm)
        latencies_flat = []
        all_flat_labels = None
        for _ in range(3):
            t0 = time.perf_counter()
            _, labels = idx_flat.search(queries_norm, k)
            elapsed_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
            latencies_flat.append(elapsed_us)
            all_flat_labels = labels
        mean_us_flat = sum(latencies_flat) / len(latencies_flat)
        print(f"  faiss-flat-ip: mean_us={mean_us_flat:.1f} qps={vla.qps_from_query_us(mean_us_flat):.1f}", flush=True)
        results['engines']['faiss-flat-ip'] = {
            'build_s': 0.0,  # add is instant for flat
            'mean_query_us': mean_us_flat,
            'p99_query_us': p99(latencies_flat),
            'qps': vla.qps_from_query_us(mean_us_flat),
            'role': 'exact oracle (normalized inner product)',
        }

        # FAISS IVFFlat on normalized (inner product)
        print("[gather] FAISS IVFFlat sweep (IP)...", flush=True)
        quantizer_ip = faiss.IndexFlatIP(data.shape[1])
        idx_ivf = faiss.IndexIVFFlat(quantizer_ip, data.shape[1], 1024, faiss.METRIC_INNER_PRODUCT)
        idx_ivf.train(data_norm)
        idx_ivf.add(data_norm)
        ivfflat_pts = []
        for nprobe in [1, 4, 8, 16, 32, 64, 128, 256]:
            idx_ivf.nprobe = nprobe
            lats = []
            lbls = None
            for _ in range(3):
                t0 = time.perf_counter()
                _, labels = idx_ivf.search(queries_norm, k)
                lats.append((time.perf_counter() - t0) * 1e6 / len(queries))
                lbls = labels
            approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(lbls)}
            recall, _ = vla.recall_at_k(approx, ground_truth, k)
            mean_us = sum(lats)/len(lats)
            ivfflat_pts.append({'nprobe': nprobe, 'recall': recall, 'deficit_milli': vla.recall_deficit_milli(recall),
                                  'mean_query_us': mean_us, 'p99_query_us': p99(lats), 'qps': vla.qps_from_query_us(mean_us)})
            print(f"  faiss-ivfflat-ip nprobe={nprobe}: recall@{k}={recall:.4f} qps={vla.qps_from_query_us(mean_us):.1f}", flush=True)
        results['engines']['faiss-ivfflat-ip'] = {'engine': 'faiss-ivfflat-ip', 'config': {'nlist': 1024}, 'points': ivfflat_pts,
            'matched_recall_qps': pareto_at_targets(ivfflat_pts, RECALL_TARGETS)}

    else:
        print(f"Unknown dataset: {dataset}", file=sys.stderr)
        sys.exit(1)

    with open(out_file, 'w') as f:
        json.dump(results, f, indent=2)
    print(f"[gather] Results written to {out_file}", flush=True)
