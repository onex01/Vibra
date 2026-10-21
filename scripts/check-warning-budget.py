#!/usr/bin/env python3
import os
import re
import subprocess
import sys


WARNING_SUMMARY = re.compile(r"`vibra-kernel` \(lib\) generated (\d+) warnings?")
BUILDS = (
    ("debug", [], int(os.environ.get("DEBUG_WARNING_BUDGET", "107"))),
    ("no-default-features", ["--no-default-features"], int(os.environ.get("NO_DEFAULT_WARNING_BUDGET", "110"))),
)


def main() -> int:
    for name, features, budget in BUILDS:
        command = [
            "cargo", "+nightly", "build", "-p", "vibra-kernel", *features,
            "-Z", "build-std=core,alloc,compiler_builtins", "-Z", "build-std-features",
        ]
        environment = os.environ.copy()
        environment["CARGO_INCREMENTAL"] = "0"
        result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=environment)
        if result.returncode:
            sys.stdout.write(result.stdout)
            return result.returncode

        summaries = WARNING_SUMMARY.findall(result.stdout)
        warnings = int(summaries[-1]) if summaries else 0
        print(f"{name}: {warnings} warnings (budget {budget})", flush=True)
        if warnings > budget:
            print(f"warning budget exceeded for {name}", file=sys.stderr)
            sys.stdout.write(result.stdout)
            return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())