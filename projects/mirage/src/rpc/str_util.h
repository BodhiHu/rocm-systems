#pragma once
// str_util.h — Helpers for converting between C/C++ strings and FlatBuffers
//              fixed-length Str types (Str15, Str31, Str63, Str255).

#include <cstring>
#include <string>
#include "rpc_generated.h"

namespace AmdgpuProxy {

/// Create a fixed-length Str<N> from a C string (null-terminated).
/// The source is truncated if it exceeds capacity.
template <typename StrT>
inline StrT make_str(const char* src) {
    StrT s{};
    if (src) {
        auto* dst = const_cast<uint8_t*>(s.data()->data());
        size_t cap = s.data()->size() - 1;  // reserve last byte for '\0'
        std::strncpy(reinterpret_cast<char*>(dst), src, cap);
        dst[cap] = 0;
    }
    return s;
}

/// Create a fixed-length Str<N> from a C string with explicit length.
template <typename StrT>
inline StrT make_str(const char* src, size_t len) {
    StrT s{};
    if (src) {
        auto* dst = const_cast<uint8_t*>(s.data()->data());
        size_t cap = s.data()->size() - 1;
        size_t n = len < cap ? len : cap;
        std::memcpy(dst, src, n);
        dst[n] = 0;
    }
    return s;
}

/// Create a fixed-length Str<N> from a std::string.
template <typename StrT>
inline StrT make_str(const std::string& src) {
    return make_str<StrT>(src.c_str(), src.size());
}

/// Read a fixed-length Str as a const char* (null-terminated).
template <typename StrT>
inline const char* str_data(const StrT& s) {
    return reinterpret_cast<const char*>(s.data()->data());
}

/// Read a fixed-length Str as a std::string.
template <typename StrT>
inline std::string str_to_string(const StrT& s) {
    return std::string(str_data(s));
}

}  // namespace AmdgpuProxy
