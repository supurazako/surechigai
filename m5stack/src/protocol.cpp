#include "protocol.hpp"

#include <algorithm>
#include <cstdio>

namespace surechigai::protocol {
namespace {

void fail(std::string& error, const char* message) { error = message; }

std::uint32_t read_u32_le(const std::uint8_t* data) {
  return static_cast<std::uint32_t>(data[0]) |
         (static_cast<std::uint32_t>(data[1]) << 8U) |
         (static_cast<std::uint32_t>(data[2]) << 16U) |
         (static_cast<std::uint32_t>(data[3]) << 24U);
}

bool encode(const Packet& packet, Frame& bytes, std::string& error) {
  bytes.clear();
  if (const auto* profile = std::get_if<Profile>(&packet)) {
    if ((profile->missing & ~game::ALL_MISSING) != 0) {
      fail(error, "invalid missing mask");
      return false;
    }
    if (profile->name.empty() || profile->name.size() > game::MAX_NAME_BYTES ||
        !valid_utf8(profile->name)) {
      fail(error, "invalid user name");
      return false;
    }
    bytes.push_back(PROFILE);
    bytes.insert(bytes.end(), profile->node.begin(), profile->node.end());
    bytes.insert(bytes.end(), profile->round.begin(), profile->round.end());
    bytes.push_back(profile->missing);
    bytes.push_back(static_cast<std::uint8_t>(profile->name.size()));
    bytes.insert(bytes.end(), profile->name.begin(), profile->name.end());
  } else {
    const auto& gift = std::get<GiftPacket>(packet);
    bytes.push_back(GIFT);
    bytes.insert(bytes.end(), gift.receiver_round.begin(), gift.receiver_round.end());
    if (gift.gift) {
      if (!game::validate_phrase(*gift.gift, error)) return false;
      bytes.push_back(static_cast<std::uint8_t>(gift.gift->slot));
      bytes.push_back(static_cast<std::uint8_t>(gift.gift->text.size()));
      bytes.insert(bytes.end(), gift.gift->text.begin(), gift.gift->text.end());
    } else {
      bytes.push_back(NO_GIFT);
      bytes.push_back(0);
    }
  }
  if (bytes.size() > MAX_ENVELOPE_BYTES) {
    fail(error, "packet too large");
    return false;
  }
  error.clear();
  return true;
}

bool decode(const Frame& bytes, Packet& packet, std::string& error) {
  if (bytes.empty()) {
    fail(error, "empty packet");
    return false;
  }
  if (bytes[0] == PROFILE) {
    if (bytes.size() < 35) {
      fail(error, "invalid profile length");
      return false;
    }
    const std::uint8_t missing = bytes[33];
    const std::size_t name_size = bytes[34];
    if ((missing & ~game::ALL_MISSING) != 0 || name_size == 0 ||
        name_size > game::MAX_NAME_BYTES || bytes.size() != 35 + name_size) {
      fail(error, "invalid profile fields");
      return false;
    }
    Profile profile;
    std::copy(bytes.begin() + 1, bytes.begin() + 17, profile.node.begin());
    std::copy(bytes.begin() + 17, bytes.begin() + 33, profile.round.begin());
    profile.missing = missing;
    profile.name.assign(reinterpret_cast<const char*>(bytes.data() + 35),
                        name_size);
    if (!valid_utf8(profile.name)) {
      fail(error, "invalid user name");
      return false;
    }
    packet = std::move(profile);
    error.clear();
    return true;
  }
  if (bytes[0] == GIFT) {
    if (bytes.size() < 19) {
      fail(error, "truncated gift");
      return false;
    }
    const std::uint8_t slot_value = bytes[17];
    const std::size_t text_size = bytes[18];
    if (text_size > game::MAX_PHRASE_BYTES || bytes.size() != 19 + text_size) {
      fail(error, "gift length mismatch");
      return false;
    }
    GiftPacket gift;
    std::copy(bytes.begin() + 1, bytes.begin() + 17,
              gift.receiver_round.begin());
    if (slot_value == NO_GIFT) {
      if (text_size != 0) {
        fail(error, "invalid empty gift");
        return false;
      }
    } else {
      game::Slot slot;
      if (!game::parse_slot(slot_value, slot)) {
        fail(error, "unknown 5W1H slot");
        return false;
      }
      game::Phrase phrase{
          slot, std::string(reinterpret_cast<const char*>(bytes.data() + 19),
                            text_size)};
      if (!game::validate_phrase(phrase, error)) return false;
      gift.gift = std::move(phrase);
    }
    packet = std::move(gift);
    error.clear();
    return true;
  }
  fail(error, "unknown packet type");
  return false;
}

}  // namespace

bool valid_utf8(const std::string& text) {
  const auto* bytes = reinterpret_cast<const std::uint8_t*>(text.data());
  std::size_t i = 0;
  while (i < text.size()) {
    const std::uint8_t first = bytes[i];
    if (first <= 0x7f) {
      ++i;
      continue;
    }
    std::size_t continuation = 0;
    std::uint32_t codepoint = 0;
    if ((first & 0xe0U) == 0xc0U) {
      continuation = 1;
      codepoint = first & 0x1fU;
    } else if ((first & 0xf0U) == 0xe0U) {
      continuation = 2;
      codepoint = first & 0x0fU;
    } else if ((first & 0xf8U) == 0xf0U) {
      continuation = 3;
      codepoint = first & 0x07U;
    } else {
      return false;
    }
    if (i + continuation >= text.size()) return false;
    for (std::size_t j = 1; j <= continuation; ++j) {
      const std::uint8_t next = bytes[i + j];
      if ((next & 0xc0U) != 0x80U) return false;
      codepoint = (codepoint << 6U) | (next & 0x3fU);
    }
    if ((continuation == 1 && codepoint < 0x80U) ||
        (continuation == 2 && codepoint < 0x800U) ||
        (continuation == 3 && codepoint < 0x10000U) ||
        (codepoint >= 0xd800U && codepoint <= 0xdfffU) ||
        codepoint > 0x10ffffU) {
      return false;
    }
    i += continuation + 1;
  }
  return true;
}

Frame command(std::uint8_t kind, std::uint32_t exchange) {
  return {VERSION,
          kind,
          static_cast<std::uint8_t>(exchange),
          static_cast<std::uint8_t>(exchange >> 8U),
          static_cast<std::uint8_t>(exchange >> 16U),
          static_cast<std::uint8_t>(exchange >> 24U)};
}

Frame identity(const NodeId& node) {
  Frame value{VERSION};
  value.insert(value.end(), node.begin(), node.end());
  return value;
}

bool parse_identity(const std::uint8_t* data, std::size_t size, NodeId& node,
                    std::string& error) {
  if (data == nullptr || size != 17 || data[0] != VERSION) {
    fail(error, "invalid peer identity/version");
    return false;
  }
  std::copy(data + 1, data + 17, node.begin());
  error.clear();
  return true;
}

bool parse_header(const std::uint8_t* data, std::size_t size, Header& header,
                  std::string& error) {
  if (data == nullptr || size < 6 || size > MAX_FRAME_BYTES) {
    fail(error, "invalid frame length");
    return false;
  }
  if (data[0] != VERSION) {
    fail(error, "unsupported protocol version");
    return false;
  }
  header.kind = data[1];
  header.exchange = read_u32_le(data + 2);
  error.clear();
  return true;
}

bool make_frames(const Packet& packet, std::uint32_t exchange,
                 std::vector<Frame>& frames, std::string& error) {
  Frame envelope;
  if (!encode(packet, envelope, error)) return false;
  const auto count = static_cast<std::uint8_t>(
      (envelope.size() + CHUNK_BYTES - 1) / CHUNK_BYTES);
  frames.clear();
  frames.reserve(count);
  for (std::uint8_t index = 0; index < count; ++index) {
    const std::size_t begin = static_cast<std::size_t>(index) * CHUNK_BYTES;
    const std::size_t end = std::min(begin + CHUNK_BYTES, envelope.size());
    Frame frame = command(DATA, exchange);
    frame.push_back(index);
    frame.push_back(count);
    frame.insert(frame.end(), envelope.begin() + begin, envelope.begin() + end);
    frames.push_back(std::move(frame));
  }
  error.clear();
  return true;
}

bool require_exchange(const Frame& frame, std::uint32_t expected,
                      std::string& error) {
  Header header;
  if (!parse_header(frame.data(), frame.size(), header, error)) return false;
  if (header.exchange != expected) {
    fail(error, "reply belongs to another exchange");
    return false;
  }
  error.clear();
  return true;
}

PushResult Assembler::push(const std::uint8_t* data, std::size_t size,
                           Packet& packet, std::string& error) {
  Header header;
  if (!parse_header(data, size, header, error)) return PushResult::Error;
  if (header.kind != DATA || size < 9) {
    fail(error, "expected data frame");
    return PushResult::Error;
  }
  const std::uint8_t sequence = data[6];
  const std::uint8_t count = data[7];
  if (count < 1 || count > MAX_FRAMES || sequence >= count) {
    fail(error, "invalid fragment count/index");
    return PushResult::Error;
  }
  if (sequence != next_) {
    fail(error, "out-of-order or duplicate fragment");
    return PushResult::Error;
  }
  if (sequence + 1 != count && size != MAX_FRAME_BYTES) {
    fail(error, "short non-final fragment");
    return PushResult::Error;
  }
  if (has_exchange_ && (exchange_ != header.exchange || count_ != count)) {
    fail(error, "fragment belongs to another exchange");
    return PushResult::Error;
  }
  if (bytes_.size() + size - 8 > MAX_ENVELOPE_BYTES) {
    fail(error, "packet too large");
    return PushResult::Error;
  }
  has_exchange_ = true;
  exchange_ = header.exchange;
  count_ = count;
  bytes_.insert(bytes_.end(), data + 8, data + size);
  ++next_;
  if (next_ != count_) {
    error.clear();
    return PushResult::Incomplete;
  }
  return decode(bytes_, packet, error) ? PushResult::Complete
                                       : PushResult::Error;
}

void Assembler::reset() {
  has_exchange_ = false;
  exchange_ = 0;
  count_ = 0;
  next_ = 0;
  bytes_.clear();
}

std::string node_to_string(const NodeId& node) {
  char output[37];
  std::snprintf(output, sizeof(output),
                "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-"
                "%02x%02x%02x%02x%02x%02x",
                node[0], node[1], node[2], node[3], node[4], node[5], node[6],
                node[7], node[8], node[9], node[10], node[11], node[12],
                node[13], node[14], node[15]);
  return output;
}

}  // namespace surechigai::protocol
