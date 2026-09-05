# surechigai

BLEを使って近くの端末と5W1Hの文節を交換し、文章を組み立てるアプリです。実行環境ごとにディレクトリを分けています。

```text
cli/       macOS・Linux向けRust CLI（PC同士のBLE交換）
m5stack/   M5Stack向けファームウェア（実装予定。Tab5の広場表示のみ実装済み）
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

## M5Stack

M5Stack向けのコードは `m5stack/` に配置します。AtomS3R同士のBLE交換ファームウェアは実装予定、
Tab5向けの広場表示（`tab5_hiroba.ino`）は実装済みです。詳細は [m5stack/README.md](m5stack/README.md) を参照してください。

## 広場サーバ（画像生成）

完成した文章を受け取り、OpenAIで画像を生成して広場ページ・Tab5に表示するPythonサーバです。
本体（CLI・M5Stackファームウェア）とは独立して動作し、`POST /submit` でのみ繋がります。
詳細は [server/README.md](server/README.md) を参照してください。
