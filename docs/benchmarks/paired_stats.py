"""Paired statistics for two configurations in an asr_bench JSON file.

Usage:
    python paired_stats.py asr-final-raw.json <baseline config> <candidate config>

Both configurations must have been run on the same corpus in the same file. Each clip's
latency is its median across passes. Intervals are 95 % percentile bootstrap intervals,
resampling clips with replacement (10,000 resamples, fixed seed), so they account for the
fact that errors within one clip are correlated. They do not account for the corpus having
only a few speakers.
"""

import json
import random
import statistics
import sys


def load(path, name):
    data = json.load(open(path, encoding="utf-8"))
    for config in data["configs"]:
        if config["config"] == name:
            return {clip["id"]: clip for clip in config["perUtterance"]}
    raise SystemExit(f"config not found: {name}")


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[round(fraction * (len(ordered) - 1))]


def interval(samples):
    return percentile(samples, 0.025), percentile(samples, 0.975)


def main():
    path, baseline_name, candidate_name = sys.argv[1:4]
    baseline = load(path, baseline_name)
    candidate = load(path, candidate_name)
    ids = sorted(set(baseline) & set(candidate))

    rows = []
    for clip_id in ids:
        b, c = baseline[clip_id], candidate[clip_id]
        rows.append(
            (
                statistics.median(b["latenciesMs"]),
                statistics.median(c["latenciesMs"]),
                b["edits"],
                c["edits"],
                b["referenceWords"],
            )
        )

    def summarise(sample):
        base_latency = [row[0] for row in sample]
        cand_latency = [row[1] for row in sample]
        words = sum(row[4] for row in sample)
        return {
            "base_p50": statistics.median(base_latency),
            "cand_p50": statistics.median(cand_latency),
            "base_p95": percentile(base_latency, 0.95),
            "cand_p95": percentile(cand_latency, 0.95),
            "base_wer": 100.0 * sum(row[2] for row in sample) / words,
            "cand_wer": 100.0 * sum(row[3] for row in sample) / words,
        }

    point = summarise(rows)
    generator = random.Random(20261122)
    p50_change, p95_change, wer_delta = [], [], []
    for _ in range(10_000):
        sample = [rows[generator.randrange(len(rows))] for _ in rows]
        stats = summarise(sample)
        p50_change.append(100.0 * (stats["cand_p50"] / stats["base_p50"] - 1.0))
        p95_change.append(100.0 * (stats["cand_p95"] / stats["base_p95"] - 1.0))
        wer_delta.append(stats["cand_wer"] - stats["base_wer"])

    faster = sum(1 for row in rows if row[1] < row[0])
    print(f"clips: {len(rows)}; passes: {len(baseline[ids[0]]['latenciesMs'])}")
    print(f"baseline : {baseline_name}")
    print(f"candidate: {candidate_name}")
    print(
        f"median latency  {point['base_p50']:.0f} -> {point['cand_p50']:.0f} ms, "
        f"change {100 * (point['cand_p50'] / point['base_p50'] - 1):+.1f} % "
        f"(95 % interval {interval(p50_change)[0]:+.1f} to {interval(p50_change)[1]:+.1f} %)"
    )
    print(
        f"p95 latency     {point['base_p95']:.0f} -> {point['cand_p95']:.0f} ms, "
        f"change {100 * (point['cand_p95'] / point['base_p95'] - 1):+.1f} % "
        f"(95 % interval {interval(p95_change)[0]:+.1f} to {interval(p95_change)[1]:+.1f} %)"
    )
    print(f"candidate faster on {faster} of {len(rows)} clips")
    print(
        f"WER             {point['base_wer']:.2f} -> {point['cand_wer']:.2f} %, "
        f"difference {point['cand_wer'] - point['base_wer']:+.2f} points "
        f"(95 % interval {interval(wer_delta)[0]:+.2f} to {interval(wer_delta)[1]:+.2f})"
    )


if __name__ == "__main__":
    main()
