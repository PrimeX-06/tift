#!/usr/bin/env python3
"""Benchmark tift on 14 authored works from 13 pinned public repositories (Python 3.10+).

verify checks the corpus without invoking tift. formal measures seven passes after
one warm-up; local defaults to five measured passes and accepts --runs/--threads.
Each pass launches one read-only process per work, serially, in a deterministic
order. Times are wall-clock milliseconds including CLI startup. Per-work ranges
and medians exclude preparation and warm-up. The corpus median is taken over the
measured per-pass sums of CLI times. JSON goes to stdout or a new --report file.
"""

import argparse
from datetime import datetime, timezone
import hashlib
from http.client import HTTPException
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from urllib.request import urlopen
import zipfile


REPOS = {
    "dfm/emcee": ("ff3dc94006167397e4ee2814fea9cb9532caeecc", "5f265ae0f791c12e2b3b0b5bd4cfaf953d7b157f8684ccb4f74cf5f48df95dd8"),
    "jakevdp/PracticalLombScargle": ("5f4ca27deefa7d4edc68da3c92d34f484c5bc98e", "a5832439e617e3e28139271fae4abd15e8eade953b92a91584c19a4099517e30"),
    "hadley/tidy-data": ("5623c7a302977e8c88f623a08556d5c08e940d8c", "3df5069726c7515f66976db18bcc0953944aacd52cc33f03fb6e9285434a1737"),
    "AllenDowney/ThinkPython2": ("cf3d99c002933e1829e4aa89109d2e1dae809de0", "6ad724621bdc5eb3011b0e95a977b7033265f77b003e0316a48e6d0e8bfd9152"),
    "CTeX-org/lshort-zh-cn": ("3b6fbb2f88f81eb12a405db0988e53702cac10ee", "877f1436134ffa0eb5e5926850307cefccc85d704412e20bdbc2a799b575cad3"),
    "langsci/91": ("bca083ed2f5f30dd4c2d6587366e8e0f649285c3", "5681dce897a1f002a9775970b25832514bb6a9b6e6659fe0b7474d386f25cc81"),
    "habi/PhD-Thesis": ("2453f652da5d90887e494d01727e8c3cc474f567", "4ba3e801f77024acf40beeb51b02d9327200569747b79aa036eec1eea023c55e"),
    "eduardszoecs/talk_webscrapingr": ("61587ea5dca920edf0d71ddfc12c9c4bda77b53c", "cf1ec9151d688a15bf51173df0bfc621e7b43f83d61694dc4678f6eb836d11fd"),
    "t-tk/upmendex-package": ("6850a15281169e1976a2e9dd20fea7fe331052c9", "0128a96bbd8a4701d44ee97fd31ef12d8719e7151f7fd26ce6009025fc7e6ee4"),
    "billryan/resume": ("eadc3955cf006254dc8f2419e971fbe3f5bab413", "b20319a963768aa2710e2c8d6fe229089c9ee2d8c6d1135b3ad23cc3ec38ebc3"),
    "mavam/stat-cookbook": ("09bd804a5251911088d55f8ce787a991b8b4ba15", "12feb3378d4a125f9bf7d565eab39c905e92ba94fbfdcc5ffb5d07e98102d1b7"),
    "langsci/25": ("59eec1d2f610ef92a57aee8dddba66df82047677", "3c621918f01567d90804039d1340608f35dd8dd76a341a5fcb9b4b8022d03e1b"),
    "cldf-datasets/wals": ("81298aa26d01fdc11b769bcc7fa87ad9e43d7ccd", "015c0b4bc01f12ac90cd98ed62746d3551ba5198a9a43d55237896c38cacb8db"),
}

RAW = {"langsci/25": "gt.bib", "cldf-datasets/wals": "cldf/sources.bib"}

WORKS = {
    "emcee": ("paper", "dfm/emcee", "document", [
        ("ms.tex", 41540, "59b74396735688662fb6b5926ed577f5d7e2347d5ab7beceb54521663c1abd1d"),
    ]),
    "Lomb-Scargle": ("paper", "jakevdp/PracticalLombScargle", ".", [
        ("PracticalLombScargle.tex", 130666, "7052b055f4f6ec2fa0128c0481ee59328ca99f0e0e06b379f6c7fe1db02293e4"),
    ]),
    "Tidy Data": ("paper", "hadley/tidy-data", ".", [
        ("tidy-data.tex", 52965, "e3fc1ed6bce81cf0be050972f5bcd510c90ce526b8fde847ae4c14a6a6bfb30c"),
    ]),
    "Think Python": ("book", "AllenDowney/ThinkPython2", "book", [
        ("book.tex", 503765, "f7db910748bd78b916824231f28568a3abd85c648e8a6b2920f81a20d2c5e945"),
    ]),
    "lshort-zh-cn": ("book", "CTeX-org/lshort-zh-cn", "src", [
        ("lshort-zh-cn.tex", 1142, "12bca2492a36619055f30afba1d740711e2eb98fb8fd9012440eb7fed21c0e75"),
        ("chap/app.A.install.tex", 8338, "8a76b8245e4af8e0dd0808fa05ba546703acdb4783ccedd5b9aab41b2a81897e"),
        ("chap/app.B.error.help.tex", 13848, "737ded478e22554d77fbd6870c97b7308dd09125b446d69014c85c1961d35157"),
        ("chap/biblio.tex", 4655, "124cd41618f03e4f258cfcd473ef710a7c77d0ae10a63f8c110bc92df26c0025"),
        ("chap/chap.01.basics.tex", 27323, "5a06906d0ecbef58c1ad83b584bf88c634669f6cd21f7b3eddb05c075ec6e17b"),
        ("chap/chap.02.text.tex", 19405, "bbe03d66b2f878300ddcc85981094af24c27b03029b2169d18d1bc914cd755ef"),
        ("chap/chap.03.elements.tex", 47060, "03b40712ee24c2a4301ce262611d95153da5ead7e3f5a5903f0de731aa3d3e85"),
        ("chap/chap.04.math.tex", 36689, "aa5c6ee911885f9654777c273e5e04e05a51567aba336fe6abac17002d07b38c"),
        ("chap/chap.05.style.tex", 40379, "a515aaefda4338c427d28223842fa0ece71d16b6d344b9188e5ef258701c1eb3"),
        ("chap/chap.06.spec.tex", 33350, "7e50bfb09f17d90092ab0bb7013ed7dd563485d8942e948689c2497b74e98c29"),
        ("chap/chap.07.graphic.tex", 17335, "12b34a9e8a0d212f0f41268aa5a0d99f9282b5f8d531331763872b1537b5a656"),
        ("chap/chap.08.custom.tex", 23565, "af97a3173583a24aa9470e667b20611e320070f0dfbd7a61dd52dcd0a74949e2"),
        ("chap/fdl.tex", 22944, "dfb97e7004f037dd059ec266f7db9cf76f44bef840aeb9e8cff37e3d863841f9"),
        ("chap/preface.contrib.tex", 10963, "0eadcdc7b20f3d0926b26ced7228d189459aafa7fbffef1d5be60f1f44c83f70"),
        ("chap/preface.overview.tex", 3658, "0edebeeb72fa4a03501cd79134018365c7446df7b0a7020e755f7f9158701355"),
        ("chap/sec.symbol.table.tex", 17722, "ce9968af5ceae78f8077ecd4d78857e97869c6d37471c3ddb1b77a1ded94a5b6"),
        ("chap/titlepage.tex", 1379, "dc196e9729690329bd6201e26527e9ed173afd93383955b58711dfb7bb42804a"),
    ]),
    "Roots of Language": ("book", "langsci/91", ".", [
        ("main.tex", 2674, "ff078d3dbb0e345d87bdf94be6452f5edca79c42177bb5152baf4d8999c5fca7"),
        ("chapters/01.tex", 94713, "9b68a7871b8776a879f0ca1c0006886e43632e9e54599a653d297efc720580a7"),
        ("chapters/02.tex", 222364, "4c006b226dc1e99097ec1b1dbfab34011386c60729cb2ff6dcd2b11d3878efda"),
        ("chapters/03.tex", 189108, "9886b879db101acbb62654bee1992ebf9a9876480bc1cc66aa28f2243cff9830"),
        ("chapters/04.tex", 197787, "73bd90601987dd1fdd4d1a29e4872e5088d99b7f6366769e6d4cb4b727a52eb2"),
        ("chapters/05.tex", 20971, "eec0953c75ec812074bcf6bc5d12f36cb2167028e6ce81e1b60996bb6b106424"),
        ("chapters/preface.tex", 8889, "b715f8852b864115cbc24f3e381260fafc489f814584d20918230568600b8c6f"),
        ("chapters/introduction.tex", 5867, "bcad93cc7e5fff97ff80f4b6322f434ad65a2c08f60d75a8c88e6ce920e6e644"),
        ("chapters/abbreviations.tex", 1158, "1496b09f6fb6884899c46a10f02471550c61ab082f33cb97c7ea191b7093bf5c"),
        ("chapters/acknowledgments.tex", 938, "9d0ea1037b23edb3d0cd865b178cf48312a01199a326da7c9e03e010fcdd7c66"),
    ]),
    "Lung tomography": ("thesis", "habi/PhD-Thesis", ".", [
        ("Thesis.tex", 6016, "4eb987880943be903f75eb7794d729b15aa957825fc7c4522cdea495087a8cd5"),
        ("Chapters/CTandTomcat.tex", 38458, "c9cfbb23eb06c384f2f4621fc07f0888ecf4695f00d57608905040cdd6df186c"),
        ("Chapters/Discussion.tex", 36549, "121da04c36bf4ae69e03aaa270f9398acdc5219e43b0f66366317ed7edd11d12"),
        ("Chapters/Haberthuer2010.tex", 66873, "6909ee36fb3cd9c8730a8eeae78a26326735e9f47d8f80ec6147741b7a6a65eb"),
        ("Chapters/Introduction.tex", 15435, "538969781eb6c51a65722b4e79eb407a6985a7e52c8e3e374c36f9b1325745e4"),
        ("Chapters/Outlook.tex", 4745, "f530fbe247a335e66be5b6e2d885c509807639fcc7750bc67de9d3ed84a885b8"),
        ("Chapters/Publications.tex", 6355, "694cdfbc8822de9cf8c523c519db031356f1a98fe653c0912670e5154f4b387e"),
        ("Chapters/TheMammalianLung.tex", 11527, "73a8740187a0965536ffc03f4643589bafbf76df97692950c93cff71553418d4"),
        ("Chapters/Tsuda2008.tex", 59115, "40b54335070df4c760e7b7a37dac15b59e4234f2ba393e5f76908bf7aaf52f8d"),
        ("Chapters/XRM2008.tex", 9946, "1df8edacc1ceed5e222d9a54a7f18049676964315e9128437ec6f90c1411e50a"),
    ]),
    "Web Scraping with R": ("slides", "eduardszoecs/talk_webscrapingr", ".", [
        ("slides.tex", 13146, "6acc1ba3fd513f75295bfffe251db9d53871a633dc9223c115ab18b0065dd017"),
    ]),
    "TeXConf 2022": ("slides", "t-tk/upmendex-package", "doc/present/src", [
        ("upmendex-slide-texconf2022.tex", 51858, "8558c4303b52f13df601a0a6feed3c51467cd36f1fb69650f886d094319afdc0"),
    ]),
    "Resume": ("short document", "billryan/resume", ".", [
        ("resume.tex", 2594, "ab6ccae5471721811304f7c1243954d6a51baca55d8a873baae04019f5357003"),
    ]),
    "Statistical cookbook": ("reference sheet", "mavam/stat-cookbook", ".", [
        ("stat-cookbook.tex", 85350, "c33e618d65beb5b32243321c20e1ad15706581e2746ee939fd2056b382905f01"),
        ("probstat.tex", 12148, "f3198adad83f9a2b65c4eca279a6cbd94fa9afd418bae365083d894e321455dc"),
    ]),
    "Lomb-Scargle references": ("bibliography", "jakevdp/PracticalLombScargle", ".", [
        ("PracticalLombScargle.bib", 57399, "36d5bd64dbe5cb1b9a0e9011d205131496821cc6fc796b646d37d9461b1352d4"),
    ]),
    "Grammatical theory references": ("bibliography", "langsci/25", ".", [
        ("gt.bib", 649287, REPOS["langsci/25"][1]),
    ]),
    "WALS Online": ("bibliography", "cldf-datasets/wals", "cldf", [
        ("sources.bib", 3042068, REPOS["cldf-datasets/wals"][1]),
    ]),
}


def digest_stream(source, output=None):
    digest = hashlib.sha256()
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(chunk)
        if output is not None:
            output.write(chunk)
    return digest


def sha(path):
    with path.open("rb") as source:
        return digest_stream(source).hexdigest()


def source_location(repo):
    commit, _ = REPOS[repo]
    slug = repo.replace("/", "--")
    if repo in RAW:
        path = RAW[repo]
        return f"{slug}--{Path(path).name}", f"https://raw.githubusercontent.com/{repo}/{commit}/{path}"
    return f"{slug}.zip", f"https://codeload.github.com/{repo}/zip/{commit}"


def materialize(repo, source, root):
    """Hash ZIP files in path order as (UTF-8 name, NUL, binary content digest)."""
    commit, expected = REPOS[repo]
    if repo in RAW:
        target = root / RAW[repo]
        target.parent.mkdir(parents=True)
        with source.open("rb") as data, target.open("xb") as output:
            actual = digest_stream(data, output)
    else:
        prefix = f"{repo.split('/')[1]}-{commit}/"
        actual = hashlib.sha256()
        with zipfile.ZipFile(source) as archive:
            if archive.comment != commit.encode():
                raise ValueError(f"{repo}: ZIP does not identify commit {commit}")
            for entry in sorted(archive.infolist(), key=lambda item: item.filename):
                if not entry.filename.startswith(prefix):
                    raise ValueError(f"{repo}: unexpected ZIP prefix in {entry.filename!r}")
                if entry.is_dir():
                    continue
                name = entry.filename[len(prefix):]
                if any(part in ("", ".", "..") for part in name.split("/")) or re.search(r"[\\:]", name):
                    raise ValueError(f"{repo}: invalid ZIP path {name!r}")
                target = (root / name).resolve()
                if not target.is_relative_to(root) or target == root:
                    raise ValueError(f"{repo}: ZIP path outside project: {name!r}")
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as data, target.open("xb") as output:
                    digest = digest_stream(data, output)
                actual.update(name.encode("utf-8") + b"\0" + digest.digest())
    if actual.hexdigest() != expected:
        raise ValueError(f"{repo}@{commit}: project SHA-256 mismatch (got {actual.hexdigest()})")


def prepare(work, archives, offline):
    roots = {}
    for repo in REPOS:
        name, url = source_location(repo)
        cached = archives / name
        root = work / repo.replace("/", "--")
        if cached.is_file():
            materialize(repo, cached, root)
        elif offline:
            raise FileNotFoundError(f"offline archive missing: {cached}")
        else:
            print(f"Downloading {url}", file=sys.stderr, flush=True)
            archives.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=archives, prefix="download-") as staging:
                download = Path(staging) / name
                with urlopen(url, timeout=90) as response, download.open("xb") as output:
                    shutil.copyfileobj(response, output)
                materialize(repo, download, root)
                download.replace(cached)
        roots[repo] = root
    return roots


def verify_files(roots):
    for title, (_, repo, directory, files) in WORKS.items():
        for name, size, expected in files:
            path = roots[repo] / directory / name
            if path.stat().st_size != size or sha(path) != expected:
                raise ValueError(f"{title}: input length or SHA-256 mismatch: {name}")


def invoke(binary, flags, env, cwd, names):
    start = time.perf_counter_ns()
    proc = subprocess.run([binary, *flags, *names], cwd=cwd, env=env, capture_output=True, timeout=240)
    elapsed = (time.perf_counter_ns() - start) / 1e6
    stderr = proc.stderr.decode("utf-8", "replace")
    summaries = re.findall(r"(?m)^(\d+) file\(s\) would change; (\d+) checked\.\r?$", stderr)
    changed, checked = map(int, summaries[0]) if len(summaries) == 1 else (None, None)
    valid = checked == len(names) and changed <= checked and proc.returncode == int(changed > 0)
    row = dict(ms=elapsed, exit=proc.returncode, checked=checked, would_change=changed, ok=valid)
    if not valid:
        row["diagnostics"] = stderr[-4000:]
    return row


def benchmark(args, binary, roots, report, work):
    threads = str(args.threads)
    flags = ["--check", "--no-cache", "--no-config", "--threads", threads]
    env = dict(os.environ, RAYON_NUM_THREADS=threads, TMP=str(work), TEMP=str(work), TMPDIR=str(work))
    measured = 7 if args.mode == "formal" else args.runs or 5
    report.update(binary=str(binary), binary_sha256=sha(binary), flags=flags, RAYON_NUM_THREADS=threads,
                  warmups=1, measured=measured, concurrent_cli=1, passes=[])
    report["version"] = subprocess.check_output([binary, "--version"], cwd=work, env=env, text=True, timeout=30).strip()
    for turn in range(measured + 1):
        order = sorted(WORKS, key=lambda title: hashlib.sha256(f"{turn}/{title}".encode()).digest())
        current = dict(index=turn, phase="warmup" if turn == 0 else "measured", order=order)
        report["passes"].append(current)
        times = []
        for title in order:
            item = report["works"][title]
            rows = item.setdefault("runs", [])
            cwd = roots[item["repo"]] / item["cwd"]
            row = invoke(binary, flags, env, cwd, list(item["files"]))
            rows.append(row)
            if not row["ok"]:
                raise ValueError(f"{title}, pass {turn}: invalid check (exit={row['exit']}, checked={row['checked']}, "
                                 f"would_change={row['would_change']}); {row['diagnostics'][-500:]}")
            if row["would_change"] != rows[0]["would_change"]:
                raise ValueError(f"{title}, pass {turn}: would_change changed from {rows[0]['would_change']} to {row['would_change']}")
            times.append(row["ms"])
        current["sum_cli_ms"] = sum(times)
        print(f"{current['phase']} {turn}/{measured}: {len(times)} works checked", file=sys.stderr, flush=True)
    verify_files(roots)
    report["binary_sha256_after"] = sha(binary)
    if report["binary_sha256_after"] != report["binary_sha256"]:
        raise ValueError("tift executable SHA-256 changed during benchmarking")
    for item in report["works"].values():
        samples = [row["ms"] for row in item["runs"][1:]]
        item["median_ms"] = statistics.median(samples)
        item["range_ms"] = [min(samples), max(samples)]
    report["median_sum_cli_ms"] = statistics.median(row["sum_cli_ms"] for row in report["passes"][1:])


def outside_checkout(path):
    path = path.resolve()
    script_dir = Path(__file__).resolve().parent
    if script_dir.name == "performance" and path.is_relative_to(script_dir.parent):
        raise ValueError(f"choose a location outside the checkout: {path}")
    return path


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("expected a positive integer")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mode", choices=("verify", "local", "formal"), required=True)
    parser.add_argument("--tift", "--binary", help="release executable (default: tift on PATH)", default="tift")
    parser.add_argument("--threads", type=positive, default=1, help="threads per process (default: 1; formal requires 1)")
    parser.add_argument("--runs", type=positive, help="measured passes in local mode (default: 5)")
    parser.add_argument("--archives", type=Path, help="reuse verified archives here and download missing ones; default: temporary")
    parser.add_argument("--offline", action="store_true", help="require all files in --archives; never download")
    parser.add_argument("--temp-dir", type=Path, help="temporary workspace parent outside checkout (default: system temp)")
    parser.add_argument("--report", type=Path, help="new JSON file outside checkout (default: stdout)")
    args = parser.parse_args()
    if args.mode != "local" and (args.threads != 1 or args.runs is not None):
        parser.error("custom threads and measured pass counts require --mode local")
    if args.offline and not args.archives:
        parser.error("--offline requires --archives DIR")
    temp_dir = outside_checkout(args.temp_dir or Path(tempfile.gettempdir()))
    archives = outside_checkout(args.archives) if args.archives else None
    report_path = outside_checkout(args.report) if args.report else None
    if report_path and report_path.exists():
        parser.error(f"report already exists: {report_path}")
    report = dict(mode=args.mode, accepted=False, started=datetime.now(timezone.utc).isoformat(),
                  host=platform.platform(), architecture=platform.machine(), logical_cpus=os.cpu_count(),
                  processor=platform.processor(), python=platform.python_version(), runner_sha256=sha(Path(__file__)),
                  repositories={repo: dict(commit=commit, project_sha256=digest, url=source_location(repo)[1])
                                for repo, (commit, digest) in REPOS.items()}, works={})
    try:
        binary = None if args.mode == "verify" else Path(shutil.which(args.tift) or args.tift).resolve(strict=True)
        temp_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temp_dir, prefix="tift-authored-") as temp:
            work = Path(temp)
            roots = prepare(work, archives or work / "archives", args.offline)
            verify_files(roots)
            for title, (genre, repo, directory, files) in WORKS.items():
                report["works"][title] = dict(genre=genre, repo=repo, commit=REPOS[repo][0], cwd=directory,
                                            files={name: dict(bytes=size, sha256=digest) for name, size, digest in files},
                                            bytes=sum(size for _, size, _ in files))
            report["corpus"] = dict(repositories=len(roots), works=len(WORKS),
                                   files=sum(len(item["files"]) for item in report["works"].values()),
                                   bytes=sum(item["bytes"] for item in report["works"].values()))
            if binary is not None:
                benchmark(args, binary, roots, report, work)
        report["accepted"] = True
    except (OSError, ValueError, HTTPException, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        report["error"] = str(error)
        print(f"Benchmark failed: {error}", file=sys.stderr)
    report["finished"] = datetime.now(timezone.utc).isoformat()
    text = json.dumps(report, indent=2) + "\n"
    if report_path:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        with report_path.open("x", encoding="utf-8") as output:
            output.write(text)
        print(report_path, file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0 if report["accepted"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        sys.exit(str(error))
