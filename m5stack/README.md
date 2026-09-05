# surechigai for M5Stack Tab5

M5Stack Tab5とPC向け[surechigai CLI](../cli/)の間で、近距離のBLE 5W1H文節交換を行うファームウェアです。
Wi-Fiやインターネットは使用しません。

CLI版と同じ動作を実装しています。

- Central（接続）とPeripheral（待受）を1〜5秒ごとに交互に切り替える自動モード
- RSSI閾値以上の端末だけに接続
- 双方が相手の作成中文章に足りない5W1H文節を1つずつ配布・受取
- 6種類の配布デッキと、出所を保持する作成中文章
- 端末UUID、分割フレーム、返信選択、ACKを含むCLI互換プロトコル
- 同じ相手との再交換を30秒間抑制
- 交換タイムアウトと途中切断からの復帰
- 日本語表示対応のTab5タッチUIとUSBシリアルログ
- PSRAM上の画面バッファを使った、ちらつきのない一括描画

## 必要なもの

- M5Stack Tab5
- データ通信対応USB Type-Cケーブル
- [PlatformIO Core](https://docs.platformio.org/en/latest/core/installation/index.html) またはPlatformIO IDE

Tab5はESP32-P4と無線用ESP32-C6の2チップ構成です。このプロジェクトはESP-Hosted経由のBLEに対応したArduino Core 3.3.11を使用します。
PlatformIO、Arduino Core、M5Unified、M5GFXのバージョンは[`platformio.ini`](platformio.ini)で固定しています。

## ビルドと書き込み

このディレクトリで実行します。

```sh
pio run
pio run --target upload
pio device monitor
```

生成される主なファイル:

```text
.pio/build/tab5/firmware.bin
.pio/build/tab5/firmware.factory.bin
```

書き込みポートを自動検出できない場合は、`platformio.ini`の`upload_port`またはコマンドの`--upload-port`で指定してください。

### ESP32-C6の注意

BLE初期化で停止する、Hosted HCIエラーが出る、スキャンできない場合は、Tab5内蔵ESP32-C6のファームウェアを確認してください。
[M5Stack公式の復元手順](https://docs.m5stack.com/en/guide/restore_factory/m5tab5_c6_wifi)に従ってC6の工場ファームウェアを復元してから再起動します。

2025年10月以降のTab5には異なる画面・タッチドライバーが搭載された個体があります。このプロジェクトで固定しているM5Unified/M5GFXは両方に対応する版です。機種情報は[M5Stack公式Tab5ページ](https://docs.m5stack.com/en/core/Tab5)も参照してください。

## 設定

初期値は[`include/app_config.hpp`](include/app_config.hpp)で変更できます。

| 設定 | 初期値 | 内容 |
| --- | --- | --- |
| `NAME` | `tab5` | 相手に表示するユーザー名。UTF-8で1〜32バイト |
| `WHEN` | `ある日` | 配布する「いつ」。UTF-8で1〜64バイト |
| `WHERE` | `パリに` | 配布する「どこで」。UTF-8で1〜64バイト |
| `WHO` | `犬が` | 配布する「だれが」。UTF-8で1〜64バイト |
| `WHAT` | `行く` | 配布する「なにをする」。UTF-8で1〜64バイト |
| `WHY` | `散歩のため` | 配布する「なぜ」。UTF-8で1〜64バイト |
| `HOW` | `従順な` | 配布する「どのように」。UTF-8で1〜64バイト |
| `INITIAL_ROLE` | `Role::Auto` | `Auto` / `Central` / `Peripheral` |
| `RSSI_THRESHOLD` | `-65` | Central時に接続するRSSI下限（dBm） |
| `ROLE_MIN_SECONDS` | `1` | 役割継続時間の最小値 |
| `ROLE_MAX_SECONDS` | `5` | 役割継続時間の最大値 |
| `EXCHANGE_TIMEOUT_SECONDS` | `10` | 1回の交換の制限時間 |
| `COOLDOWN_SECONDS` | `30` | 同じ端末と再交換するまでの時間 |

上限は文字数ではなくUTF-8のバイト数です。例えば日本語の多くは1文字3バイトです。長さはコンパイル時、不正なUTF-8は起動時に検出します。

## 画面操作

- `停止` / `再開`: BLE動作を停止・再開します。
- `役割`: 自動 → 接続 → 待受の順に切り替えます。交換中の役割変更は交換完了またはタイムアウトまで待ちます。
- `−` / `+`: RSSI閾値を5dBmずつ変更します。値が高いほど接続条件が厳しくなります。

画面には現在の実役割と設定役割、作成中の文章と残り文節数、最後に配布・受取した文節、相手名、端末ID、直近の状態を表示します。タッチで変更した役割とRSSIは再起動すると`app_config.hpp`の初期値に戻ります。

## CLIとの実機確認

まず役割を固定すると確認しやすくなります。

1. Tab5の画面で役割を`待受`にします。
2. PCでCLIをCentralとして起動します。

```sh
cd ../cli
cargo run --locked -- --role central --name "alice" --who "猫が" --rssi-threshold=-85
```

続いて逆方向を確認します。

1. Tab5の画面で役割を`接続`にします。
2. PCでCLIをPeripheralとして起動します。

```sh
cargo run --locked -- --role peripheral --name "alice" --who "猫が"
```

両方向で交換できたらTab5を`自動`に戻し、CLIも`--role auto`で確認します。交換できない場合は最初にRSSIを`-85dBm`程度まで緩め、端末を近づけてください。

## テスト

プロトコルと待受状態機械は、Tab5なしでmacOS/Linux上でもテストできます。

```sh
cmake -S . -B build
cmake --build build
ctest --test-dir build --output-on-failure
```

テスト対象にはProfile/Giftの往復、日本語・絵文字・64バイト境界、不正UTF-8、文章の重複防止、対称交換、ACK順序、クールダウン、タイムアウトが含まれます。

Tab5向けのコンパイル確認:

```sh
pio run
```

## 実装構成

```text
include/app_config.hpp      初期設定
include/game.hpp            配布デッキと作成中文章
include/protocol.hpp        CLI互換BLEプロトコル
include/exchange_state.hpp  対称交換の状態とクールダウン
src/game.cpp                5W1H選択・文章生成
src/protocol.cpp            フレーム生成・復元・検証
src/exchange_state.cpp      Profile / Gift / ACK状態機械
src/main.cpp                ESP-Hosted BLE、役割切替、Tab5 UI
test/test_protocol.cpp      ホスト上の自動テスト
```

Service UUID、Characteristic UUID、フレーム形式は[CLIの通信仕様](../cli/README.md#構成と通信仕様)と同一です。通信はアプリ独自の認証・暗号化を追加していないため、秘密情報の交換には使用しないでください。

## 広場サーバ（画像生成）との連携

Tab5・CLIともに、完成した文章から画像を生成する[../server/](../server/)へは接続していません。
完成した文章をサーバーへ送るのはCLI側の役目です（`--post-url`。[cli/README.md](../cli/README.md#広場サーバ連携)を参照）。
Tab5同士だけで交換が完結した場合は、そのままではサーバーに届きません。CLIを介して交換するか、
`tab5_hiroba.ino`（予備のTab5・ブラウザを公開表示用の"広場"画面として使う。Wi-Fi経由で[../server/](../server/)から取得表示するだけで、5W1H交換には関与しない）を別端末で動かして観客向けに表示してください。
