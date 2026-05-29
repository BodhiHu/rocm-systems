cmake_minimum_required(VERSION 3.21)
project(rocprofiler-compute-tool LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Sanitizers are only wired in when this subtree is built as part of the
# top-level rocprofiler-compute project. The runtime "native tool" path
# does not have the parent cmake/ dir and is not an ASAN target.
set(rocprofiler_compute_sanitizers_module "${CMAKE_SOURCE_DIR}/cmake/sanitizers.cmake")
if(EXISTS "${rocprofiler_compute_sanitizers_module}")
    include("${rocprofiler_compute_sanitizers_module}")
    enable_sanitizer()
endif()

set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)

# Set cmake_prefix_path for searching, ROCM_PATH if avail otherwise default to general rocm install path
set(CMAKE_PREFIX_PATH $ENV{ROCM_PATH} "/opt/rocm/")

find_package(
    rocprofiler-sdk
    HINTS $ENV{ROCM_PATH}/lib/cmake /opt/rocm/lib/cmake
    CONFIG
    REQUIRED
)

add_subdirectory(external)
add_subdirectory(rocprofiler_compute_tool)
add_subdirectory(utils)
