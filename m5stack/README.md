# M5Stack版

M5Stack向けファームウェアを配置するディレクトリです。

- `tab5_hiroba.ino` — Tab5向け。[../server/](../server/) の広場サーバから完成した文章・画像を取得し全画面表示する（実装済み・実機未検証）
- AtomS3R同士のBLE交換ファームウェアは未実装です。対象機種と開発環境は未選定です。

PC向け実装は [../cli/](../cli/) にあります。
CLIとデータを交換する際のBLE Service UUID・Characteristic・フレーム形式は、[CLIの通信仕様](../cli/README.md#構成と通信仕様) を参照してください。
