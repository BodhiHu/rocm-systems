cmake_minimum_required(VERSION 3.28)
project(mirage LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# --------------------------------------------------------------------------
# Options
# --------------------------------------------------------------------------
option(MIRAGE_BUILD_TESTS "Build mirage tests" ON)

# --------------------------------------------------------------------------
# FlatBuffers schema generation
# --------------------------------------------------------------------------
find_program(FLATC flatc REQUIRED)

set(MIRAGE_SCHEMA_DIR  ${CMAKE_CURRENT_SOURCE_DIR}/schema)
set(MIRAGE_GENERATED_DIR ${CMAKE_CURRENT_BINARY_DIR}/generated)
file(MAKE_DIRECTORY ${MIRAGE_GENERATED_DIR})

set(MIRAGE_SCHEMAS
    ${MIRAGE_SCHEMA_DIR}/common.fbs
    ${MIRAGE_SCHEMA_DIR}/config.fbs
    ${MIRAGE_SCHEMA_DIR}/container.fbs
    ${MIRAGE_SCHEMA_DIR}/simulator.fbs
    ${MIRAGE_SCHEMA_DIR}/socket.fbs
)

# Compute expected generated headers (foo.fbs → foo_generated.h)
set(MIRAGE_GENERATED_HEADERS)
foreach(schema ${MIRAGE_SCHEMAS})
    get_filename_component(stem ${schema} NAME_WE)
    list(APPEND MIRAGE_GENERATED_HEADERS
         ${MIRAGE_GENERATED_DIR}/${stem}_generated.h)
endforeach()

add_custom_command(
    OUTPUT  ${MIRAGE_GENERATED_HEADERS}
    COMMAND ${FLATC} --cpp -o ${MIRAGE_GENERATED_DIR}
            -I ${MIRAGE_SCHEMA_DIR}
            ${MIRAGE_SCHEMAS}
    DEPENDS ${MIRAGE_SCHEMAS}
    COMMENT "Generating FlatBuffer C++ headers"
    VERBATIM
)

add_custom_target(mirage_schemas DEPENDS ${MIRAGE_GENERATED_HEADERS})

# --------------------------------------------------------------------------
# Compiler warnings
# --------------------------------------------------------------------------
add_library(mirage_warnings INTERFACE)
if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(mirage_warnings INTERFACE
        -Wall -Wextra -Wpedantic -Werror)
elseif(MSVC)
    target_compile_options(mirage_warnings INTERFACE /W4 /WX)
endif()

# --------------------------------------------------------------------------
# Library
# --------------------------------------------------------------------------
add_subdirectory(lib)

# --------------------------------------------------------------------------
# Tools
# --------------------------------------------------------------------------
option(MIRAGE_BUILD_TOOLS "Build mirage tools (server, etc.)" ON)
if(MIRAGE_BUILD_TOOLS)
    add_subdirectory(tools)
endif()

# --------------------------------------------------------------------------
# Tests
# --------------------------------------------------------------------------
if(MIRAGE_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()

# --------------------------------------------------------------------------
# Dashboard (npm build)
# --------------------------------------------------------------------------
set(MIRAGE_DASHBOARD_SRC ${CMAKE_CURRENT_SOURCE_DIR}/dashboard)
set(MIRAGE_DASHBOARD_DIST ${MIRAGE_DASHBOARD_SRC}/dist)

add_custom_command(
    OUTPUT  ${MIRAGE_DASHBOARD_DIST}/index.html
    COMMAND npm install --prefer-offline --no-audit --no-fund
    COMMAND npm run build
    WORKING_DIRECTORY ${MIRAGE_DASHBOARD_SRC}
    DEPENDS
        ${MIRAGE_DASHBOARD_SRC}/package.json
        ${MIRAGE_DASHBOARD_SRC}/tsconfig.json
        ${MIRAGE_DASHBOARD_SRC}/vite.config.ts
        ${MIRAGE_DASHBOARD_SRC}/index.html
        ${MIRAGE_DASHBOARD_SRC}/src/main.tsx
        ${MIRAGE_DASHBOARD_SRC}/src/App.tsx
        ${MIRAGE_DASHBOARD_SRC}/src/App.css
        ${MIRAGE_DASHBOARD_SRC}/src/index.css
    COMMENT "Building dashboard (npm install + vite build)"
    VERBATIM
)

add_custom_target(mirage_dashboard DEPENDS ${MIRAGE_DASHBOARD_DIST}/index.html)

# --------------------------------------------------------------------------
# Convenience: `cmake --build build --target run`
# --------------------------------------------------------------------------
if(MIRAGE_BUILD_TOOLS)
    add_custom_target(run
        COMMAND $<TARGET_FILE:mirage-server>
                --static ${MIRAGE_DASHBOARD_DIST}
        DEPENDS mirage-server mirage_dashboard
        USES_TERMINAL
        COMMENT "Running mirage-server"
        VERBATIM
    )
endif()
