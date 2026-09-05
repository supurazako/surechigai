# surechigai

近くのPC同士で、実際のBLE接続を使って短いメッセージを交換するRust製CLIです。
両方のPCで同じコマンドを起動すると、待受役（Peripheral）と接続役（Central）をランダムな間隔で切り替えます。
相手のRSSIが閾値以上なら接続し、お互いのメッセージを表示して切断します。Wi-Fiやインターネットは通信に使いません。

## 起動

Rust/Cargoをインストールした、BLE対応のPCを2台用意してください。開発・検証にはRust 1.97系を使用しています。
1台のMacで2プロセスを起動する方法は、2台でのBLE通信確認の代わりにはなりません。

```sh
cargo build --locked
```

PC A:

```sh
./target/debug/surechigai --message "Aです、こんにちは" --rssi-threshold=-65
```

PC B:

```sh
./target/debug/surechigai --message "Bです、こんにちは" --rssi-threshold=-65
```

成功すると両方に次の形式で表示されます。メッセージは起動時に固定されます。

```text
交換成功 peer=<相手のID> 送信="Aです、こんにちは" 受信="Bです、こんにちは"
```

Ctrl+CまたはSIGTERMで終了します。自分が開始したスキャン・広告・接続を終了処理で停止します。

### macOS

- Xcode Command Line Toolsが必要です。未導入なら `xcode-select --install` で導入します。
- Bluetoothを有効にし、初回のBluetoothアクセス要求を許可します。
- 権限エラーの場合は「システム設定 → プライバシーとセキュリティ → Bluetooth」で、実行元のTerminal、iTerm、IDEなどへの許可を確認して再起動します。
- ビルド時にBluetoothの利用目的を含む `Info.plist` を実行ファイルへ埋め込みます。通常のCLIとして起動でき、アプリ化は不要です。
- Mac上でビルドしてください。macOS 26.2 / Apple Siliconで起動・広告・役割切替・終了を確認しています。

### Linux

Debian / Ubuntuでの準備例です。

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libdbus-1-dev bluez dbus
sudo systemctl start bluetooth
bluetoothctl power on
cargo build --locked
```

BlueZのGATTサーバーとLE広告に対応したアダプターが必要です。まずBluetoothアダプター1個の構成で実行してください。
接続側は最初に見つかったアダプター、待受側はBlueZのデフォルトアダプターを使用します。アダプター指定オプションは未実装です。
`NotPermitted` / `AccessDenied` が出る場合は、実行ユーザーに対するBlueZのD-Busポリシーを確認してください。
`NotSupported` は、アダプターの広告対応やBlueZの機能を確認してください。

Docker DesktopのコンテナではホストMacのBluetoothをそのまま使えません。下記のDocker手順はビルドと自動テスト専用です。

## オプション

| オプション | 既定値 | 動作 |
| --- | --- | --- |
| `--message` | 必須 | UTF-8で128バイト以内。日本語・絵文字・空文字も可 |
| `--role` | `auto` | `auto` / `central` / `peripheral` |
| `--rssi-threshold` | `-65` | この値以上のRSSIを観測した相手へ接続 |
| `--role-min-secs` | `3` | 役割継続時間の最小値 |
| `--role-max-secs` | `8` | 役割継続時間の最大値 |
| `--exchange-timeout-secs` | `10` | 1回の交換の制限時間 |
| `--cooldown-secs` | `30` | 同じ相手と再交換するまでの時間。`0`で抑制なし |

初期役割もランダムです。毎回3〜8秒の継続時間を抽選し、交換中は役割切替を延期します。
役割がかみ合わない場合もあるため、発見までの時間は保証されません。
固定役割モードでも同じ継続時間でスキャンや広告を再開します。

RSSIは距離ではありません。壁、端末の向き、アンテナなどで変動します。
`発見` ログを見て調整してください。例えば `-50` は `-65` より条件が厳しく、`-85` は緩くなります。
最新のスキャンイベントを起点にOSが提供するRSSIを参照し、RSSI不明や無効値では接続しません。
閾値判定は接続側で行い、接続済みの交換をRSSI低下で中断することはありません。

## 実機確認

最初は役割を固定すると切り分けやすくなります。

```sh
# PC A
./target/debug/surechigai --role peripheral --message "待受側です"

# PC B
./target/debug/surechigai --role central --message "接続側です" --rssi-threshold=-85
```

1. 両方に相手のメッセージと `交換成功` が出ることを確認します。
2. 両方を再起動して役割を逆にし、同様に確認します。
3. 両方を `auto` で起動し、役割を指定せず交換できることを確認します。
4. 観測RSSIより厳しい閾値で起動し、`見送り` が表示され交換しないことを確認します。閾値を緩めて再度交換します。
5. 近くに置いたまま、成功後30秒未満では再交換せず、その後は再交換できることを確認します。役割切替とスキャンの時間が加わるため、30秒ぴったりにはなりません。
6. 交換開始後に相手を終了し、失敗／タイムアウト後に役割切替へ戻ることを確認します。

同じ相手かどうかは起動ごとのランダムUUIDで判断します。再起動した相手は別の相手として扱い、OSのMACアドレスに依存しません。
2台で初期化に成功しても発見できない場合は、まず固定役割・緩いRSSI閾値で確認してください。

## テスト

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

Linux向けのコンパイルと自動テスト:

```sh
docker build -f Dockerfile.check -t surechigai-linux-check .
docker run --rm surechigai-linux-check
```

検証状況（2026-09-05）:

- macOS / Apple Silicon: ビルド、自動テスト、Clippy、BLE待受の広告停止・再開、自動役割切替、Ctrl+C終了を確認。
- Linux / Debian 12 / ARM64コンテナ: ビルド・自動テストを確認。Bluetooth実機での通信は未確認。
- Mac 2台間のBLEデータ交換、実距離に対するRSSI調整、途中切断からの実機復帰は未確認。上記の実機手順で確認する必要があります。

## 構成と通信仕様

`src/ble.rs` がBLE入出力、`src/state.rs` が待受の交換状態と再交換抑制、`src/protocol.rs` がフレーム処理を担当します。
接続側は `btleplug 0.13`、待受側は `ble-peripheral-rust 0.2` を使用し、依存バージョンを `Cargo.lock` で固定しています。

Service UUIDは `478f5400-73ad-47a6-a131-562697033a90` です。

| Characteristic UUID | 属性 | 用途 |
| --- | --- | --- |
| `478f5401-73ad-47a6-a131-562697033a90` | Read | バージョン1バイト＋端末UUID16バイト |
| `478f5402-73ad-47a6-a131-562697033a90` | Write with response | データ、返信フレーム選択、受信確認 |
| `478f5403-73ad-47a6-a131-562697033a90` | Read | 選択済みの返信フレーム |

共通ヘッダーは `[version:u8=1, kind:u8, exchange_id:u32 LE]` の6バイトです。

- DATA (`kind=1`): ヘッダー＋連番1バイト＋総数1バイト＋最大12バイトのデータ。連番は0始まり、最終フレーム以外は20バイト固定。
- SELECT (`kind=2`): ヘッダー＋読み出す返信フレームの連番1バイト。
- ACK (`kind=3`): ヘッダーのみ。

データ本体は `[端末UUID16バイト, テキスト長:u16 LE, UTF-8テキスト]` です。
接続側がDATAを全て書き込み、待受側の返信をSELECT → Readで順に取得し、全体を検証してからACKを書き込みます。
同じSELECTに対するReadは同じ値を返し、Readのたびに進むカーソルにはしていません。
待受側は全返信フレームの読出とACKを確認して成功を記録し、接続側はACKへの書込応答を受けて成功を記録します。

待受側のライブラリには汎用の接続・切断イベントがないため、最初のDATAからタイムアウトまでを交換中として扱います。
同時交換は1件で、別の交換要求は拒否します。Linuxでは広告停止でGATTサービスも解除されるため、交換中は広告を維持します。
ACK処理後は応答送出の猶予として1秒間サービスを維持します。
最後のACK応答が途中切断で失われると、一方だけが成功を記録する場合があります。初版では永続的な配達保証や厳密な一度限りの交換は扱いません。

履歴保存、常駐、スマートフォン対応、アプリ独自の認証・暗号化は未実装です。
