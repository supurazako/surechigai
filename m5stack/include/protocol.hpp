#pragma once

#include "game.hpp"

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <variant>
#include <vector>

namespace surechigai::protocol {

inline constexpr char SERVICE_UUID[] = "478f5400-73ad-47a6-a131-562697033a90";
inline constexpr char INFO_UUID[] = "478f5401-73ad-47a6-a131-562697033a90";
inline constexpr char RX_UUID[] = "478f5402-73ad-47a6-a131-562697033a90";
inline constexpr char TX_UUID[] = "478f5403-73ad-47a6-a131-562697033a90";

inline constexpr std::uint8_t VERSION = 2;
inline constexpr std::uint8_t DATA = 1;
inline constexpr std::uint8_t SELECT = 2;
inline constexpr std::uint8_t ACK = 3;
inline constexpr std::uint8_t PROFILE = 1;
inline constexpr std::uint8_t GIFT = 2;
inline constexpr std::uint8_t NO_GIFT = 0xff;
inline constexpr std::size_t MAX_FRAME_BYTES = 20;
inline constexpr std::size_t CHUNK_BYTES = 12;
inline constexpr std::size_t MAX_ENVELOPE_BYTES =
    1 + 16 + 1 + 1 + game::MAX_PHRASE_BYTES;
inline constexpr std::size_t MAX_FRAMES =
    (MAX_ENVELOPE_BYTES + CHUNK_BYTES - 1) / CHUNK_BYTES;

using NodeId = game::Uuid;
using Frame = std::vector<std::uint8_t>;

struct Profile {
  NodeId node{};
  std::string name;
  game::Uuid round{};
  std::uint8_t missing = 0;

  bool operator==(const Profile& other) const {
    return node == other.node && name == other.name && round == other.round &&
           missing == other.missing;
  }
};

struct GiftPacket {
  game::Uuid receiver_round{};
  std::optional<game::Phrase> gift;

  bool operator==(const GiftPacket& other) const {
    return receiver_round == other.receiver_round && gift == other.gift;
  }
};

using Packet = std::variant<Profile, GiftPacket>;

struct Header {
  std::uint8_t kind = 0;
  std::uint32_t exchange = 0;
};

bool valid_utf8(const std::string& text);
Frame command(std::uint8_t kind, std::uint32_t exchange);
Frame identity(const NodeId& node);
bool parse_identity(const std::uint8_t* data, std::size_t size, NodeId& node,
                    std::string& error);
bool parse_header(const std::uint8_t* data, std::size_t size, Header& header,
                  std::string& error);
bool make_frames(const Packet& packet, std::uint32_t exchange,
                 std::vector<Frame>& frames, std::string& error);
bool require_exchange(const Frame& frame, std::uint32_t expected,
                      std::string& error);
std::string node_to_string(const NodeId& node);

enum class PushResult { Incomplete, Complete, Error };

class Assembler {
 public:
  PushResult push(const std::uint8_t* data, std::size_t size, Packet& packet,
                  std::string& error);
  PushResult push(const Frame& frame, Packet& packet, std::string& error) {
    return push(frame.data(), frame.size(), packet, error);
  }
  void reset();

 private:
  bool has_exchange_ = false;
  std::uint32_t exchange_ = 0;
  std::uint8_t count_ = 0;
  std::uint8_t next_ = 0;
  std::vector<std::uint8_t> bytes_;
};

}  // namespace surechigai::protocol
