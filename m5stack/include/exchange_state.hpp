#pragma once

#include "game.hpp"
#include "protocol.hpp"

#include <cstddef>
#include <cstdint>
#include <deque>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace surechigai {

struct ExchangeEvent {
  protocol::Profile peer;
  protocol::GiftPacket sent;
  protocol::GiftPacket received;
  game::Sentence sentence;
  bool as_central = false;
};

class ExchangeState {
 public:
  ExchangeState(protocol::NodeId node, std::string name, game::Deck deck,
                game::Uuid round, std::uint32_t timeout_ms,
                std::uint32_t cooldown_ms);

  const protocol::NodeId& node() const { return node_; }
  const std::string& name() const { return name_; }
  protocol::Profile profile() const;
  game::Sentence sentence() const;
  protocol::GiftPacket choose_gift(const protocol::Profile& peer,
                                   std::uint32_t random) const;

  void enable();
  void shutdown();
  void expire(std::uint32_t now);
  bool disable_if_idle(std::uint32_t now);
  void connected(std::uint16_t client, std::uint32_t now);
  void disconnected(std::uint16_t client);

  bool cooling_down(const protocol::NodeId& node, std::uint32_t now);
  bool record_central(const protocol::Profile& peer,
                      const protocol::GiftPacket& sent,
                      const protocol::GiftPacket& received, std::uint32_t now,
                      std::string& error);
  std::optional<ExchangeEvent> take_event();

  bool write(std::uint16_t client, const std::uint8_t* data, std::size_t size,
             std::uint32_t now, std::string& error);
  bool selected_value(std::uint16_t client, std::uint32_t now,
                      protocol::Frame& value, std::string& error);
  bool read(std::uint16_t client, std::uint32_t now, protocol::Frame& value,
            std::string& error);

 private:
  struct Recent {
    protocol::NodeId node{};
    std::uint32_t until = 0;
  };

  struct Session {
    std::uint16_t client = 0;
    std::uint32_t exchange = 0;
    std::uint32_t deadline = 0;
    protocol::Assembler incoming;
    std::optional<protocol::Profile> peer;
    std::optional<protocol::GiftPacket> outgoing_gift;
    std::optional<protocol::GiftPacket> incoming_gift;
    std::vector<protocol::Frame> reply;
    std::optional<std::size_t> selected;
    std::vector<bool> read;
    bool committed = false;
  };

  struct ConnectionLease {
    std::uint16_t client = 0;
    std::uint32_t deadline = 0;
  };

  static bool reached(std::uint32_t now, std::uint32_t deadline);
  void expire_locked(std::uint32_t now);
  bool cooling_down_locked(const protocol::NodeId& node,
                           std::uint32_t now) const;
  protocol::Profile profile_locked() const;
  protocol::GiftPacket choose_gift_locked(const protocol::Profile& peer,
                                          std::uint32_t random) const;
  bool handle_packet_locked(protocol::Packet packet, std::uint32_t now,
                            std::string& error);
  bool record_locked(const protocol::Profile& peer,
                     const protocol::GiftPacket& sent,
                     const protocol::GiftPacket& received, std::uint32_t now,
                     bool as_central, std::string& error);

  protocol::NodeId node_{};
  std::string name_;
  game::Deck deck_;
  game::Sentence sentence_;
  bool enabled_ = false;
  std::optional<Session> session_;
  std::vector<ConnectionLease> connections_;
  std::vector<Recent> recent_;
  std::deque<ExchangeEvent> events_;
  std::uint32_t timeout_ms_;
  std::uint32_t cooldown_ms_;
  mutable std::mutex mutex_;
};

std::string gift_label(const protocol::GiftPacket& packet);

}  // namespace surechigai
