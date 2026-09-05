# M5Stack版

M5Stack向けファームウェアを配置するディレクトリです。

- `tab5_hiroba.ino` — Tab5向け。[../server/](../server/) の広場サーバから完成した文章・画像を取得し全画面表示する（実装済み・実機未検証）
- `atom_post.ino` — AtomS3R向け。5W1Hが完成した時に `postSentence()` を1回呼ぶだけで、広場サーバへ `POST /submit` する橋渡しコード（実装済み・実機未検証）
- **AtomS3R同士のBLE交換ファームウェア（5W1Hのすれ違い交換ロジック本体）はこのリポジトリにまだ含まれていません。** 完成時に `atom_post.ino` の `postSentence()` を呼び出す形で組み込む想定です。

PC向け実装は [../cli/](../cli/) にあります。
CLIとデータを交換する際のBLE Service UUID・Characteristic・フレーム形式は、[CLIの通信仕様](../cli/README.md#構成と通信仕様) を参照してください。
