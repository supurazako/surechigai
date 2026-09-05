# surechigai

BLEを使って近くの端末と5W1Hの文節を交換し、文章を組み立てるアプリです。実行環境ごとにディレクトリを分けています。

```text
cli/       macOS・Linux向けRust CLI（PC同士・PCとTab5のBLE交換）
m5stack/   M5Stack Tab5向けファームウェア（BLE交換・タッチUI実装済み）
web/       CLIに組み込む端末別Web Viewer
server/    完成した文章から画像を生成し、広場ページで表示するPythonサーバ
docs/      発表用資料
```

## CLI

```sh
cd cli
cargo build --locked
./target/debug/surechigai --name "alice" --who "犬が" --where "パリに" --what "行く" --rssi-threshold=-65
```

ビルド成果物は `cli/target/debug/surechigai`、`--release` でビルドした場合は `cli/target/release/surechigai` です。
セットアップ、実機確認、テスト、通信仕様は [cli/README.md](cli/README.md) を参照してください。

ブラウザから配布文節を設定し、作成中文章を見る場合は次のように起動します。

```sh
./target/debug/surechigai --web
```

表示されたURL（既定では `http://127.0.0.1:8787`）を同じ端末のブラウザで開いてください。

## M5Stack

M5Stack Tab5向けのファームウェアを `m5stack/` に配置しています。CLIと互換のBLEプロトコルで5W1Hを交換し、
Tab5自身のタッチUIに作成中の文章を表示します。詳細は [m5stack/README.md](m5stack/README.md) を参照してください。

## 広場サーバ（画像生成）

完成した文章を受け取り、OpenAIまたはApple Silicon上のローカルモデルで画像を生成して広場ページに表示するPythonサーバです。
CLI・Tab5ファームウェアの本体ロジックとは独立して動作し、`POST /submit` でのみ繋がります。
文章完成時にCLIから自動でPOSTする機能は `cli/README.md` の「広場サーバ連携」を参照してください。
詳細は [server/README.md](server/README.md) を参照してください。
