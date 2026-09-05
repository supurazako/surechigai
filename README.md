# surechigai

BLEを使って近くの端末と5W1Hの文節を交換し、文章を組み立てるアプリです。実行環境ごとにディレクトリを分けています。

```text
cli/       macOS・Linux向けRust CLI
m5stack/   M5Stack向けファームウェア（実装予定）
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

M5Stack向けのコードは `m5stack/` に配置します。現在はディレクトリと案内のみを用意しています。
詳細は [m5stack/README.md](m5stack/README.md) を参照してください。
