#!/usr/bin/env python
"""広場サーバ: 完成した文章を受け取り、画像を生成して配る。

  python server.py                 # 本番（OpenAI を呼ぶ。OPENAI_API_KEY 必須）
  python server.py --dry           # API を呼ばない。動作確認用（Pillow があれば文字入りの代替画像を作る）
  python server.py --port 8000 --quality low

HTTP の口（これがデバイスとの接点）:
  POST /submit            {"device":"A","sentence":"…"}  → {"id":12,"status":"queued"}
                          sentence の代わりに {"words":["…","…"]} でも可（全角スペースで連結）
  GET  /latest.json       {"latest_id":12,"items":[{id,device,sentence,status,image,created},…]}
  GET  /jobs/12           1 件の状態
  GET  /image/12.jpg      生成画像（JPEG）
  GET  /  または /gallery  ブラウザ用の広場（3 秒ごとに自動更新）
  GET  /health            {"ok":true}

依存: 標準ライブラリのみ（--dry の代替画像だけ Pillow があれば使う）。
鍵は環境変数 OPENAI_API_KEY から読む。コードに書かない。
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import queue
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

for _s in (sys.stdout, sys.stderr):
    if hasattr(_s, "reconfigure"):
        _s.reconfigure(encoding="utf-8", errors="replace")

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
IMAGES = DATA / "images"
JOBS_FILE = DATA / "jobs.jsonl"

STYLE = "シンプルで明るいイラスト。1 枚絵。文字は入れない。"
DEDUP_SECONDS = 60  # 同じデバイスから同じ文が短時間に来たら同じ id を返す（再送対策）


def build_prompt(sentence: str) -> str:
    return f"次の文をそのまま絵にしてください。文: 「{sentence}」。{STYLE}"


# ---------------------------------------------------------------- 状態
class Store:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.jobs: dict[int, dict] = {}
        self.next_id = 1
        DATA.mkdir(exist_ok=True)
        IMAGES.mkdir(exist_ok=True)
        if JOBS_FILE.exists():
            for line in JOBS_FILE.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    j = json.loads(line)
                    self.jobs[j["id"]] = j
            if self.jobs:
                self.next_id = max(self.jobs) + 1
            # 前回途中だったものは失敗扱いにする
            for j in self.jobs.values():
                if j["status"] in ("queued", "working"):
                    j["status"] = "error"
                    j["error"] = "server restarted"

    def _persist(self) -> None:
        JOBS_FILE.write_text(
            "".join(json.dumps(j, ensure_ascii=False) + "\n" for j in self.jobs.values()),
            encoding="utf-8",
        )

    def add(self, device: str, sentence: str) -> tuple[dict, bool]:
        """ジョブを追加。(job, 新規か) を返す。"""
        now = time.time()
        with self.lock:
            for j in reversed(list(self.jobs.values())):
                if j["device"] == device and j["sentence"] == sentence and now - j["created"] < DEDUP_SECONDS:
                    return j, False
            job = {
                "id": self.next_id,
                "device": device,
                "sentence": sentence,
                "status": "queued",
                "image": None,
                "error": None,
                "created": now,
            }
            self.jobs[job["id"]] = job
            self.next_id += 1
            self._persist()
            return job, True

    def update(self, job_id: int, **fields) -> None:
        with self.lock:
            self.jobs[job_id].update(fields)
            self._persist()

    def get(self, job_id: int) -> dict | None:
        with self.lock:
            j = self.jobs.get(job_id)
            return dict(j) if j else None

    def latest(self, n: int = 20) -> dict:
        with self.lock:
            items = sorted(self.jobs.values(), key=lambda j: j["id"], reverse=True)[:n]
            done = [j["id"] for j in items if j["status"] == "done"]
            return {"latest_id": done[0] if done else None, "items": [dict(j) for j in items]}


# ---------------------------------------------------------------- 画像生成
def gen_openai(prompt: str, quality: str, model: str) -> bytes:
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        raise RuntimeError("OPENAI_API_KEY が無い")
    body = {
        "model": model,
        "prompt": prompt,
        "n": 1,
        "size": "1024x1024",
        "quality": quality,
        "output_format": "jpeg",
        "output_compression": 60,
    }
    req = urllib.request.Request(
        "https://api.openai.com/v1/images/generations",
        data=json.dumps(body).encode("utf-8"),
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            res = json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:300]}") from None
    return base64.b64decode(res["data"][0]["b64_json"])


def gen_dry(sentence: str) -> bytes | None:
    """API を呼ばずに代替画像を作る。Pillow が無ければ None（画像なしで done にする）。"""
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        return None
    import io

    img = Image.new("RGB", (1024, 1024), (240, 224, 120))
    d = ImageDraw.Draw(img)
    font = None
    for cand in ("C:/Windows/Fonts/meiryo.ttc", "C:/Windows/Fonts/msgothic.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"):
        if Path(cand).exists():
            font = ImageFont.truetype(cand, 44)
            break
    words = sentence.split("　")
    y = 512 - 30 * len(words)
    for w in words:
        d.text((80, y), w, fill=(30, 30, 30), font=font)
        y += 60
    d.text((80, 940), "DRY RUN", fill=(120, 100, 40), font=font)
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=70)
    return buf.getvalue()


class Worker(threading.Thread):
    def __init__(self, store: Store, q: "queue.Queue[int]", dry: bool, quality: str, model: str):
        super().__init__(daemon=True)
        self.store, self.q, self.dry, self.quality, self.model = store, q, dry, quality, model

    def run(self) -> None:
        while True:
            job_id = self.q.get()
            job = self.store.get(job_id)
            if not job:
                continue
            self.store.update(job_id, status="working")
            t0 = time.time()
            try:
                if self.dry:
                    data = gen_dry(job["sentence"])
                else:
                    data = gen_openai(build_prompt(job["sentence"]), self.quality, self.model)
                image = None
                if data:
                    (IMAGES / f"{job_id}.jpg").write_bytes(data)
                    image = f"/image/{job_id}.jpg"
                self.store.update(job_id, status="done", image=image)
                print(f"[done ] #{job_id} {time.time() - t0:.1f}s {job['sentence']}")
            except Exception as e:  # noqa: BLE001 — 1 件の失敗でワーカーを止めない
                self.store.update(job_id, status="error", error=str(e))
                print(f"[error] #{job_id} {e}")


# ---------------------------------------------------------------- HTTP
GALLERY_HTML = """<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>すれ違い広場</title>
<style>
  body{margin:0;background:#111;color:#eee;font-family:"Hiragino Sans","Yu Gothic UI","Noto Sans JP",system-ui,sans-serif}
  header{padding:14px 22px;font-size:18px;letter-spacing:.1em;border-bottom:1px solid #333;display:flex;justify-content:space-between}
  header small{color:#888;letter-spacing:0}
  #hero{display:grid;grid-template-columns:min(58vh,560px) 1fr;gap:28px;padding:26px 22px;align-items:center}
  #hero img{width:100%;aspect-ratio:1;object-fit:cover;border-radius:12px;background:#222}
  #hero .s{font-size:clamp(22px,3.2vw,40px);line-height:1.6;letter-spacing:.04em}
  #hero .d{color:#888;margin-top:14px;font-size:14px}
  #grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:12px;padding:0 22px 40px}
  .card{background:#1b1b1b;border-radius:10px;overflow:hidden}
  .card img,.card .ph{width:100%;aspect-ratio:1;object-fit:cover;display:block;background:#2a2a2a}
  .card .ph{display:flex;align-items:center;justify-content:center;color:#777;font-size:13px}
  .card p{margin:0;padding:8px 10px;font-size:12px;line-height:1.5;color:#ccc}
  .empty{padding:60px 22px;color:#777;text-align:center}
</style></head><body>
<header><span>すれ違い広場</span><small id="st"></small></header>
<div id="hero" hidden><img id="hi" alt=""><div><div class="s" id="hs"></div><div class="d" id="hd"></div></div></div>
<div id="grid"></div>
<div class="empty" id="empty">まだ誰も完成していない</div>
<script>
let lastLatest=null;
function esc(s){return String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));}
async function tick(){
  try{
    const r=await fetch('/latest.json',{cache:'no-store'}); const j=await r.json();
    document.getElementById('st').textContent=new Date().toLocaleTimeString()+'  '+j.items.length+' 件';
    const items=j.items; document.getElementById('empty').hidden=items.length>0;
    const done=items.filter(x=>x.status==='done');
    const hero=document.getElementById('hero');
    if(done.length){ const h=done[0]; hero.hidden=false;
      if(h.id!==lastLatest){ lastLatest=h.id;
        document.getElementById('hi').src=h.image?h.image+'?'+h.id:'';
        document.getElementById('hs').textContent=h.sentence;
        document.getElementById('hd').textContent='#'+h.id+'  '+h.device; } }
    const g=document.getElementById('grid'); g.innerHTML='';
    for(const x of items){ const c=document.createElement('div'); c.className='card';
      const lab=x.status==='done'?'':(x.status==='error'?'失敗':'生成中…');
      c.innerHTML=(x.image?`<img src="${esc(x.image)}" alt="">`:`<div class="ph">${esc(lab||'画像なし')}</div>`)+`<p>#${x.id} ${esc(x.device)}<br>${esc(x.sentence)}</p>`;
      g.appendChild(c);}
  }catch(e){ document.getElementById('st').textContent='接続できない'; }
}
tick(); setInterval(tick,3000);
</script></body></html>
"""


def make_handler(store: Store, q: "queue.Queue[int]"):
    class H(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):  # 静かに。必要な行は自分で print する
            pass

        def _send(self, code: int, body: bytes, ctype: str) -> None:
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(body)

        def _json(self, code: int, obj) -> None:
            self._send(code, json.dumps(obj, ensure_ascii=False).encode("utf-8"), "application/json; charset=utf-8")

        def do_GET(self) -> None:
            p = self.path.split("?", 1)[0]
            if p in ("/", "/gallery"):
                self._send(200, GALLERY_HTML.encode("utf-8"), "text/html; charset=utf-8")
            elif p == "/health":
                self._json(200, {"ok": True})
            elif p == "/latest.json":
                self._json(200, store.latest())
            elif p.startswith("/jobs/"):
                try:
                    job = store.get(int(p[6:]))
                except ValueError:
                    job = None
                self._json(200, job) if job else self._json(404, {"error": "not found"})
            elif p.startswith("/image/") and p.endswith(".jpg"):
                f = IMAGES / p[7:]
                if f.is_file() and f.parent == IMAGES:
                    self._send(200, f.read_bytes(), "image/jpeg")
                else:
                    self._json(404, {"error": "not found"})
            else:
                self._json(404, {"error": "not found"})

        def do_POST(self) -> None:
            p = self.path.split("?", 1)[0]
            if p != "/submit":
                self._json(404, {"error": "not found"})
                return
            n = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(n) if n else b""
            try:
                body = json.loads(raw.decode("utf-8")) if raw else {}
            except (UnicodeDecodeError, json.JSONDecodeError):
                self._json(400, {"error": "JSON が読めない"})
                return
            device = str(body.get("device") or "?")[:32]
            sentence = body.get("sentence")
            if not sentence and isinstance(body.get("words"), list):
                sentence = "　".join(str(w) for w in body["words"])
            sentence = (sentence or "").strip()
            if not sentence:
                self._json(400, {"error": "sentence か words が要る"})
                return
            job, new = store.add(device, sentence[:200])
            if new:
                q.put(job["id"])
                print(f"[queue] #{job['id']} {device}: {sentence}")
            self._json(200, {"id": job["id"], "status": job["status"]})

    return H


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--dry", action="store_true", help="OpenAI を呼ばない")
    ap.add_argument("--quality", default="low", choices=["low", "medium", "high"], help="OpenAI の画質（既定 low = 速い・安い）")
    ap.add_argument("--model", default="gpt-image-1")
    args = ap.parse_args()

    if not args.dry and not os.environ.get("OPENAI_API_KEY"):
        sys.exit("OPENAI_API_KEY が無い。--dry で動かすか鍵を環境変数に入れる")

    store = Store()
    q: "queue.Queue[int]" = queue.Queue()
    Worker(store, q, args.dry, args.quality, args.model).start()
    srv = ThreadingHTTPServer(("0.0.0.0", args.port), make_handler(store, q))
    mode = "DRY（API を呼ばない）" if args.dry else f"OpenAI {args.model} quality={args.quality}"
    print(f"広場サーバ http://0.0.0.0:{args.port}/  {mode}  既存ジョブ {len(store.jobs)} 件")
    print("デバイスからは PC の LAN IP で。ipconfig の IPv4 アドレスを確認。Ctrl+C で終了")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\n終了")


if __name__ == "__main__":
    main()
