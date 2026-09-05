#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <utility>

namespace surechigai::game {

inline constexpr std::size_t MAX_PHRASE_BYTES = 64;
inline constexpr std::size_t MAX_NAME_BYTES = 32;
inline constexpr std::size_t SLOT_COUNT = 6;
inline constexpr std::uint8_t ALL_MISSING = (1U << SLOT_COUNT) - 1U;

using Uuid = std::array<std::uint8_t, 16>;

enum class Slot : std::uint8_t {
  When = 0,
  Where = 1,
  Who = 2,
  What = 3,
  Why = 4,
  How = 5,
};

inline constexpr std::array<Slot, SLOT_COUNT> ALL_SLOTS = {
    Slot::When, Slot::Where, Slot::Who, Slot::What, Slot::Why, Slot::How};

std::size_t slot_index(Slot slot);
std::uint8_t slot_bit(Slot slot);
const char* slot_label(Slot slot);
bool parse_slot(std::uint8_t value, Slot& slot);

struct Phrase {
  Slot slot = Slot::When;
  std::string text;

  bool operator==(const Phrase& other) const {
    return slot == other.slot && text == other.text;
  }
};

bool validate_phrase(const Phrase& phrase, std::string& error);

class Deck {
 public:
  Deck() = default;
  explicit Deck(std::array<std::string, SLOT_COUNT> phrases)
      : phrases_(std::move(phrases)) {}

  bool validate(std::string& error) const;
  Phrase phrase(Slot slot) const;
  std::optional<Phrase> choose_for(std::uint8_t missing,
                                   std::uint32_t random) const;

 private:
  std::array<std::string, SLOT_COUNT> phrases_{};
};

struct SentenceEntry {
  Uuid source{};
  std::string source_name;
  std::string text;
};

class Sentence {
 public:
  explicit Sentence(Uuid round = {}) : round_(round) {}

  const Uuid& round() const { return round_; }
  std::uint8_t missing_mask() const;
  bool accept(const Uuid& source, std::string source_name, Phrase phrase);
  const SentenceEntry* entry(Slot slot) const;
  bool is_complete() const { return missing_mask() == 0; }
  std::string render() const;

 private:
  Uuid round_{};
  std::array<std::optional<SentenceEntry>, SLOT_COUNT> entries_{};
};

}  // namespace surechigai::game
