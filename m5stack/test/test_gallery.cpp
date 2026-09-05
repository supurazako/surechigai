#include "gallery_rotation.hpp"

#include <cassert>
#include <cstdint>

int main() {
  surechigai::GalleryRotation rotation;
  assert(rotation.select(3, 3, 0) == 0);
  assert(rotation.select(3, 3, 10000) == 1);
  assert(rotation.select(3, 3, 12000) == -1);  // Polling must not undo rotation.
  assert(rotation.select(3, 3, 20000) == 2);
  assert(rotation.select(3, 3, 30000) == 0);
  assert(rotation.select(4, 4, 31000) == 0);  // Actual arrival takes priority.
  assert(rotation.select(4, 4, 41000) == 1);
  assert(rotation.select(-1, 0, 42000) == -1);
  assert(rotation.select(4, 1, 51000) == -1);

  surechigai::GalleryRotation wrap;
  assert(wrap.select(1, 2, UINT32_MAX - 4999U) == 0);
  assert(wrap.select(1, 2, 5000) == 1);

  surechigai::GalleryRotation retry;
  assert(retry.select(1, 1, 0) == 0);
  retry.failed(100);
  assert(retry.select(1, 1, 3000) == -1);
  assert(retry.select(1, 1, 3100) == 0);  // A single failed image must be retried.
}
