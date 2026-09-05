#!/usr/bin/env python
"""完成した文章から画像を 1 枚作る（発展目標）。

  python imagegen.py "暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた"
  python imagegen.py "..." --backend openai --out out/1.png
  python imagegen.py "..." --backend gemini

既定は --backend dry（API を呼ばずプロンプトだけ表示）。
鍵は環境変数から読む: OPENAI_API_KEY / GEMINI_API_KEY。コードや Scrapbox に鍵を書かないこと。
モデル名は既定値を置いてあるが、当日使う前に公式ドキュメントで現行名を確認すること。
依存ライブラリ無し（標準ライブラリの urllib のみ）。
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

STYLE = "シンプルで明るいイラスト。1 枚絵。文字は入れない。"


def build_prompt(sentence: str) -> str:
    return f"次の文をそのまま絵にしてください。文: 「{sentence}」。{STYLE}"


def _post_json(url: str, headers: dict, body: dict, timeout: int = 120) -> dict:
    req = urllib.request.Request(url, data=json.dumps(body).encode("utf-8"), headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        sys.exit(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:500]}")


def gen_openai(prompt: str, model: str) -> bytes:
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        sys.exit("OPENAI_API_KEY が無い")
    res = _post_json(
        "https://api.openai.com/v1/images/generations",
        {"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        {"model": model, "prompt": prompt, "n": 1, "size": "1024x1024"},
    )
    return base64.b64decode(res["data"][0]["b64_json"])


def gen_gemini(prompt: str, model: str) -> bytes:
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        sys.exit("GEMINI_API_KEY が無い")
    res = _post_json(
        f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent",
        {"x-goog-api-key": key, "Content-Type": "application/json"},
        {"contents": [{"parts": [{"text": prompt}]}], "generationConfig": {"responseModalities": ["IMAGE"]}},
    )
    for part in res["candidates"][0]["content"]["parts"]:
        if "inlineData" in part:
            return base64.b64decode(part["inlineData"]["data"])
    sys.exit("画像が返ってこなかった: " + json.dumps(res, ensure_ascii=False)[:500])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sentence")
    ap.add_argument("--backend", choices=["dry", "openai", "gemini"], default="dry")
    ap.add_argument("--model", default=None, help="既定: openai=gpt-image-1 / gemini=gemini-2.5-flash-image")
    ap.add_argument("--out", type=Path, default=Path("out/image.png"))
    args = ap.parse_args()

    prompt = build_prompt(args.sentence)
    print("prompt:", prompt)
    if args.backend == "dry":
        return

    if args.backend == "openai":
        png = gen_openai(prompt, args.model or "gpt-image-1")
    else:
        png = gen_gemini(prompt, args.model or "gemini-2.5-flash-image")

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(png)
    print("saved:", args.out)


if __name__ == "__main__":
    main()
