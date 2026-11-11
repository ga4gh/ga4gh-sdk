#!/usr/bin/env python3
"""Run two local providers, one real cached container, and the SDK reuse journey."""
import argparse
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import uuid

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=pathlib.Path)
    args = parser.parse_args()
    work = args.work or (ROOT / "target" / "result-reuse" / str(uuid.uuid4()) if sys.platform == "darwin" else pathlib.Path(tempfile.mkdtemp(prefix="ga4gh-result-reuse-")))
    work = work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    providers, logs, roots = [], [], []
    try:
        for name in ("a", "b"):
            port_file = work / f"{name}.port"
            port_file.unlink(missing_ok=True)
            logs.append((work / f"{name}.log").open("w"))
            providers.append(subprocess.Popen([sys.executable, str(HERE / "provider.py"), "--name", f"provider-{name}", "--workspace", str(work / name), "--port-file", str(port_file)], stdout=logs[-1], stderr=subprocess.STDOUT))
            for _ in range(100):
                if port_file.exists():
                    break
                if providers[-1].poll() is not None:
                    raise RuntimeError(f"provider {name} exited; inspect {work}")
                time.sleep(0.05)
            else:
                raise RuntimeError("provider startup timed out")
            roots.append(f"http://127.0.0.1:{port_file.read_text()}")
        env = os.environ.copy()
        env["GA4GH_REUSE_PROVIDERS"] = ",".join(roots)
        env["GA4GH_REUSE_EVIDENCE"] = str(work / "evidence.json")
        subprocess.run(["cargo", "test", "-p", "ga4gh-lib", "--features", "workflow-contracts", "--test", "result_reuse", "--locked", "real_local_result_reuse", "--", "--ignored", "--nocapture"], cwd=ROOT, env=env, check=True)
        print(f"Receipt and measured evidence: {work}")
    finally:
        for provider in providers:
            provider.terminate()
        for provider in providers:
            try:
                provider.wait(timeout=5)
            except subprocess.TimeoutExpired:
                provider.kill()
                provider.wait()
        for log in logs:
            log.close()


if __name__ == "__main__":
    main()
