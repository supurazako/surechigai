// AtomS3R 側: 文章が完成したら PC の広場サーバへ POST する部分。
// 既存のファームにこのファイルの関数を足して、完成した瞬間に postSentence(...) を 1 回呼ぶ。
//
// 必要な設定は下の 4 行だけ。SSID/PASS はスマホのテザリング、SERVER は PC の LAN IP（ipconfig の IPv4）。
// BLE と Wi-Fi は ESP32-S3 で同時に使える（時分割なので BLE スキャンが少し鈍る程度）。
// Wi-Fi 接続は最初の 1 回だけ 2〜5 秒かかる。setup() で WiFi.begin() を先に呼んでおくと完成時に待たない。

#include <WiFi.h>
#include <HTTPClient.h>

static const char* WIFI_SSID = "your-hotspot";
static const char* WIFI_PASS = "your-password";
static const char* SERVER    = "http://192.168.43.1:8000";  // PC の IP に書き換える
static const char* DEVICE_ID = "A";                          // 台ごとに変える

// JSON の中に入れる文字列の " と \ を逃がす（日本語はそのままで良い）
static String jsonEscape(const String& s) {
  String o;
  o.reserve(s.length() + 8);
  for (size_t i = 0; i < s.length(); i++) {
    char c = s[i];
    if (c == '"' || c == '\\') o += '\\';
    if (c == '\n') { o += "\\n"; continue; }
    o += c;
  }
  return o;
}

static bool ensureWifi(uint32_t waitMs = 8000) {
  if (WiFi.status() == WL_CONNECTED) return true;
  WiFi.mode(WIFI_STA);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  uint32_t t0 = millis();
  while (WiFi.status() != WL_CONNECTED && millis() - t0 < waitMs) delay(100);
  return WiFi.status() == WL_CONNECTED;
}

// 完成した文章（6 語を全角スペース "　" で連結したもの）を送る。成功で true。
// サーバは即 {"id":12,"status":"queued"} を返し、画像生成は裏で進む。
bool postSentence(const String& sentence) {
  if (!ensureWifi()) return false;
  HTTPClient http;
  http.setTimeout(5000);
  http.begin(String(SERVER) + "/submit");
  http.addHeader("Content-Type", "application/json");
  String body = String("{\"device\":\"") + DEVICE_ID + "\",\"sentence\":\"" + jsonEscape(sentence) + "\"}";
  int code = http.POST(body);
  String res = http.getString();
  http.end();
  Serial.printf("POST /submit -> %d %s\n", code, res.c_str());
  return code == 200;
}

// 6 語を別々に持っているならこちら。サーバ側で全角スペース連結する。
bool postWords(const String words[], size_t n) {
  if (!ensureWifi()) return false;
  String arr = "[";
  for (size_t i = 0; i < n; i++) {
    if (i) arr += ",";
    arr += "\"" + jsonEscape(words[i]) + "\"";
  }
  arr += "]";
  HTTPClient http;
  http.setTimeout(5000);
  http.begin(String(SERVER) + "/submit");
  http.addHeader("Content-Type", "application/json");
  String body = String("{\"device\":\"") + DEVICE_ID + "\",\"words\":" + arr + "}";
  int code = http.POST(body);
  http.end();
  return code == 200;
}

// 単体で動作確認するときの最小スケッチ。既存ファームに混ぜるときは setup/loop は消す。
#ifdef ATOM_POST_STANDALONE
void setup() {
  Serial.begin(115200);
  delay(500);
  ensureWifi();
  Serial.println(WiFi.localIP());
  postSentence("暇だったので　真夜中に　パリで　従順な犬が　全力で　ラーメンを食べた");
}
void loop() { delay(1000); }
#endif
