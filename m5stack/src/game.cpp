#include "game.hpp"

#include "protocol.hpp"

#include <utility>

namespace surechigai::game {

std::size_t slot_index(Slot slot) { return static_cast<std::size_t>(slot); }

std::uint8_t slot_bit(Slot slot) {
  return static_cast<std::uint8_t>(1U << slot_index(slot));
}

const char* slot_label(Slot slot) {
  switch (slot) {
    case Slot::When:
      return "いつ";
    case Slot::Where:
      return "どこで";
    case Slot::Who:
      return "だれが";
    case Slot::What:
      return "なにをする";
    case Slot::Why:
      return "なぜ";
    case Slot::How:
      return "どのように";
  }
  return "?";
}

bool parse_slot(std::uint8_t value, Slot& slot) {
  if (value >= SLOT_COUNT) return false;
  slot = static_cast<Slot>(value);
  return true;
}

bool validate_phrase(const Phrase& phrase, std::string& error) {
  if (slot_index(phrase.slot) >= SLOT_COUNT) {
    error = "unknown 5W1H slot";
    return false;
  }
  if (phrase.text.empty() || phrase.text.size() > MAX_PHRASE_BYTES) {
    error = "invalid phrase length";
    return false;
  }
  if (!protocol::valid_utf8(phrase.text)) {
    error = "phrase is not valid UTF-8";
    return false;
  }
  error.clear();
  return true;
}

bool Deck::validate(std::string& error) const {
  for (Slot slot : ALL_SLOTS) {
    if (!validate_phrase(phrase(slot), error)) return false;
  }
  error.clear();
  return true;
}

Phrase Deck::phrase(Slot slot) const {
  return {slot, phrases_[slot_index(slot)]};
}

std::optional<Phrase> Deck::choose_for(std::uint8_t missing,
                                       std::uint32_t random) const {
  std::array<Slot, SLOT_COUNT> candidates{};
  std::size_t count = 0;
  for (Slot slot : ALL_SLOTS) {
    if ((missing & slot_bit(slot)) != 0) candidates[count++] = slot;
  }
  if (count == 0) return std::nullopt;
  return phrase(candidates[random % count]);
}

std::uint8_t Sentence::missing_mask() const {
  std::uint8_t missing = 0;
  for (Slot slot : ALL_SLOTS) {
    if (!entries_[slot_index(slot)]) missing |= slot_bit(slot);
  }
  return missing;
}

bool Sentence::accept(const Uuid& source, std::string source_name,
                      Phrase phrase) {
  if (slot_index(phrase.slot) >= SLOT_COUNT) return false;
  auto& target = entries_[slot_index(phrase.slot)];
  if (target) return false;
  target = SentenceEntry{source, std::move(source_name), std::move(phrase.text)};
  return true;
}

const SentenceEntry* Sentence::entry(Slot slot) const {
  const auto& value = entries_[slot_index(slot)];
  return value ? &*value : nullptr;
}

std::string Sentence::render() const {
  constexpr std::array<Slot, SLOT_COUNT> order = {
      Slot::When, Slot::How, Slot::Who, Slot::Where, Slot::Why, Slot::What};
  std::string result;
  for (Slot slot : order) {
    const SentenceEntry* value = entry(slot);
    if (value == nullptr) continue;
    if (!result.empty()) result += " ";
    result += value->text;
  }
  return result;
}

}  // namespace surechigai::game
