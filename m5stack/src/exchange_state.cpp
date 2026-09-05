#include "exchange_state.hpp"

#include <algorithm>
#include <utility>

namespace surechigai {
namespace {

void fail(std::string& error, const char* message) { error = message; }

bool all_read(const std::vector<bool>& values) {
  return std::all_of(values.begin(), values.end(), [](bool value) {
    return value;
  });
}

}  // namespace

ExchangeState::ExchangeState(protocol::NodeId node, std::string name,
                             game::Deck deck, game::Uuid round,
                             std::uint32_t timeout_ms,
                             std::uint32_t cooldown_ms)
    : node_(node),
      name_(std::move(name)),
      deck_(std::move(deck)),
      sentence_(round),
      timeout_ms_(timeout_ms),
      cooldown_ms_(cooldown_ms) {}

bool ExchangeState::reached(std::uint32_t now, std::uint32_t deadline) {
  return static_cast<std::int32_t>(now - deadline) >= 0;
}

protocol::Profile ExchangeState::profile_locked() const {
  return {node_, name_, sentence_.round(), sentence_.missing_mask()};
}

protocol::Profile ExchangeState::profile() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return profile_locked();
}

game::Sentence ExchangeState::sentence() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return sentence_;
}

protocol::GiftPacket ExchangeState::choose_gift_locked(
    const protocol::Profile& peer, std::uint32_t random) const {
  return {peer.round, deck_.choose_for(peer.missing, random)};
}

protocol::GiftPacket ExchangeState::choose_gift(
    const protocol::Profile& peer, std::uint32_t random) const {
  std::lock_guard<std::mutex> lock(mutex_);
  return choose_gift_locked(peer, random);
}

void ExchangeState::enable() {
  std::lock_guard<std::mutex> lock(mutex_);
  enabled_ = true;
}

void ExchangeState::shutdown() {
  std::lock_guard<std::mutex> lock(mutex_);
  enabled_ = false;
  session_.reset();
  connections_.clear();
}

void ExchangeState::expire_locked(std::uint32_t now) {
  connections_.erase(
      std::remove_if(connections_.begin(), connections_.end(),
                     [now](const ConnectionLease& connection) {
                       return reached(now, connection.deadline);
                     }),
      connections_.end());
  recent_.erase(std::remove_if(recent_.begin(), recent_.end(),
                               [now](const Recent& item) {
                                 return reached(now, item.until);
                               }),
                recent_.end());
  if (session_ && reached(now, session_->deadline)) session_.reset();
}

void ExchangeState::expire(std::uint32_t now) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
}

bool ExchangeState::disable_if_idle(std::uint32_t now) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  if (session_ || !connections_.empty()) return false;
  enabled_ = false;
  return true;
}

void ExchangeState::connected(std::uint16_t client, std::uint32_t now) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  if (!enabled_) return;
  auto connection = std::find_if(
      connections_.begin(), connections_.end(),
      [client](const ConnectionLease& item) { return item.client == client; });
  if (connection == connections_.end()) {
    connections_.push_back({client, now + timeout_ms_});
  } else {
    connection->deadline = now + timeout_ms_;
  }
}

void ExchangeState::disconnected(std::uint16_t client) {
  std::lock_guard<std::mutex> lock(mutex_);
  connections_.erase(
      std::remove_if(connections_.begin(), connections_.end(),
                     [client](const ConnectionLease& connection) {
                       return connection.client == client;
                     }),
      connections_.end());
  if (session_ && session_->client == client) session_.reset();
}

bool ExchangeState::cooling_down_locked(const protocol::NodeId& node,
                                        std::uint32_t now) const {
  return std::any_of(recent_.begin(), recent_.end(),
                     [&node, now](const Recent& item) {
                       return item.node == node && !reached(now, item.until);
                     });
}

bool ExchangeState::cooling_down(const protocol::NodeId& node,
                                 std::uint32_t now) {
  std::lock_guard<std::mutex> lock(mutex_);
  return cooling_down_locked(node, now);
}

bool ExchangeState::record_locked(const protocol::Profile& peer,
                                  const protocol::GiftPacket& sent,
                                  const protocol::GiftPacket& received,
                                  std::uint32_t now, bool as_central,
                                  std::string& error) {
  if (peer.node == node_) {
    fail(error, "self exchange");
    return false;
  }
  if (sent.receiver_round != peer.round) {
    fail(error, "gift targets another round");
    return false;
  }
  if (received.receiver_round != sentence_.round()) {
    fail(error, "received gift targets another round");
    return false;
  }
  if (sent.gift && (peer.missing & game::slot_bit(sent.gift->slot)) == 0) {
    fail(error, "gift is not requested by peer");
    return false;
  }
  if (received.gift &&
      !sentence_.accept(peer.node, peer.name, *received.gift)) {
    fail(error, "received slot is already filled");
    return false;
  }
  recent_.erase(std::remove_if(recent_.begin(), recent_.end(),
                               [&peer](const Recent& item) {
                                 return item.node == peer.node;
                               }),
                recent_.end());
  if (cooldown_ms_ != 0) recent_.push_back({peer.node, now + cooldown_ms_});
  events_.push_back({peer, sent, received, sentence_, as_central});
  while (events_.size() > 8) events_.pop_front();
  error.clear();
  return true;
}

bool ExchangeState::record_central(const protocol::Profile& peer,
                                   const protocol::GiftPacket& sent,
                                   const protocol::GiftPacket& received,
                                   std::uint32_t now, std::string& error) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  return record_locked(peer, sent, received, now, true, error);
}

std::optional<ExchangeEvent> ExchangeState::take_event() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (events_.empty()) return std::nullopt;
  ExchangeEvent event = std::move(events_.front());
  events_.pop_front();
  return event;
}

bool ExchangeState::handle_packet_locked(protocol::Packet packet,
                                         std::uint32_t now,
                                         std::string& error) {
  if (!session_->peer) {
    const auto* peer = std::get_if<protocol::Profile>(&packet);
    if (peer == nullptr) {
      fail(error, "profile must be sent first");
      return false;
    }
    if (peer->node == node_) {
      fail(error, "self exchange");
      return false;
    }
    if (cooling_down_locked(peer->node, now)) {
      fail(error, "peer is cooling down");
      return false;
    }
    const protocol::GiftPacket outgoing =
        choose_gift_locked(*peer, now ^ session_->exchange);
    std::vector<protocol::Frame> reply;
    if (!protocol::make_frames(profile_locked(), session_->exchange, reply,
                               error)) {
      return false;
    }
    session_->peer = *peer;
    session_->outgoing_gift = outgoing;
    session_->reply = std::move(reply);
    session_->read.assign(session_->reply.size(), false);
    session_->selected.reset();
    session_->incoming.reset();
    error.clear();
    return true;
  }

  const auto* gift = std::get_if<protocol::GiftPacket>(&packet);
  if (gift == nullptr) {
    fail(error, "expected gift packet");
    return false;
  }
  if (gift->receiver_round != sentence_.round()) {
    fail(error, "gift targets another round");
    return false;
  }
  if (gift->gift && sentence_.entry(gift->gift->slot) != nullptr) {
    fail(error, "gift slot is already filled");
    return false;
  }
  std::vector<protocol::Frame> reply;
  if (!protocol::make_frames(*session_->outgoing_gift, session_->exchange,
                             reply, error)) {
    return false;
  }
  session_->incoming_gift = *gift;
  session_->reply = std::move(reply);
  session_->read.assign(session_->reply.size(), false);
  session_->selected.reset();
  session_->incoming.reset();
  error.clear();
  return true;
}

bool ExchangeState::write(std::uint16_t client, const std::uint8_t* data,
                          std::size_t size, std::uint32_t now,
                          std::string& error) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  if (!enabled_) {
    fail(error, "not in peripheral role");
    return false;
  }
  protocol::Header header;
  if (!protocol::parse_header(data, size, header, error)) return false;
  if (!session_) {
    if (header.kind != protocol::DATA || size < 9 || data[6] != 0) {
      fail(error, "start with data fragment zero");
      return false;
    }
    Session session;
    session.client = client;
    session.exchange = header.exchange;
    session.deadline = now + timeout_ms_;
    protocol::Packet packet;
    const auto result = session.incoming.push(data, size, packet, error);
    if (result == protocol::PushResult::Error) return false;
    connections_.erase(
        std::remove_if(connections_.begin(), connections_.end(),
                       [client](const ConnectionLease& connection) {
                         return connection.client == client;
                       }),
        connections_.end());
    session_ = std::move(session);
    if (result == protocol::PushResult::Complete) {
      return handle_packet_locked(std::move(packet), now, error);
    }
    error.clear();
    return true;
  }

  Session& session = *session_;
  if (session.client != client || session.exchange != header.exchange) {
    fail(error, "busy with another exchange");
    return false;
  }
  switch (header.kind) {
    case protocol::DATA: {
      const bool receiving_gift = session.peer.has_value();
      if (receiving_gift && !all_read(session.read)) {
        fail(error, "profile reply has not been read");
        return false;
      }
      if (receiving_gift && session.incoming_gift) {
        fail(error, "gift already received");
        return false;
      }
      protocol::Packet packet;
      const auto result = session.incoming.push(data, size, packet, error);
      if (result == protocol::PushResult::Error) return false;
      if (result == protocol::PushResult::Complete) {
        return handle_packet_locked(std::move(packet), now, error);
      }
      break;
    }
    case protocol::SELECT: {
      if (size != 7 || session.committed) {
        fail(error, "invalid select");
        return false;
      }
      const std::size_t index = data[6];
      if (index >= session.reply.size()) {
        fail(error, "reply unavailable/index out of range");
        return false;
      }
      session.selected = index;
      break;
    }
    case protocol::ACK: {
      if (size != 6 || !session.incoming_gift || !all_read(session.read)) {
        fail(error, "premature acknowledgement");
        return false;
      }
      if (!session.committed) {
        if (!record_locked(*session.peer, *session.outgoing_gift,
                           *session.incoming_gift, now, false, error)) {
          return false;
        }
        session.committed = true;
        session.deadline = now + 1000;
      }
      break;
    }
    default:
      fail(error, "unknown command");
      return false;
  }
  error.clear();
  return true;
}

bool ExchangeState::read(std::uint16_t client, std::uint32_t now,
                         protocol::Frame& value, std::string& error) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  if (!enabled_ || !session_ || session_->client != client ||
      !session_->selected) {
    fail(error, "no selected reply for this client");
    return false;
  }
  const std::size_t selected = *session_->selected;
  session_->read[selected] = true;
  value = session_->reply[selected];
  error.clear();
  return true;
}

bool ExchangeState::selected_value(std::uint16_t client, std::uint32_t now,
                                   protocol::Frame& value,
                                   std::string& error) {
  std::lock_guard<std::mutex> lock(mutex_);
  expire_locked(now);
  if (!enabled_ || !session_ || session_->client != client ||
      !session_->selected) {
    fail(error, "no selected reply for this client");
    return false;
  }
  value = session_->reply[*session_->selected];
  error.clear();
  return true;
}

std::string gift_label(const protocol::GiftPacket& packet) {
  if (!packet.gift) return "なし";
  return std::string(game::slot_label(packet.gift->slot)) + ":\"" +
         packet.gift->text + "\"";
}

}  // namespace surechigai
