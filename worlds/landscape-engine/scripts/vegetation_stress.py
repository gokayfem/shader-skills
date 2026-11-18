#!/usr/bin/env python3
"""Run two prebuilt vegetation_stress binaries sequentially at identical quality.

Example:
  python scripts/vegetation_stress.py --baseline /tmp/before --candidate /tmp/after \
      --output target/vegetation-comparison --stress

Build/copy each binary before starting; compiling or running another GPU workload
during measurements contaminates CPU and GPU timings. No third-party packages.
"""

import argparse
import csv
import subprocess
from pathlib import Path


CASES = (
    ("128-ground", 128, 1024, "ground"),
    ("128-walk", 128, 1024, "walk"),
    ("512-overview", 512, 16384, "overview"),
)
STRESS = (
    ("1024-overview", 1024, 65536, "overview"),
    ("128-dense", 128, 16384, "ground"),
    ("512-grass", 512, 0, "ground"),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--frames", type=int, default=240)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--stress", action="store_true")
    args = parser.parse_args()
    if args.frames < 1 or args.repeats < 1:
        parser.error("frames and repeats must be positive")
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    for binary in binaries.values():
        if not binary.is_file():
            parser.error(f"binary does not exist: {binary}")
    args.output.mkdir(parents=True, exist_ok=True)
    results = []
    failures = []
    cases = CASES + (STRESS if args.stress else ())
    for repeat in range(1, args.repeats + 1):
        for name, side, trees, mode in cases:
            # Balance startup/thermal ordering across repetitions.
            versions = ("baseline", "candidate") if repeat % 2 else ("candidate", "baseline")
            for version in versions:
                run = f"{version}-{name}-r{repeat}"
                output = (args.output / run).resolve()
                # Never silently replace a previous measurement.
                output.mkdir()
                with (output / "summary.csv").open("w") as summary, (output / "stderr.log").open("w") as log:
                    result = subprocess.run(
                        [str(binaries[version]), str(side), str(trees), str(args.frames), mode, str(output)],
                        stdout=summary, stderr=log, check=False,
                    )
                print(f"{run}: exit {result.returncode}", flush=True)
                if result.returncode:
                    failures.append(run)
                with (output / "summary.csv").open(newline="") as summary:
                    for row in csv.DictReader(summary):
                        results.append({"run": run, "exit_code": result.returncode, **row})
    if results:
        with (args.output / "comparison.csv").open("w", newline="") as output:
            writer = csv.DictWriter(output, fieldnames=list(results[0]))
            writer.writeheader()
            writer.writerows(results)
    if failures:
        raise SystemExit("Failed or overflowed runs (inspect stderr.log): " + ", ".join(failures))


if __name__ == "__main__":
    main()
