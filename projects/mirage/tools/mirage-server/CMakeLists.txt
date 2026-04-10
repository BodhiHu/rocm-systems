include(FetchContent)

FetchContent_Declare(
    httplib
    GIT_REPOSITORY https://github.com/yhirose/cpp-httplib.git
    GIT_TAG        v0.18.0
)
FetchContent_MakeAvailable(httplib)

add_executable(mirage-server main.cpp)
target_link_libraries(mirage-server PRIVATE
    mirage
    mirage_warnings
    httplib::httplib
)
