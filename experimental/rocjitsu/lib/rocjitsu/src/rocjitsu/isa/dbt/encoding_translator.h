// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#pragma once

#include <cstdint>

namespace rocjitsu {

struct CoherencyGfx940 {
  uint8_t sc0;
  uint8_t sc1;
  uint8_t nt;
};

struct CoherencyGfx9 {
  uint8_t glc;
};

struct CoherencyGfx12 {
  uint8_t scope;
  uint8_t th;
};

inline constexpr CoherencyGfx12 remap_gfx940_to_gfx12(CoherencyGfx940 c) {
  return {static_cast<uint8_t>((c.sc1 << 1) | c.sc0), c.nt ? uint8_t(0x3) : uint8_t(0x0)};
}

inline constexpr CoherencyGfx12 remap_gfx9_to_gfx12(CoherencyGfx9 c) {
  return {c.glc ? uint8_t(0x2) : uint8_t(0x0), uint8_t(0x0)};
}

struct TranslationResult {
  uint32_t words[3]{};
  uint8_t word_count{0};
};

} // namespace rocjitsu
