#pragma once

#include <cstddef>
#include <cstdint>

namespace surechigai::config {

enum class Role { Auto, Central, Peripheral };

// 名前はUTF-8で1〜32バイト、各文節は1〜64バイト。
inline constexpr char NAME[] = "tab5";
inline constexpr char WHEN[] = "ある日";
inline constexpr char WHERE[] = "パリに";
inline constexpr char WHO[] = "犬が";
inline constexpr char WHAT[] = "行く";
inline constexpr char WHY[] = "散歩のため";
inline constexpr char HOW[] = "従順な";
inline constexpr Role INITIAL_ROLE = Role::Auto;
inline constexpr std::int16_t RSSI_THRESHOLD = -65;
inline constexpr std::uint32_t ROLE_MIN_SECONDS = 1;
inline constexpr std::uint32_t ROLE_MAX_SECONDS = 5;
inline constexpr std::uint32_t EXCHANGE_TIMEOUT_SECONDS = 10;
inline constexpr std::uint32_t COOLDOWN_SECONDS = 30;

static_assert(sizeof(NAME) > 1 && sizeof(NAME) - 1 <= 32,
              "NAME must be 1 to 32 UTF-8 bytes");
static_assert(sizeof(WHEN) > 1 && sizeof(WHEN) - 1 <= 64,
              "WHEN must be 1 to 64 UTF-8 bytes");
static_assert(sizeof(WHERE) > 1 && sizeof(WHERE) - 1 <= 64,
              "WHERE must be 1 to 64 UTF-8 bytes");
static_assert(sizeof(WHO) > 1 && sizeof(WHO) - 1 <= 64,
              "WHO must be 1 to 64 UTF-8 bytes");
static_assert(sizeof(WHAT) > 1 && sizeof(WHAT) - 1 <= 64,
              "WHAT must be 1 to 64 UTF-8 bytes");
static_assert(sizeof(WHY) > 1 && sizeof(WHY) - 1 <= 64,
              "WHY must be 1 to 64 UTF-8 bytes");
static_assert(sizeof(HOW) > 1 && sizeof(HOW) - 1 <= 64,
              "HOW must be 1 to 64 UTF-8 bytes");
static_assert(RSSI_THRESHOLD >= -127 && RSSI_THRESHOLD <= 20,
              "RSSI_THRESHOLD must be between -127 and 20 dBm");
static_assert(ROLE_MIN_SECONDS >= 1 && ROLE_MIN_SECONDS <= ROLE_MAX_SECONDS &&
                  ROLE_MAX_SECONDS <= 3600,
              "role duration must satisfy 1 <= min <= max <= 3600");
static_assert(EXCHANGE_TIMEOUT_SECONDS >= 1 && EXCHANGE_TIMEOUT_SECONDS <= 3600,
              "exchange timeout must be between 1 and 3600 seconds");
static_assert(COOLDOWN_SECONDS <= 86400,
              "cooldown must be at most 86400 seconds");

}  // namespace surechigai::config
