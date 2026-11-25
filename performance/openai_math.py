"""Time tift's read-only check of the OpenAI math manuscript sources (Python 3.10+)."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request


REVISION = "adc7f1241b42e322a6451854ab7e4b4c146bf78a"
UPSTREAM = "https://github.com/openai/math.git"
EXCLUDED_PAPERS = frozenset({
    "The-Strong-Hyperkahler-SYZ-Conjecture-September-23-2026",
})


def prepare(checkout, work):
    if checkout is None:
        checkout = work / "upstream"
        subprocess.run(["git", "init", "-q", checkout], check=True)
        git = ["git", "-C", checkout]
        subprocess.run([*git, "remote", "add", "origin", UPSTREAM], check=True)
        subprocess.run([*git, "config", "remote.origin.promisor", "true"], check=True)
        subprocess.run([*git, "config", "remote.origin.partialclonefilter", "blob:none"], check=True)
        proxy = urllib.request.getproxies().get("https")
        settings = ["-c", f"http.proxy={proxy}"] if proxy else []
        subprocess.run([*git, *settings, "fetch", "--depth=1", "--filter=blob:none", "origin", REVISION], check=True)
        subprocess.run([*git, "sparse-checkout", "set", "--no-cone", "/preprints/**/*.tex"], check=True)
        subprocess.run([*git, *settings, "-c", "core.longpaths=true", "-c", "core.autocrlf=false",
                        "checkout", "-q", "--detach", REVISION], check=True)
    git = ["git", "-C", checkout]
    actual = subprocess.check_output([*git, "rev-parse", "HEAD"], text=True).strip()
    if actual != REVISION:
        raise ValueError(f"Expected pinned OpenAI math revision {REVISION}, got {actual}")
    entries = subprocess.check_output([*git, "ls-tree", "-rz", "-r", REVISION, "preprints"])
    root = work / "papers"
    source_counts = Counter()
    sample_counts = Counter()
    source_bytes = 0
    sample_bytes = 0
    with subprocess.Popen([*git, "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE) as blobs:
        try:
            for entry in entries.split(b"\0"):
                if not entry.endswith(b".tex"):
                    continue
                meta, name = entry.split(b"\t", 1)
                mode, kind, oid = meta.split()
                parts = Path(name.decode("utf-8")).parts
                if mode not in (b"100644", b"100755") or kind != b"blob" or parts[0] != "preprints":
                    raise ValueError(f"Unexpected source: {name!r}")
                blobs.stdin.write(oid + b"\n")
                blobs.stdin.flush()
                actual_oid, kind, size = blobs.stdout.readline().split()
                data = blobs.stdout.read(int(size))
                if actual_oid != oid or kind != b"blob" or len(data) != int(size) or blobs.stdout.read(1) != b"\n":
                    raise ValueError(f"Incomplete blob: {name!r}")
                source_counts[parts[1]] += 1
                source_bytes += len(data)
                if parts[1] in EXCLUDED_PAPERS:
                    continue
                dest = root.joinpath(*parts[1:])
                dest.parent.mkdir(parents=True, exist_ok=True)
                with dest.open("xb") as output:
                    output.write(data)
                sample_counts[parts[1]] += 1
                sample_bytes += len(data)
        finally:
            blobs.stdin.close()
    if blobs.returncode or (len(source_counts), sum(source_counts.values()), source_bytes) != (722, 7729, 127982216):
        raise ValueError("Corpus does not match the pinned 722 manuscripts")
    if not EXCLUDED_PAPERS.issubset(source_counts):
        raise ValueError("An excluded manuscript is missing from the pinned corpus")
    excluded_files = sum(source_counts[paper] for paper in EXCLUDED_PAPERS)
    if len(sample_counts) != len(source_counts) - len(EXCLUDED_PAPERS):
        raise ValueError("The compilable sample does not match its exclusion list")
    return (
        root,
        len(sample_counts),
        sum(sample_counts.values()),
        sample_bytes,
        excluded_files,
        source_bytes - sample_bytes,
    )


def check(binary, papers, workers, expected, target="."):
    process = subprocess.run([binary, "--check", "--no-cache", "--no-config", "--threads",
                              str(workers), str(target)], cwd=papers, capture_output=True)
    messages = process.stderr.decode("utf-8", "replace")
    summary = re.search(r"(?m)^(\d+) file\(s\) would change; (\d+) checked\.\r?$", messages)
    diagnostics = re.findall(r"(?m)^(.+?): error: ", messages)
    if not summary or int(summary[2]) != expected or process.returncode not in (0, 1, 2):
        raise RuntimeError(f"Incomplete corpus run: {messages[-1200:]}")
    if process.returncode == 2 and not diagnostics:
        raise RuntimeError(f"tift exited with {process.returncode}: {messages[-1200:]}")
    return len(set(diagnostics)), len(diagnostics), int(summary[1])


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("expected a positive integer")
    return number


def time_individual_files(binary, papers, count, runs):
    paths = [path.relative_to(papers) for path in papers.rglob("*.tex")]
    if count > len(paths):
        raise ValueError(f"Requested {count} files from a corpus of {len(paths)}")
    # Ranking the relative paths by hash keeps the sample stable across platforms.
    paths.sort(key=lambda path: hashlib.sha256(path.as_posix().encode()).digest())
    timings = {path: [] for path in paths[:count]}
    files_with_errors = set()
    for i in range(runs):
        for checked, (path, samples) in enumerate(timings.items(), 1):
            start = time.perf_counter()
            errors, _, _ = check(binary, papers, 1, 1, path)
            samples.append((time.perf_counter() - start) * 1000)
            if errors:
                files_with_errors.add(path)
            if checked % 1000 == 0:
                print(f"Pass {i + 1}/{runs}: {checked}/{count} files", file=sys.stderr, flush=True)
        print(f"Single-file pass {i + 1}: {count} checks", file=sys.stderr, flush=True)
    return {path.as_posix(): samples for path, samples in timings.items()}, len(files_with_errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tift", required=True, help="release executable")
    parser.add_argument("--checkout", type=Path, help="reuse local openai/math Git objects")
    parser.add_argument("--temp-dir", type=Path, help="parent directory for temporary corpus")
    parser.add_argument("--workers", type=positive, default=8, help="worker threads for the whole-corpus check")
    individual = parser.add_mutually_exclusive_group()
    individual.add_argument("--sample-files", type=positive, help="time a fixed sample of files individually, one thread per invocation")
    individual.add_argument("--all-files", action="store_true", help="time every included .tex file individually, one thread per invocation")
    parser.add_argument("--runs", type=positive, default=3)
    args = parser.parse_args()
    binary = Path(shutil.which(args.tift) or args.tift).resolve(strict=True)
    if args.temp_dir is not None:
        args.temp_dir.mkdir(parents=True, exist_ok=True)
    result = {"revision": REVISION, "version": subprocess.check_output([binary, "--version"], text=True).strip(),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "platform": platform.platform(), "processor": platform.processor(),
              "workers": 1 if args.sample_files or args.all_files else args.workers}
    with tempfile.TemporaryDirectory(dir=args.temp_dir, prefix="tift-math-") as work:
        papers, paper_count, file_count, size, excluded_files, excluded_bytes = prepare(
            args.checkout, Path(work)
        )
        result.update(
            papers=paper_count,
            files=file_count,
            bytes=size,
            excluded_papers=len(EXCLUDED_PAPERS),
            excluded_files=excluded_files,
            excluded_bytes=excluded_bytes,
        )
        if args.sample_files or args.all_files:
            result["runs_per_file"] = args.runs
            times, errors = time_individual_files(binary, papers, args.sample_files or file_count, args.runs)
            result["elapsed_ms_by_file"] = times
            result["median_ms_by_file"] = {path: statistics.median(samples) for path, samples in times.items()}
            result["files_with_errors"] = errors
        else:
            result["runs"] = []
            for i in range(args.runs):
                start = time.perf_counter()
                outcome = check(binary, papers, args.workers, result["files"])
                elapsed = time.perf_counter() - start
                result["runs"].append({"seconds": elapsed, "files_with_errors": outcome[0],
                                       "diagnostics": outcome[1], "would_change": outcome[2]})
                print(f"Run {i + 1}: {elapsed:.3f}s, {outcome[0]} files with errors", file=sys.stderr, flush=True)
            result["median_seconds"] = statistics.median(r["seconds"] for r in result["runs"])
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
