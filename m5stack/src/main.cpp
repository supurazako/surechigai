#include <Arduino.h>
#include <BLEAdvertisedDevice.h>
#include <BLEAdvertising.h>
#include <BLEClient.h>
#include <BLEDevice.h>
#include <BLERemoteCharacteristic.h>
#include <BLERemoteService.h>
#include <BLEScan.h>
#include <BLEServer.h>
#include <M5Unified.h>
#include <esp_random.h>
#include <host/ble_gap.h>

#include "app_config.hpp"
#include "exchange_state.hpp"
#include "protocol.hpp"

#include <algorithm>
#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace surechigai {
namespace {

using config::Role;
using protocol::GiftPacket;
using protocol::Frame;
using protocol::NodeId;
using protocol::Packet;
using protocol::Profile;

constexpr std::uint32_t kBackground = 0x101820U;
constexpr std::uint32_t kPanel = 0x1b2835U;
constexpr std::uint32_t kPanelLight = 0x25384aU;
constexpr std::uint32_t kPrimary = 0x40d9b0U;
constexpr std::uint32_t kAccent = 0xffbe55U;
constexpr std::uint32_t kDanger = 0xff6b70U;
constexpr std::uint32_t kText = 0xf3f7faU;
constexpr std::uint32_t kMuted = 0xa4b4c2U;

bool reached(std::uint32_t now, std::uint32_t deadline) {
  return static_cast<std::int32_t>(now - deadline) >= 0;
}

std::uint32_t random_between(std::uint32_t minimum, std::uint32_t maximum) {
  return minimum + esp_random() % (maximum - minimum + 1);
}

std::uint8_t bit_count(std::uint8_t value) {
  std::uint8_t count = 0;
  while (value != 0) {
    count += value & 1U;
    value >>= 1U;
  }
  return count;
}

NodeId random_node_id() {
  NodeId node{};
  esp_fill_random(node.data(), node.size());
  node[6] = static_cast<std::uint8_t>((node[6] & 0x0fU) | 0x40U);
  node[8] = static_cast<std::uint8_t>((node[8] & 0x3fU) | 0x80U);
  return node;
}

const char* role_name(Role role) {
  switch (role) {
    case Role::Auto:
      return "自動";
    case Role::Central:
      return "接続";
    case Role::Peripheral:
      return "待受";
  }
  return "?";
}

class App;

class RxCallbacks final : public BLECharacteristicCallbacks {
 public:
  explicit RxCallbacks(App& app) : app_(app) {}
  void onWrite(BLECharacteristic* characteristic,
               ble_gap_conn_desc* connection) override;

 private:
  App& app_;
};

class TxCallbacks final : public BLECharacteristicCallbacks {
 public:
  explicit TxCallbacks(App& app) : app_(app) {}
  void onRead(BLECharacteristic* characteristic,
              ble_gap_conn_desc* connection) override;

 private:
  App& app_;
};

class ServerCallbacks final : public BLEServerCallbacks {
 public:
  explicit ServerCallbacks(App& app) : app_(app) {}
  void onConnect(BLEServer* server, ble_gap_conn_desc* connection) override;
  void onDisconnect(BLEServer* server, ble_gap_conn_desc* connection) override;

 private:
  App& app_;
};

class ScanCallbacks final : public BLEAdvertisedDeviceCallbacks {
 public:
  explicit ScanCallbacks(App& app) : app_(app) {}
  void onResult(BLEAdvertisedDevice device) override;

 private:
  App& app_;
};

class App {
 public:
  App()
      : rx_callbacks_(*this),
        tx_callbacks_(*this),
        server_callbacks_(*this),
        scan_callbacks_(*this) {}

  bool begin() {
    canvas_.setColorDepth(16);
    if (canvas_.createSprite(M5.Display.width(), M5.Display.height()) == nullptr) {
      fatal_error_ = "画面バッファを確保できません";
      return false;
    }
    const NodeId node = random_node_id();
    const game::Uuid round = random_node_id();
    game::Deck deck({config::WHEN, config::WHERE, config::WHO, config::WHAT,
                     config::WHY, config::HOW});
    std::string validation_error;
    if (!protocol::valid_utf8(config::NAME) || !deck.validate(validation_error)) {
      fatal_error_ = "名前または配布デッキの設定が不正です";
      return false;
    }
    state_ = std::make_unique<ExchangeState>(
        node, config::NAME, std::move(deck), round,
        config::EXCHANGE_TIMEOUT_SECONDS * 1000U,
        config::COOLDOWN_SECONDS * 1000U);
    role_mode_ = config::INITIAL_ROLE;
    active_role_ = role_mode_ == Role::Auto
                       ? ((esp_random() & 1U) ? Role::Central : Role::Peripheral)
                       : role_mode_;
    rssi_threshold_ = config::RSSI_THRESHOLD;

    Serial.printf(
        "ユーザー=%s 自分のID=%s 配布=[いつ:\"%s\", どこで:\"%s\", "
        "だれが:\"%s\", なにをする:\"%s\", なぜ:\"%s\", "
        "どのように:\"%s\"] RSSI閾値=%ddBm\n",
        config::NAME, protocol::node_to_string(node).c_str(), config::WHEN,
        config::WHERE, config::WHO, config::WHAT, config::WHY, config::HOW,
        rssi_threshold_);
    set_status("BLEを初期化しています…");
    draw();

    if (!BLEDevice::init("surechigai")) {
      fatal_error_ = "BLE初期化に失敗しました";
      return false;
    }

    server_ = BLEDevice::createServer();
    if (server_ == nullptr) {
      fatal_error_ = "GATTサーバーを作成できません";
      return false;
    }
    server_->setCallbacks(&server_callbacks_);
    BLEService* service = server_->createService(protocol::SERVICE_UUID);
    if (service == nullptr) {
      fatal_error_ = "GATTサービスを作成できません";
      return false;
    }
    info_ = service->createCharacteristic(protocol::INFO_UUID,
                                          BLECharacteristic::PROPERTY_READ);
    rx_ = service->createCharacteristic(protocol::RX_UUID,
                                        BLECharacteristic::PROPERTY_WRITE);
    tx_ = service->createCharacteristic(protocol::TX_UUID,
                                        BLECharacteristic::PROPERTY_READ);
    if (info_ == nullptr || rx_ == nullptr || tx_ == nullptr) {
      fatal_error_ = "Characteristicを作成できません";
      return false;
    }
    const Frame own_identity = protocol::identity(node);
    info_->setValue(own_identity.data(), own_identity.size());
    rx_->setCallbacks(&rx_callbacks_);
    tx_->setCallbacks(&tx_callbacks_);
    service->start();

    advertising_ = BLEDevice::getAdvertising();
    advertising_->addServiceUUID(protocol::SERVICE_UUID);
    advertising_->setScanResponse(true);
    advertising_->setMinPreferred(0x06);
    advertising_->setMaxPreferred(0x12);

    scan_ = BLEDevice::getScan();
    scan_->setAdvertisedDeviceCallbacks(&scan_callbacks_, false, true);
    // Hosted HCIの安定性と消費電力を優先してパッシブスキャンを使う。
    scan_->setActiveScan(false);
    scan_->setInterval(100);
    scan_->setWindow(80);

    client_ = BLEDevice::createClient();
    if (client_ == nullptr) {
      fatal_error_ = "GATTクライアントを作成できません";
      return false;
    }
    set_status("準備完了");
    add_log("BLEの初期化が完了しました");
    return true;
  }

  void loop_once() {
    drain_events();
    update_ui();
    if (!running_) {
      active_role_ = Role::Auto;
      set_status("停止中");
      slot_deadline_ = 0;
      draw_if_needed();
      delay(20);
      return;
    }

    active_role_ = role_mode_ == Role::Auto ? active_role_ : role_mode_;
    if (active_role_ == Role::Auto) {
      active_role_ = (esp_random() & 1U) ? Role::Central : Role::Peripheral;
    }
    const std::uint32_t seconds =
        random_between(config::ROLE_MIN_SECONDS, config::ROLE_MAX_SECONDS);
    slot_deadline_ = millis() + seconds * 1000U;
    role_change_requested_ = false;
    Serial.printf("役割=%s 継続時間=%lu秒\n", role_name(active_role_),
                  static_cast<unsigned long>(seconds));

    if (active_role_ == Role::Peripheral) {
      peripheral_slot();
    } else {
      central_slot();
    }
    stop_radio_activity();
    slot_deadline_ = 0;

    if (role_mode_ == Role::Auto) {
      active_role_ = active_role_ == Role::Central ? Role::Peripheral
                                                   : Role::Central;
    } else {
      active_role_ = role_mode_;
    }
  }

  const std::string& fatal_error() const { return fatal_error_; }

  void on_server_write(std::uint16_t client, BLECharacteristic* characteristic) {
    const String value = characteristic->getValue();
    std::string error;
    const auto* bytes = reinterpret_cast<const std::uint8_t*>(value.c_str());
    if (!state_->write(client, bytes, value.length(), millis(), error)) {
      Serial.printf("書込見送り: %s\n", error.c_str());
      return;
    }
    // SELECTのWrite Responseより先に値を用意する。Read時のコールバックでは
    // 同じ値を返しつつ、実際に読み出されたことを状態機械へ記録する。
    protocol::Header header;
    if (protocol::parse_header(bytes, value.length(), header, error) &&
        header.kind == protocol::SELECT) {
      Frame selected;
      if (state_->selected_value(client, millis(), selected, error)) {
        tx_->setValue(selected.data(), selected.size());
      }
    }
  }

  void on_server_read(std::uint16_t client, BLECharacteristic* characteristic) {
    Frame value;
    std::string error;
    if (state_->read(client, millis(), value, error)) {
      characteristic->setValue(value.data(), value.size());
    } else {
      characteristic->setValue(static_cast<const std::uint8_t*>(nullptr), 0);
      Serial.printf("読出見送り: %s\n", error.c_str());
    }
  }

  void on_server_connect(std::uint16_t client) {
    state_->connected(client, millis());
    Serial.printf("接続されました client=%u\n", client);
  }

  void on_server_disconnect(std::uint16_t client) {
    state_->disconnected(client);
    Serial.printf("切断されました client=%u\n", client);
  }

  void on_advertised_device(BLEAdvertisedDevice& device) {
    if (!device.haveServiceUUID() ||
        !device.isAdvertisingService(BLEUUID(protocol::SERVICE_UUID)) ||
        !device.haveRSSI()) {
      return;
    }
    const int rssi = device.getRSSI();
    const std::string address = device.getAddress().toString().c_str();
    bool cooling = false;
    for (const KnownPeer& peer : known_peers_) {
      if (peer.address == address && state_->cooling_down(peer.node, millis())) {
        cooling = true;
        break;
      }
    }
    Serial.printf("発見 device=%s RSSI=%ddBm %s\n", address.c_str(), rssi,
                  cooling ? "見送り（再交換待ち）"
                          : (rssi >= rssi_threshold_ ? "接続候補"
                                                    : "見送り（閾値未満）"));
    if (cooling || rssi < rssi_threshold_) return;

    std::lock_guard<std::mutex> lock(found_mutex_);
    if (!found_device_) {
      found_device_ = std::make_unique<BLEAdvertisedDevice>(device);
      if (scan_ != nullptr && scan_->isScanning()) scan_->stop();
    }
  }

 private:
  struct KnownPeer {
    std::string address;
    NodeId node{};
  };

  struct Button {
    int x;
    int y;
    int w;
    int h;

    bool contains(int px, int py) const {
      return px >= x && px < x + w && py >= y && py < y + h;
    }
  };

  void peripheral_slot() {
    state_->enable();
    if (!advertising_->start()) {
      state_->shutdown();
      communication_error("広告を開始できません");
      return;
    }
    advertising_active_ = true;
    set_status("近くの端末を待っています");
    draw_if_needed();

    while (running_) {
      state_->expire(millis());
      drain_events();
      update_ui();
      draw_if_needed();
      if ((role_change_requested_ || reached(millis(), slot_deadline_)) &&
          state_->disable_if_idle(millis())) {
        break;
      }
      delay(20);
    }
    if (!running_) state_->shutdown();
  }

  void central_slot() {
    set_status("近くの端末を探しています");
    draw_if_needed();
    while (running_ && !role_change_requested_ &&
           !reached(millis(), slot_deadline_)) {
      {
        std::lock_guard<std::mutex> lock(found_mutex_);
        found_device_.reset();
      }
      scan_->clearResults();
      scanning_active_ = true;
      BLEScanResults* result = scan_->start(1, false);
      (void)result;
      scanning_active_ = scan_->isScanning();
      update_ui();
      drain_events();
      draw_if_needed();

      std::unique_ptr<BLEAdvertisedDevice> device;
      {
        std::lock_guard<std::mutex> lock(found_mutex_);
        device = std::move(found_device_);
      }
      if (device) {
        exchange_with(*device);
        return;
      }
    }
  }

  bool exchange_with(BLEAdvertisedDevice& device) {
    const std::string address = device.getAddress().toString().c_str();
    set_status("接続しています…");
    draw_if_needed();
    const std::uint32_t timeout_ms = config::EXCHANGE_TIMEOUT_SECONDS * 1000U;
    if (!client_->connectTimeout(&device, timeout_ms)) {
      disconnect_client();
      communication_error("BLE接続に失敗しました");
      return false;
    }
    client_connected_ = true;

    bool succeeded = false;
    bool retry_after_role_change = false;
    std::string error;
    do {
      BLERemoteService* service = client_->getService(protocol::SERVICE_UUID);
      if (service == nullptr) {
        error = "サービスが見つかりません";
        break;
      }
      BLERemoteCharacteristic* info =
          service->getCharacteristic(protocol::INFO_UUID);
      BLERemoteCharacteristic* rx = service->getCharacteristic(protocol::RX_UUID);
      BLERemoteCharacteristic* tx = service->getCharacteristic(protocol::TX_UUID);
      if (info == nullptr || rx == nullptr || tx == nullptr || !info->canRead() ||
          !rx->canWrite() || !tx->canRead()) {
        error = "必要なCharacteristicがありません";
        break;
      }

      const std::uint32_t exchange = esp_random();
      const Profile own_profile = state_->profile();
      std::vector<Frame> profile_frames;
      if (!protocol::make_frames(own_profile, exchange, profile_frames, error) ||
          profile_frames.empty()) {
        if (error.empty()) error = "Profileフレームを作成できません";
        break;
      }

      // PC版の待受側は汎用の接続イベントを取得できないため、最初のDATAを
      // 受け取った時点からロール切替を保留する。先にINFOを読むと、その応答と
      // DATA書込みの間で待受時間が終了し、ATT 0x06になる競合が生じる。
      // 先頭DATAで交換を確保してから相手IDを検証し、残りを送信する。
      set_status("5W1Hを交換しています");
      draw_if_needed();
      if (!rx->writeValue(profile_frames.front().data(),
                          profile_frames.front().size(), true)) {
        error = "相手の待受終了を検出しました";
        retry_after_role_change = role_mode_ == Role::Auto;
        break;
      }

      const String identity = info->readValue();
      NodeId node{};
      if (!protocol::parse_identity(
              reinterpret_cast<const std::uint8_t*>(identity.c_str()),
              identity.length(), node, error)) {
        break;
      }
      if (node == state_->node()) {
        error = "自分自身への接続です";
        break;
      }
      if (state_->cooling_down(node, millis())) {
        error = "再交換待ちの相手です";
        break;
      }

      Serial.printf("交換開始 role=central peer=%s exchange=%08lx\n",
                    protocol::node_to_string(node).c_str(),
                    static_cast<unsigned long>(exchange));
      bool write_ok = true;
      for (std::size_t index = 1; index < profile_frames.size(); ++index) {
        Frame& frame = profile_frames[index];
        if (!rx->writeValue(frame.data(), frame.size(), true)) {
          error = "Profile書込に失敗しました";
          write_ok = false;
          break;
        }
      }
      if (!write_ok) break;

      Packet reply;
      if (!read_packet(rx, tx, exchange, reply, error)) {
        if (error.empty()) error = "Profile返信読出に失敗しました";
        break;
      }
      const auto* profile_reply = std::get_if<Profile>(&reply);
      if (profile_reply == nullptr) {
        error = "Profileではない返信を受信しました";
        break;
      }
      const Profile peer_profile = *profile_reply;
      if (peer_profile.node != node) {
        error = "相手IDが交換途中で変わりました";
        break;
      }

      const GiftPacket sent = state_->choose_gift(peer_profile, esp_random());
      if (!write_packet(rx, sent, exchange, error)) {
        if (error.empty()) error = "Gift書込に失敗しました";
        break;
      }
      if (!read_packet(rx, tx, exchange, reply, error)) {
        if (error.empty()) error = "Gift返信読出に失敗しました";
        break;
      }
      const auto* received = std::get_if<GiftPacket>(&reply);
      if (received == nullptr) {
        error = "Giftではない返信を受信しました";
        break;
      }
      if (received->receiver_round != own_profile.round) {
        error = "受け取ったGiftが別の文章を対象にしています";
        break;
      }
      if (received->gift &&
          (own_profile.missing & game::slot_bit(received->gift->slot)) == 0) {
        error = "既に所持している種類のGiftです";
        break;
      }
      Frame ack = protocol::command(protocol::ACK, exchange);
      if (!rx->writeValue(ack.data(), ack.size(), true)) {
        error = "受信確認に失敗しました";
        break;
      }
      if (!state_->record_central(peer_profile, sent, *received, millis(),
                                  error)) {
        break;
      }
      remember_peer(address, node);
      succeeded = true;
    } while (false);

    disconnect_client();
    if (!succeeded) {
      if (retry_after_role_change) {
        communication_retry(error);
      } else {
        communication_error(error.empty() ? "交換に失敗しました" : error);
      }
    }
    drain_events();
    return succeeded;
  }

  bool write_packet(BLERemoteCharacteristic* rx, const Packet& packet,
                    std::uint32_t exchange, std::string& error) {
    std::vector<Frame> frames;
    if (!protocol::make_frames(packet, exchange, frames, error)) return false;
    for (Frame& frame : frames) {
      if (!rx->writeValue(frame.data(), frame.size(), true)) {
        error = "DATA書込に失敗しました";
        return false;
      }
    }
    error.clear();
    return true;
  }

  bool read_packet(BLERemoteCharacteristic* rx,
                   BLERemoteCharacteristic* tx, std::uint32_t exchange,
                   Packet& packet, std::string& error) {
    protocol::Assembler assembler;
    for (std::size_t index = 0; index < protocol::MAX_FRAMES; ++index) {
      Frame select = protocol::command(protocol::SELECT, exchange);
      select.push_back(static_cast<std::uint8_t>(index));
      if (!rx->writeValue(select.data(), select.size(), true)) {
        error = "返信フレームの選択に失敗しました";
        return false;
      }
      const String raw = tx->readValue();
      Frame frame(reinterpret_cast<const std::uint8_t*>(raw.c_str()),
                  reinterpret_cast<const std::uint8_t*>(raw.c_str()) +
                      raw.length());
      if (!protocol::require_exchange(frame, exchange, error)) return false;
      const auto result = assembler.push(frame, packet, error);
      if (result == protocol::PushResult::Error) return false;
      if (result == protocol::PushResult::Complete) return true;
    }
    error = "返信フレーム数が上限を超えました";
    return false;
  }

  void disconnect_client() {
    if (client_ == nullptr) return;

    // Hosted NimBLEではMTU交換がBLE_HS_EALREADYで失敗すると、connect()は
    // falseを返す一方で接続ハンドルだけが残ることがある。isConnected()は
    // falseのため通常のdisconnect()では切れず、以後ずっとClient busyになる。
    const std::uint16_t connection = client_->getConnId();
    if (connection != BLE_HS_CONN_HANDLE_NONE) {
      const int result =
          ble_gap_terminate(connection, BLE_ERR_REM_USER_CONN_TERM);
      if (result != 0 && result != BLE_HS_ENOTCONN &&
          result != BLE_HS_EALREADY) {
        Serial.printf("BLE切断要求に失敗しました rc=%d\n", result);
      }
    } else if (client_->isConnected()) {
      client_->disconnect();
    }
    client_connected_ = false;
  }

  void remember_peer(const std::string& address, const NodeId& node) {
    for (KnownPeer& peer : known_peers_) {
      if (peer.address == address) {
        peer.node = node;
        return;
      }
    }
    known_peers_.push_back({address, node});
    if (known_peers_.size() > 32) known_peers_.erase(known_peers_.begin());
  }

  void stop_radio_activity() {
    if (scan_ != nullptr && (scanning_active_ || scan_->isScanning())) {
      scan_->stop();
    }
    scanning_active_ = false;
    if (client_ != nullptr &&
        (client_connected_ || client_->isConnected() ||
         client_->getConnId() != BLE_HS_CONN_HANDLE_NONE)) {
      disconnect_client();
    }
    if (advertising_ != nullptr && advertising_active_) advertising_->stop();
    advertising_active_ = false;
    state_->shutdown();
  }

  void drain_events() {
    bool exchanged = false;
    while (state_) {
      std::optional<ExchangeEvent> event = state_->take_event();
      if (!event) break;
      latest_peer_ = event->peer.name + " (" +
                     protocol::node_to_string(event->peer.node) + ")";
      latest_sent_ = gift_label(event->sent);
      latest_received_ = gift_label(event->received);
      ++exchange_count_;
      set_status("交換に成功しました");
      latest_exchange_role_ = event->as_central ? "接続" : "待受";
      add_log(latest_exchange_role_ + "で交換: 受取=" + latest_received_);
      Serial.printf(
          "交換成功 peer=%s 配布=%s 受取=%s 作成中=\"%s\" 残り=%u\n",
          latest_peer_.c_str(), latest_sent_.c_str(), latest_received_.c_str(),
          event->sentence.render().c_str(),
          bit_count(event->sentence.missing_mask()));
      if (event->sentence.is_complete()) {
        Serial.printf("文章完成 round=%s 文=\"%s\"\n",
                      protocol::node_to_string(event->sentence.round()).c_str(),
                      event->sentence.render().c_str());
        set_status("文章が完成しました");
      }
      exchanged = true;
    }
    // 成功内容は次のロール更新を待たず、その場で画面へ反映する。
    if (exchanged) draw_if_needed();
  }

  void communication_error(const std::string& message) {
    Serial.printf("通信失敗: %s\n", message.c_str());
    set_status("通信失敗: " + message);
    add_log("失敗: " + message);
  }

  void communication_retry(const std::string& message) {
    Serial.printf("通信再試行: %s\n", message.c_str());
    set_status("相手が切替中・再試行します");
    add_log("再試行: " + message);
  }

  void set_status(std::string status) {
    if (status_ == status) return;
    status_ = std::move(status);
    dirty_ = true;
  }

  void add_log(std::string log) {
    logs_.push_back(std::move(log));
    while (logs_.size() > 3) logs_.erase(logs_.begin());
    dirty_ = true;
  }

  void update_ui() {
    M5.update();
    if (M5.Touch.getCount() > 0) {
      const auto touch = M5.Touch.getDetail();
      if (touch.wasClicked()) handle_touch(touch.x, touch.y);
    }
  }

  void handle_touch(int x, int y) {
    const int width = M5.Display.width();
    const int height = M5.Display.height();
    const int button_y = height - 88;
    const Button run{32, button_y, width * 28 / 100, 64};
    const Button role{width * 33 / 100, button_y, width * 28 / 100, 64};
    const Button minus{width * 64 / 100, button_y, width * 10 / 100, 64};
    const Button plus{width * 87 / 100, button_y, width * 10 / 100, 64};
    if (run.contains(x, y)) {
      running_ = !running_;
      role_change_requested_ = true;
      add_log(running_ ? "動作を再開しました" : "動作を停止しました");
    } else if (role.contains(x, y)) {
      role_mode_ = role_mode_ == Role::Auto
                       ? Role::Central
                       : (role_mode_ == Role::Central ? Role::Peripheral
                                                      : Role::Auto);
      role_change_requested_ = true;
      add_log(std::string("役割設定: ") + role_name(role_mode_));
    } else if (minus.contains(x, y)) {
      rssi_threshold_ = std::max(-127, rssi_threshold_ - 5);
      dirty_ = true;
    } else if (plus.contains(x, y)) {
      rssi_threshold_ = std::min(20, rssi_threshold_ + 5);
      dirty_ = true;
    }
  }

  void draw_if_needed() {
    if (dirty_) draw();
  }

  void draw_button(const Button& button, std::uint32_t color,
                   const char* label) {
    canvas_.fillRoundRect(button.x, button.y, button.w, button.h, 16, color);
    canvas_.setTextColor(kText, color);
    canvas_.setTextDatum(middle_center);
    canvas_.drawString(label, button.x + button.w / 2,
                       button.y + button.h / 2);
    canvas_.setTextDatum(top_left);
  }

  void draw_wrapped(const std::string& text, int x, int y, int width,
                    int max_lines, std::uint32_t color) {
    canvas_.setTextColor(color);
    std::string line;
    std::size_t offset = 0;
    int line_number = 0;
    while (offset < text.size() && line_number < max_lines) {
      const unsigned char first = static_cast<unsigned char>(text[offset]);
      std::size_t bytes = first < 0x80U   ? 1
                          : first < 0xe0U ? 2
                          : first < 0xf0U ? 3
                                         : 4;
      bytes = std::min(bytes, text.size() - offset);
      const std::string next = line + text.substr(offset, bytes);
      if (!line.empty() && canvas_.textWidth(next.c_str()) > width) {
        canvas_.drawString(line.c_str(), x, y + line_number * 32);
        line.clear();
        ++line_number;
        if (line_number >= max_lines) break;
      }
      line += text.substr(offset, bytes);
      offset += bytes;
    }
    if (!line.empty() && line_number < max_lines) {
      canvas_.drawString(line.c_str(), x, y + line_number * 32);
    }
  }

  void draw() {
    dirty_ = false;
    const int width = canvas_.width();
    const int height = canvas_.height();
    canvas_.startWrite();
    canvas_.fillScreen(kBackground);
    canvas_.setTextDatum(top_left);
    canvas_.setFont(&fonts::lgfxJapanGothic_32);
    canvas_.setTextColor(kText, kBackground);
    canvas_.drawString("すれちがい", 38, 24);

    canvas_.setFont(&fonts::lgfxJapanGothic_20);
    const std::string id = state_ ? protocol::node_to_string(state_->node())
                                  : "初期化前";
    canvas_.setTextColor(kMuted, kBackground);
    canvas_.drawString(("ID  " + id).c_str(), 42, 76);

    const char* active = !running_ || active_role_ == Role::Auto
                             ? "停止"
                             : role_name(active_role_);
    canvas_.fillRoundRect(width - 300, 25, 258, 65, 20, kPanelLight);
    canvas_.setTextDatum(middle_center);
    canvas_.setTextColor(kPrimary, kPanelLight);
    canvas_.drawString((std::string(active) + " / " + role_name(role_mode_))
                           .c_str(),
                       width - 171, 57);
    canvas_.setTextDatum(top_left);

    const int panel_width = width - 80;
    canvas_.fillRoundRect(40, 120, panel_width, 150, 20, kPanel);
    const game::Sentence current = state_ ? state_->sentence()
                                          : game::Sentence{};
    const std::uint8_t remaining = current.missing_mask();
    canvas_.setTextColor(kPrimary, kPanel);
    const std::string sentence_title =
        "作成中の文章  残り" + std::to_string(bit_count(remaining)) +
        " / ユーザー: " + (state_ ? state_->name() : "—");
    canvas_.drawString(sentence_title.c_str(), 65, 140);
    canvas_.setFont(&fonts::lgfxJapanGothic_24);
    const std::string sentence = current.render().empty()
                                     ? "まだ文節を受け取っていません"
                                     : current.render();
    draw_wrapped(sentence, 65, 182, panel_width - 50, 2, kText);

    canvas_.fillRoundRect(40, 290, panel_width, 180, 20, kPanel);
    canvas_.setFont(&fonts::lgfxJapanGothic_20);
    canvas_.setTextColor(kAccent, kPanel);
    const std::string exchange_title =
        exchange_count_ == 0
            ? "最後の交換"
            : "最後の交換  #" + std::to_string(exchange_count_) + " / " +
                  latest_exchange_role_;
    canvas_.drawString(exchange_title.c_str(), 65, 310);
    canvas_.setTextColor(kMuted, kPanel);
    const std::string peer = latest_peer_.empty() ? "まだ交換していません"
                                                  : "peer  " + latest_peer_;
    canvas_.drawString(peer.c_str(), 390, 310);
    canvas_.setFont(&fonts::lgfxJapanGothic_24);
    const std::string received = exchange_count_ == 0
                                     ? "受取: —"
                                     : "受取: " + latest_received_;
    const std::string sent = exchange_count_ == 0 ? "配布: —"
                                                   : "配布: " + latest_sent_;
    draw_wrapped(received, 65, 354, panel_width - 50, 1, kText);
    draw_wrapped(sent, 65, 398, panel_width - 50, 1, kMuted);

    canvas_.setFont(&fonts::lgfxJapanGothic_20);
    canvas_.setTextColor(kPrimary, kBackground);
    canvas_.drawString(status_.c_str(), 45, 492);
    canvas_.setTextColor(kMuted, kBackground);
    int log_y = 530;
    for (auto it = logs_.rbegin(); it != logs_.rend(); ++it) {
      draw_wrapped(*it, 45, log_y, width - 90, 1, kMuted);
      log_y += 28;
    }

    const int button_y = height - 88;
    const Button run{32, button_y, width * 28 / 100, 64};
    const Button role{width * 33 / 100, button_y, width * 28 / 100, 64};
    const Button minus{width * 64 / 100, button_y, width * 10 / 100, 64};
    const Button plus{width * 87 / 100, button_y, width * 10 / 100, 64};
    draw_button(run, running_ ? 0x344b5eU : 0x267b67U,
                running_ ? "停止" : "再開");
    draw_button(role, 0x344b5eU,
                (std::string("役割: ") + role_name(role_mode_)).c_str());
    draw_button(minus, 0x344b5eU, "−");
    draw_button(plus, 0x344b5eU, "+");
    canvas_.setTextDatum(middle_center);
    canvas_.setTextColor(kText, kBackground);
    canvas_.drawString((std::to_string(rssi_threshold_) + " dBm").c_str(),
                       width * 805 / 1000, button_y + 32);
    canvas_.setTextDatum(top_left);
    canvas_.endWrite();

    // 表示中の画面を消さず、PSRAM上で完成した1フレームをまとめて転送する。
    M5.Display.waitDisplay();
    canvas_.pushSprite(&M5.Display, 0, 0);
  }

  RxCallbacks rx_callbacks_;
  TxCallbacks tx_callbacks_;
  ServerCallbacks server_callbacks_;
  ScanCallbacks scan_callbacks_;
  M5Canvas canvas_{&M5.Display};
  std::unique_ptr<ExchangeState> state_;
  BLEServer* server_ = nullptr;
  BLECharacteristic* info_ = nullptr;
  BLECharacteristic* rx_ = nullptr;
  BLECharacteristic* tx_ = nullptr;
  BLEAdvertising* advertising_ = nullptr;
  BLEScan* scan_ = nullptr;
  BLEClient* client_ = nullptr;
  std::mutex found_mutex_;
  std::unique_ptr<BLEAdvertisedDevice> found_device_;
  std::vector<KnownPeer> known_peers_;
  Role role_mode_ = Role::Auto;
  Role active_role_ = Role::Auto;
  int rssi_threshold_ = config::RSSI_THRESHOLD;
  bool running_ = true;
  bool role_change_requested_ = false;
  bool advertising_active_ = false;
  bool scanning_active_ = false;
  bool client_connected_ = false;
  bool dirty_ = true;
  std::uint32_t slot_deadline_ = 0;
  std::uint32_t exchange_count_ = 0;
  std::string status_ = "起動しています…";
  std::string latest_peer_;
  std::string latest_sent_;
  std::string latest_received_;
  std::string latest_exchange_role_;
  std::vector<std::string> logs_;
  std::string fatal_error_;
};

void RxCallbacks::onWrite(BLECharacteristic* characteristic,
                          ble_gap_conn_desc* connection) {
  app_.on_server_write(connection->conn_handle, characteristic);
}

void TxCallbacks::onRead(BLECharacteristic* characteristic,
                         ble_gap_conn_desc* connection) {
  app_.on_server_read(connection->conn_handle, characteristic);
}

void ServerCallbacks::onConnect(BLEServer*, ble_gap_conn_desc* connection) {
  app_.on_server_connect(connection->conn_handle);
}

void ServerCallbacks::onDisconnect(BLEServer*, ble_gap_conn_desc* connection) {
  app_.on_server_disconnect(connection->conn_handle);
}

void ScanCallbacks::onResult(BLEAdvertisedDevice device) {
  app_.on_advertised_device(device);
}

std::unique_ptr<App> app;

void draw_fatal(const std::string& error) {
  M5.Display.fillScreen(kBackground);
  M5.Display.setFont(&fonts::lgfxJapanGothic_28);
  M5.Display.setTextColor(kDanger, kBackground);
  M5.Display.drawString("起動できません", 40, 40);
  M5.Display.setFont(&fonts::lgfxJapanGothic_20);
  M5.Display.setTextColor(kText, kBackground);
  M5.Display.drawString(error.c_str(), 40, 100);
  M5.Display.setTextColor(kMuted, kBackground);
  M5.Display.drawString("USBシリアルログとREADMEを確認してください。", 40, 150);
}

}  // namespace
}  // namespace surechigai

void setup() {
  Serial.begin(115200);
  auto m5_config = M5.config();
  m5_config.clear_display = true;
  M5.begin(m5_config);
  if (M5.Display.width() < M5.Display.height()) M5.Display.setRotation(1);
  M5.Display.setBrightness(160);

  surechigai::app = std::make_unique<surechigai::App>();
  if (!surechigai::app->begin()) {
    Serial.printf("起動失敗: %s\n", surechigai::app->fatal_error().c_str());
    surechigai::draw_fatal(surechigai::app->fatal_error());
  }
}

void loop() {
  if (surechigai::app && surechigai::app->fatal_error().empty()) {
    surechigai::app->loop_once();
  } else {
    M5.update();
    delay(100);
  }
}
