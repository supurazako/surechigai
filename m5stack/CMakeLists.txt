cmake_minimum_required(VERSION 3.16)
project(surechigai_host_tests LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(surechigai_tests
  test/test_protocol.cpp
  src/game.cpp
  src/protocol.cpp
  src/exchange_state.cpp
)
target_include_directories(surechigai_tests PRIVATE include)
target_compile_options(surechigai_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)

enable_testing()
add_test(NAME protocol_and_state COMMAND surechigai_tests)
add_executable(gallery_tests test/test_gallery.cpp)
target_include_directories(gallery_tests PRIVATE include)
target_compile_options(gallery_tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
add_test(NAME gallery_rotation COMMAND gallery_tests)
