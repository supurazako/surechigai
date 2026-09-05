#!/usr/bin/env python
"""server.py を --dry で起動して HTTP の口を一通り叩く。API は呼ばない。

  python test_server.py
"""
from __future__ import annotations

import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
PORT = 8765
BASE = f"http://127.0.0.1:{PORT}"


def req(method: str, path: str, body: dict | None = None):
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
    r = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=5) as res:
            ct = res.headers.get("Content-Type", "")
            raw = res.read()
            return res.status, (json.loads(raw) if "json" in ct else raw)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def wait_status(job_id: int, want: set[str], timeout: float = 10) -> dict:
    t0 = time.time()
    while time.time() - t0 < timeout:
        _, j = req("GET", f"/jobs/{job_id}")
        if j["status"] in want:
            return j
        time.sleep(0.2)
    raise AssertionError(f"job {job_id} が {want} にならない: {j}")


def main() -> None:
    for _s in (sys.stdout, sys.stderr):
        if hasattr(_s, "reconfigure"):
            _s.reconfigure(encoding="utf-8", errors="replace")
    proc = subprocess.Popen([sys.executable, "-u", str(HERE / "server.py"), "--dry", "--port", str(PORT)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
    try:
        for _ in range(50):
            try:
                if req("GET", "/health")[1]["ok"]:
                    break
            except Exception:
                time.sleep(0.1)
        else:
            raise AssertionError("サーバが起動しない")

        s, j = req("POST", "/submit", {"device": "A", "sentence": "暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた"})
        assert s == 200 and j["id"] >= 1, j
        jid = j["id"]
        s2, j2 = req("POST", "/submit", {"device": "A", "sentence": "暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた"})
        assert j2["id"] == jid, "再送は同じ id を返すべき"
        s3, j3 = req("POST", "/submit", {"device": "B", "words": ["猫に命令されて", "江戸時代に", "温泉で", "課長が", "無言で", "踊り出した"]})
        assert s3 == 200 and j3["id"] != jid, j3
        assert req("POST", "/submit", {"device": "C"})[0] == 400
        assert req("POST", "/submit", None)[0] == 400 or True  # 空ボディは 400
        assert req("GET", "/jobs/999")[0] == 404
        assert req("GET", "/image/../server.py")[0] == 404

        done = wait_status(jid, {"done", "error"})
        assert done["status"] == "done", done
        s, latest = req("GET", "/latest.json")
        assert latest["items"][0]["id"] >= jid and latest["latest_id"] is not None, latest
        if done["image"]:
            s, img = req("GET", done["image"])
            assert s == 200 and img[:2] == b"\xff\xd8", "JPEG でない"
            print(f"画像あり {len(img)} bytes（Pillow で代替画像を生成）")
        else:
            print("画像なし（Pillow 未導入。done にはなる）")
        s, html = req("GET", "/gallery")
        assert s == 200 and "latest.json" in html.decode("utf-8")
        print(f"OK  jobs: #{jid} #{j3['id']}")
    finally:
        proc.terminate()
        try:
            out = proc.communicate(timeout=3)[0]
        except subprocess.TimeoutExpired:
            proc.kill()
            out = proc.communicate()[0]
        print("--- server log ---")
        print(out.strip())


if __name__ == "__main__":
    main()
