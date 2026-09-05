#include "exchange_state.hpp"
#include "game.hpp"
#include "protocol.hpp"

#include <cassert>
#include <iostream>
#include <string>
#include <variant>
#include <vector>

using surechigai::ExchangeState;
using surechigai::game::ALL_MISSING;
using surechigai::game::Deck;
using surechigai::game::Phrase;
using surechigai::game::Slot;
using surechigai::game::Uuid;
using surechigai::protocol::ACK;
using surechigai::protocol::Assembler;
using surechigai::protocol::Frame;
using surechigai::protocol::GiftPacket;
using surechigai::protocol::Packet;
using surechigai::protocol::Profile;
using surechigai::protocol::PushResult;
using surechigai::protocol::SELECT;

namespace {

Uuid uuid(std::uint8_t seed) {
  Uuid result{};
  for (std::size_t i = 0; i < result.size(); ++i) {
    result[i] = static_cast<std::uint8_t>(seed + i);
  }
  return result;
}

Deck deck(const std::string& prefix) {
  return Deck({prefix + "-when", prefix + "-where", prefix + "-who",
               prefix + "-what", prefix + "-why", prefix + "-how"});
}

Profile profile(std::uint8_t seed, std::string name,
                std::uint8_t missing = ALL_MISSING) {
  return {uuid(seed), std::move(name), uuid(seed + 20), missing};
}

Packet roundtrip(const Packet& original, std::uint32_t exchange = 42) {
  std::vector<Frame> frames;
  std::string error;
  assert(surechigai::protocol::make_frames(original, exchange, frames, error));
  Assembler assembler;
  Packet decoded;
  PushResult result = PushResult::Incomplete;
  for (const Frame& frame : frames) {
    assert(frame.size() <= 20);
    result = assembler.push(frame, decoded, error);
  }
  assert(result == PushResult::Complete);
  return decoded;
}

void packets_roundtrip() {
  const Profile original = profile(1, "alice");
  assert(std::get<Profile>(roundtrip(original)) == original);

  GiftPacket gift{uuid(30), Phrase{Slot::How, "従順な🍺"}};
  assert(std::get<GiftPacket>(roundtrip(gift)) == gift);
  gift = {uuid(40), std::nullopt};
  assert(std::get<GiftPacket>(roundtrip(gift)) == gift);
  gift = {uuid(50), Phrase{Slot::What, std::string(64, 'a')}};
  assert(std::get<GiftPacket>(roundtrip(gift)) == gift);
}

void rejects_invalid_packets_and_frames() {
  std::string error;
  std::vector<Frame> frames;
  GiftPacket gift{uuid(1), Phrase{Slot::What, "hello"}};
  assert(surechigai::protocol::make_frames(gift, 7, frames, error));
  Assembler assembler;
  Packet decoded;
  assert(assembler.push(frames[1], decoded, error) == PushResult::Error);

  assembler.reset();
  assert(assembler.push(frames[0], decoded, error) == PushResult::Incomplete);
  assert(assembler.push(frames[0], decoded, error) == PushResult::Error);

  Frame bad = frames[0];
  bad[0] = 1;
  assembler.reset();
  assert(assembler.push(bad, decoded, error) == PushResult::Error);
  bad = frames[0];
  bad[7] = 255;
  assembler.reset();
  assert(assembler.push(bad, decoded, error) == PushResult::Error);

  gift.gift = Phrase{Slot::What, std::string(65, 'x')};
  assert(!surechigai::protocol::make_frames(gift, 1, frames, error));
  gift.gift = Phrase{Slot::What, std::string("\xc0\xaf", 2)};
  assert(!surechigai::protocol::make_frames(gift, 1, frames, error));
  Profile invalid = profile(1, "");
  assert(!surechigai::protocol::make_frames(invalid, 1, frames, error));
}

void sentence_and_deck_rules() {
  std::string error;
  Deck local = deck("local");
  assert(local.validate(error));
  const auto selected = local.choose_for(surechigai::game::slot_bit(Slot::Who), 5);
  assert(selected && selected->slot == Slot::Who && selected->text == "local-who");
  assert(!local.choose_for(0, 0));

  surechigai::game::Sentence sentence(uuid(90));
  assert(sentence.accept(uuid(1), "alice", {Slot::How, "従順な"}));
  assert(sentence.accept(uuid(1), "alice", {Slot::Who, "犬が"}));
  assert(sentence.accept(uuid(1), "alice", {Slot::Where, "パリに"}));
  assert(sentence.accept(uuid(1), "alice", {Slot::What, "行く"}));
  assert(!sentence.accept(uuid(2), "bob", {Slot::What, "踊る"}));
  assert(sentence.render() == "従順な 犬が パリに 行く");
  assert(sentence.missing_mask() ==
         (surechigai::game::slot_bit(Slot::When) |
          surechigai::game::slot_bit(Slot::Why)));
}

void write_packet(ExchangeState& state, std::uint16_t client,
                  std::uint32_t exchange, const Packet& packet,
                  std::uint32_t now) {
  std::vector<Frame> frames;
  std::string error;
  assert(surechigai::protocol::make_frames(packet, exchange, frames, error));
  for (const Frame& frame : frames) {
    assert(state.write(client, frame.data(), frame.size(), now, error));
  }
}

Packet read_reply(ExchangeState& state, std::uint16_t client,
                  std::uint32_t exchange, std::uint32_t now) {
  Assembler assembler;
  Packet decoded;
  for (std::size_t index = 0; index < surechigai::protocol::MAX_FRAMES;
       ++index) {
    Frame select = surechigai::protocol::command(SELECT, exchange);
    select.push_back(static_cast<std::uint8_t>(index));
    std::string error;
    assert(state.write(client, select.data(), select.size(), now, error));
    Frame prepared;
    assert(state.selected_value(client, now, prepared, error));
    Frame value;
    assert(state.read(client, now, value, error));
    assert(prepared == value);
    const auto result = assembler.push(value, decoded, error);
    assert(result != PushResult::Error);
    if (result == PushResult::Complete) return decoded;
  }
  assert(false);
  return decoded;
}

void complete_symmetric_exchange_and_cooldown() {
  ExchangeState state(uuid(10), "local-user", deck("local"), uuid(70), 10000,
                      30000);
  const Profile peer = profile(40, "peer-user");
  state.enable();
  const std::uint32_t exchange = 9;
  write_packet(state, 1, exchange, peer, 100);

  std::string error;
  Frame ack = surechigai::protocol::command(ACK, exchange);
  assert(!state.write(1, ack.data(), ack.size(), 100, error));
  const Profile reply = std::get<Profile>(read_reply(state, 1, exchange, 100));
  assert(reply.node == state.node());

  GiftPacket received{reply.round, Phrase{Slot::Who, "peer-who"}};
  write_packet(state, 1, exchange, received, 100);
  const GiftPacket sent =
      std::get<GiftPacket>(read_reply(state, 1, exchange, 100));
  assert(sent.receiver_round == peer.round && sent.gift);
  assert(state.write(1, ack.data(), ack.size(), 100, error));
  assert(state.write(1, ack.data(), ack.size(), 500, error));

  const auto sentence = state.sentence();
  assert(sentence.entry(Slot::Who));
  assert(sentence.entry(Slot::Who)->text == "peer-who");
  assert(sentence.entry(Slot::Who)->source == peer.node);
  assert(sentence.entry(Slot::Who)->source_name == "peer-user");
  assert(state.cooling_down(peer.node, 30099));
  assert(!state.cooling_down(peer.node, 30100));
  const auto event = state.take_event();
  assert(event && event->peer == peer && !event->as_central);
  assert(event->received == received && event->sent == sent);
  assert(state.disable_if_idle(1100));
}

void both_devices_give_and_build() {
  ExchangeState a(uuid(1), "alice", deck("a"), uuid(80), 10000, 30000);
  ExchangeState b(uuid(20), "bob", deck("b"), uuid(100), 10000, 30000);
  const Profile a_profile = a.profile();
  const Profile b_profile = b.profile();
  const GiftPacket a_to_b = a.choose_gift(b_profile, 2);
  const GiftPacket b_to_a = b.choose_gift(a_profile, 4);
  std::string error;
  assert(a.record_central(b_profile, a_to_b, b_to_a, 0, error));
  assert(b.record_central(a_profile, b_to_a, a_to_b, 0, error));
  assert(a.sentence().missing_mask() != ALL_MISSING);
  assert(b.sentence().missing_mask() != ALL_MISSING);
  assert(a.sentence().entry(b_to_a.gift->slot)->text.rfind("b-", 0) == 0);
  assert(b.sentence().entry(a_to_b.gift->slot)->text.rfind("a-", 0) == 0);
}

void timeout_and_connection_lease() {
  const Profile peer = profile(40, "peer");
  ExchangeState state(uuid(10), "local", deck("local"), uuid(70), 1000, 0);
  state.enable();
  std::vector<Frame> frames;
  std::string error;
  assert(surechigai::protocol::make_frames(peer, 1, frames, error));
  assert(state.write(1, frames[0].data(), frames[0].size(), 0, error));
  assert(!state.disable_if_idle(999));
  assert(state.disable_if_idle(1000));

  state.enable();
  state.connected(7, 1000);
  assert(!state.disable_if_idle(1999));
  state.disconnected(7);
  assert(state.disable_if_idle(1999));
}

}  // namespace

int main() {
  packets_roundtrip();
  rejects_invalid_packets_and_frames();
  sentence_and_deck_rules();
  complete_symmetric_exchange_and_cooldown();
  both_devices_give_and_build();
  timeout_and_connection_lease();
  std::cout << "all game/protocol/state tests passed\n";
}
