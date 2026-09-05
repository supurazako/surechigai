// Tab5 側: 広場の表示。PC の server.py から最新の画像と文章を取って全画面に出す。
//
// 必要なライブラリ: M5Unified（M5GFX 同梱）, ArduinoJson (v7)
// ボード: M5Stack Tab5（arduino-esp32 3.x。Wi-Fi は内蔵の ESP32-C6 経由で WiFi.h がそのまま使える想定）
// ※ 実機未検証。Wi-Fi がつながらない場合は arduino-esp32 のバージョンを M5Stack の Tab5 サンプルに合わせる。
//
// 動き:
//   3 秒ごとに GET /latest.json → 新しい完成があれば GET /image/N.jpg を取って左に描き、右に文章
//   新着が無い間は、直近の完成分を 10 秒ごとに順番に見せる

#include <M5Unified.h>
#include <WiFi.h>
#include <HTTPClient.h>
#include <ArduinoJson.h>
#include "include/gallery_rotation.hpp"

static const char* WIFI_SSID = "your-hotspot";
static const char* WIFI_PASS = "your-password";
static const char* SERVER    = "http://192.168.43.1:8000";  // PC の IP に書き換える

static const uint32_t POLL_MS   = 3000;

struct Item { int id; String device; String sentence; String image; };
static Item items[20];
static int  itemCount = 0;
static int  shownId   = -1;
static surechigai::GalleryRotation rotation;
static uint32_t lastPoll = 0;

static int W, H;  // 画面サイズ（横向きで 1280x720 を想定）

static bool ensureWifi(uint32_t waitMs = 10000) {
  if (WiFi.status() == WL_CONNECTED) return true;
  WiFi.mode(WIFI_STA);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  uint32_t t0 = millis();
  while (WiFi.status() != WL_CONNECTED && millis() - t0 < waitMs) delay(100);
  return WiFi.status() == WL_CONNECTED;
}

static void status(const String& msg) {
  M5.Display.fillRect(0, H - 40, W, 40, TFT_BLACK);
  M5.Display.setFont(&fonts::lgfxJapanGothic_24);
  M5.Display.setTextColor(TFT_DARKGREY, TFT_BLACK);
  M5.Display.setCursor(16, H - 32);
  M5.Display.print(msg);
}

// /latest.json を読んで items を更新。done のものだけ残す。
static bool fetchLatest() {
  HTTPClient http;
  http.setTimeout(4000);
  http.begin(String(SERVER) + "/latest.json");
  int code = http.GET();
  if (code != 200) { http.end(); return false; }
  JsonDocument doc;
  DeserializationError err = deserializeJson(doc, http.getStream());
  http.end();
  if (err) return false;
  itemCount = 0;
  for (JsonObject o : doc["items"].as<JsonArray>()) {
    if (itemCount >= 20) break;
    if (String(o["status"].as<const char*>()) != "done" || o["image"].isNull()) continue;
    items[itemCount].id       = o["id"].as<int>();
    items[itemCount].device   = o["device"].as<const char*>();
    items[itemCount].sentence = o["sentence"].as<const char*>();
    items[itemCount].image    = o["image"].as<const char*>();
    itemCount++;
  }
  return true;
}

// 画像を取ってきて左側 720x720 に描き、右側に文章
static bool show(const Item& it) {
  HTTPClient http;
  http.setTimeout(8000);
  http.begin(String(SERVER) + it.image);
  int code = http.GET();
  if (code != 200) { http.end(); status("画像の取得に失敗 #" + String(it.id)); return false; }
  int len = http.getSize();
  if (len <= 0 || len > 600000) { http.end(); status("画像サイズが変"); return false; }
  uint8_t* buf = (uint8_t*)heap_caps_malloc(len, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
  if (!buf) buf = (uint8_t*)malloc(len);
  if (!buf) { http.end(); status("メモリ不足"); return false; }
  WiFiClient* s = http.getStreamPtr();
  int got = 0;
  uint32_t t0 = millis();
  while (got < len && millis() - t0 < 8000) {
    int n = s->read(buf + got, len - got);
    if (n > 0) got += n; else delay(5);
  }
  http.end();

  if (got != len) { free(buf); status("画像の受信が途中で止まりました"); return false; }

  M5.Display.fillScreen(TFT_BLACK);
  int side = H;  // 正方形を画面の高さいっぱいに
  float scale = (float)side / 1024.0f;
  M5.Display.drawJpg(buf, got, 0, 0, side, side, 0, 0, scale, scale);
  free(buf);

  // 右側: 文章を 1 語 1 行で
  int x = side + 32, y = 48;
  M5.Display.setFont(&fonts::lgfxJapanGothic_40);
  M5.Display.setTextColor(TFT_WHITE, TFT_BLACK);
  String s2 = it.sentence;
  int start = 0;
  while (start < (int)s2.length()) {
    int sp = s2.indexOf("　", start);  // 全角スペース（UTF-8 で 3 バイト）
    String line = sp < 0 ? s2.substring(start) : s2.substring(start, sp);
    M5.Display.setCursor(x, y);
    M5.Display.print(line);
    y += 56;
    if (sp < 0) break;
    start = sp + 3;
  }
  M5.Display.setFont(&fonts::lgfxJapanGothic_24);
  M5.Display.setTextColor(TFT_DARKGREY, TFT_BLACK);
  M5.Display.setCursor(x, H - 80);
  M5.Display.print("#" + String(it.id) + "  " + it.device);
  shownId = it.id;
  return true;
}

void setup() {
  auto cfg = M5.config();
  M5.begin(cfg);
  M5.Display.setRotation(1);  // 横向き
  W = M5.Display.width();
  H = M5.Display.height();
  M5.Display.fillScreen(TFT_BLACK);
  M5.Display.setFont(&fonts::lgfxJapanGothic_40);
  M5.Display.setTextColor(TFT_WHITE, TFT_BLACK);
  M5.Display.setCursor(48, 48);
  M5.Display.print("すれ違い広場");
  status("Wi-Fi 接続中…");
  if (ensureWifi()) status("接続: " + WiFi.localIP().toString() + "  サーバ待ち");
  else status("Wi-Fi につながらない。SSID/PASS を確認");
}

void loop() {
  M5.update();
  uint32_t now = millis();
  if (now - lastPoll >= POLL_MS) {
    lastPoll = now;
    if (ensureWifi() && fetchLatest()) {
      if (itemCount == 0 && shownId < 0) {
        status("まだ誰も完成していない");
      }
    } else {
      status("サーバに届かない: " + String(SERVER));
    }
  }
  const int selected = rotation.select(itemCount > 0 ? items[0].id : -1, itemCount, now);
  if (selected >= 0 && !show(items[selected])) rotation.failed(millis());
  delay(20);
}
