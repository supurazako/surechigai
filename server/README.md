# 広場サーバ（文章 → 画像 → 表示）

完成した5W1Hの文章を受け取り、OpenAIで画像を生成して広場ページ・Tab5に表示するPythonサーバです。
`cli/`（PC同士のBLE交換）や `m5stack/`（M5Stack向けファームウェア）とは独立して動作し、
`POST /submit` でのみ繋がります。どちらが完成した文章を送ってきても構いません。

## 動かす

```powershell
# PowerShell（Windows 標準）。&& は使えないので 1 行ずつ
cd server
python server.py --dry     # API を呼ばずに動作確認（Pillow があれば文字入りの代替画像）
python server.py           # 本番。OPENAI_API_KEY を環境変数に入れておく
python test_server.py      # 口を一通り叩く自動テスト（dry）
```

別のターミナルから手で投げる（PowerShell の `curl` は別物なので使わない）:

```powershell
python submit.py A "暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた"
python submit.py B --random          # words.json からランダムに 1 文
```

BLE無しで交換ルールをPC上で試すシミュレータ:

```sh
python sim.py                      # 対話モード（meet A B / show A / auto 20）
python sim.py --auto 30 --seed 1   # 一気に 30 回すれ違わせて結果を見る
```

ブラウザで http://localhost:8000/ を開いておくと、広場ページが3秒ごと自動更新されます。

## 接点（ここだけ合わせれば合体できる）

| 口 | 使う側 | 内容 |
|---|---|---|
| `POST /submit` | AtomS3R / CLI | `{"device":"A","sentence":"暇だったので　真夜中に　…"}` または `{"device":"A","words":["…",…]}`。即 `{"id":12,"status":"queued"}` |
| `GET /latest.json` | Tab5 / ブラウザ | `{"latest_id":12,"items":[{id,device,sentence,status,image},…]}`（新しい順 20 件）|
| `GET /image/12.jpg` | Tab5 / ブラウザ | 生成画像 JPEG 1024×1024（100〜200 KB）|
| `GET /` | ブラウザ | 広場ページ。3 秒ごと自動更新 |

- 同じデバイスから同じ文が 60 秒以内に来たら同じ id を返す（再送しても二重生成しない）
- 生成は 10〜30 秒。`status` が `queued → working → done`（失敗は `error`）
- Tab5向けの表示コードは [../m5stack/tab5_hiroba.ino](../m5stack/tab5_hiroba.ino)

## 語句表の書き方

`words.json` の `slots` は並び順のまま文章になる。`key` は固定、`label` と `words` は自由。
1 語 20 字以内。組み合わせたときにギャップが出る語（時代・場所・固有名詞のズレ）が面白い。
