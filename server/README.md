# 広場サーバ（文章 → 画像 → 表示）

完成した5W1Hの文章を受け取り、OpenAIまたはApple Silicon上のローカルモデルで画像を生成して広場ページ・Tab5に表示するPythonサーバです。
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

### Apple Siliconでローカル生成

M1以降のMacでは、PyTorchのMPSバックエンドとSD-Turboを使って、APIキーなしで画像を生成できます。メモリ8GBを想定し、モデルを1回だけ読み込んで512×512・1枚・1ステップで直列生成します。

```sh
cd server
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements-apple-silicon.txt
python server.py --backend apple-silicon
```

初回起動時はHugging Faceからモデルを取得するため、インターネット接続と数GBの空き容量が必要です。2回目以降はキャッシュされたモデルを使用します。画質を少し上げる場合は `--steps 2`、モデル名を変更する場合は `--model <Hugging FaceのモデルID>` を指定できます。SD-Turbo向けのステップ数は1〜4に制限しています。

Apple M2・メモリ8GB・Python 3.12で確認したところ、モデルは約2.6GB、キャッシュ済みモデルでの生成は約3.6〜7.0秒でした。生成時間は他のアプリによるメモリ使用量や温度で変わります。

ローカル生成にはarm64版PythonとmacOS 13以降を推奨します。MPSを利用できない場合や依存パッケージがない場合は、サーバ起動時に理由を表示して終了します。

SD-TurboにはOpenAI APIと同等の入力・出力フィルターはありません。不特定多数が自由入力できる状態で公開せず、イベントでは配布する語句を運営側で確認してください。利用前に[モデルカード](https://huggingface.co/stabilityai/sd-turbo)のライセンスと制限事項も確認してください。

SD-Turboのテキストエンコーダーには77トークンの上限があります。日本語はトークン数が増えやすいため、ローカル用プロンプトでは長い指示を付けず、完成した5W1H本文を先頭に置いています。長い文章では末尾が画像へ反映されない場合があります。

画像生成方式は `--backend openai|apple-silicon|dry` で切り替えます。既定は従来どおり `openai` で、`--dry` も引き続き利用できます。

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
| `GET /image/12.jpg` | Tab5 / ブラウザ | 生成画像 JPEG（OpenAIは1024×1024、ローカルは512×512）|
| `GET /` | ブラウザ | 広場ページ。3 秒ごと自動更新 |

- 同じデバイスから同じ文が 60 秒以内に来たら同じ id を返す（再送しても二重生成しない）
- `status` が `queued → working → done` と変化する（失敗は `error`）。生成時間はバックエンドと端末性能による
- Tab5向けの表示コードは [../m5stack/tab5_hiroba.ino](../m5stack/tab5_hiroba.ino)

## 語句表の書き方

`words.json` の `slots` は並び順のまま文章になる。`key` は固定、`label` と `words` は自由。
1 語 20 字以内。組み合わせたときにギャップが出る語（時代・場所・固有名詞のズレ）が面白い。
