#pragma once

#include <cstdint>

namespace surechigai {

// Track arrivals independently from the image currently shown by the carousel.
class GalleryRotation {
 public:
  int select(int latest_id, int count, std::uint32_t now) {
    if (count <= 0) return -1;
    if (latest_id != latest_id_) {
      latest_id_ = latest_id;
      index_ = 0;
      last_rotate_ = now;
      retry_ = false;
      return index_;
    }
    if (retry_ && static_cast<std::uint32_t>(now - last_rotate_) >= 3000U) {
      retry_ = false;
      last_rotate_ = now;
      index_ %= count;
      return index_;
    }
    if (count > 1 && static_cast<std::uint32_t>(now - last_rotate_) >= 10000U) {
      last_rotate_ = now;
      index_ = (index_ + 1) % count;
      return index_;
    }
    return -1;
  }

  void failed(std::uint32_t now) {
    retry_ = true;
    last_rotate_ = now;
  }

 private:
  int latest_id_ = -1;
  int index_ = 0;
  std::uint32_t last_rotate_ = 0;
  bool retry_ = false;
};

}  // namespace surechigai
