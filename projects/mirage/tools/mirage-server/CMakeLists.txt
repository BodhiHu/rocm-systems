include(FetchContent)

FetchContent_Declare(
    httplib
    GIT_REPOSITORY https://github.com/yhirose/cpp-httplib.git
    GIT_TAG        v0.18.0
)
FetchContent_MakeAvailable(httplib)

find_package(PkgConfig REQUIRED)
pkg_check_modules(GRPC REQUIRED IMPORTED_TARGET grpc++)

add_executable(mirage-server
    main.cpp
    ${MIRAGE_GRPC_SOURCE}
)
add_dependencies(mirage-server mirage_schemas)
target_include_directories(mirage-server PRIVATE ${MIRAGE_GENERATED_DIR})
target_link_libraries(mirage-server PRIVATE
    mirage
    mirage_warnings
    httplib::httplib
    PkgConfig::GRPC
)
