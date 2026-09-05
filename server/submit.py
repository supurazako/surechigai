#!/usr/bin/env python
"""手で文章を広場サーバに投げる（curl の代わり。Windows の引用符問題を避ける）。

  python submit.py A "暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた"
  python submit.py B --random                 # words.json からランダムに 1 文作って投げる
  python submit.py A "..." --server http://192.168.43.12:8000
"""
from __future__ import annotations

import argparse
import json
import random
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
for _s in (sys.stdout, sys.stderr):
    if hasattr(_s, "reconfigure"):
        _s.reconfigure(encoding="utf-8", errors="replace")


def rq(base: str, method: str, path: str, body: dict | None = None):
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body else None
    r = urllib.request.Request(base + path, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=10) as x:
        return json.loads(x.read())


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("device")
    ap.add_argument("sentence", nargs="?")
    ap.add_argument("--random", action="store_true", help="words.json からランダムに作る")
    ap.add_argument("--server", default="http://127.0.0.1:8000")
    ap.add_argument("--no-wait", action="store_true", help="完成を待たずに終了")
    a = ap.parse_args()

    if a.random:
        slots = json.loads((HERE / "words.json").read_text(encoding="utf-8"))["slots"]
        a.sentence = "　".join(random.choice(s["words"]) for s in slots)
    if not a.sentence:
        sys.exit("文章か --random が要る")

    try:
        j = rq(a.server, "POST", "/submit", {"device": a.device, "sentence": a.sentence})
    except (urllib.error.URLError, ConnectionError) as e:
        sys.exit(f"サーバに届かない ({a.server}): {e}。server.py は動いている？")
    print(f"#{j['id']} {j['status']}  {a.sentence}")
    if a.no_wait:
        return
    t0 = time.time()
    while True:
        s = rq(a.server, "GET", f"/jobs/{j['id']}")
        if s["status"] in ("done", "error"):
            break
        print(f"  {s['status']} {time.time() - t0:.0f}s", end="\r")
        time.sleep(1)
    print()
    if s["status"] == "done":
        print(f"done {time.time() - t0:.1f}s  {a.server}{s['image'] or '（画像なし）'}")
    else:
        sys.exit(f"error: {s['error']}")


if __name__ == "__main__":
    main()
