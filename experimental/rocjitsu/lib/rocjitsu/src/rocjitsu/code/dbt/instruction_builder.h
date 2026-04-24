// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file instruction_builder.h
/// @brief ISA-neutral builder functions for common AMDGPU instructions.
///
/// @details Provides helpers for encoding frequently-used instructions
/// (s_branch, s_nop) in both the DBT and DBI layers. The SOPP encoding
/// format is identical across all AMDGPU ISA generations (CDNA1-4, RDNA1-4):
///   bits[31:23] = 0x17F (SOPP encoding prefix)
///   bits[22:16] = op (7-bit opcode)
///   bits[15:0]  = simm16 (16-bit signed/unsigned immediate)
///
/// These builders pack the 32-bit word directly from the bit layout
/// without depending on any ISA-specific machine_insts.h header.

#pragma once

#include <cstdint>

namespace rocjitsu {

/// @brief SOPP encoding prefix, consistent across all AMDGPU ISA generations.
inline constexpr uint32_t kSoppEncodingPrefix = 0x17F;

/// @brief SOPP opcodes, consistent across all AMDGPU ISA generations.
inline constexpr uint32_t kSoppOpNop = 0;
inline constexpr uint32_t kSoppOpBranch = 2;

/// @brief Pack a SOPP instruction word from its constituent fields.
///
/// @param op      7-bit SOPP opcode.
/// @param simm16  16-bit immediate field.
/// @returns The encoded 32-bit instruction word.
[[nodiscard]] inline constexpr uint32_t pack_sopp(uint32_t op, uint16_t simm16) {
  return (kSoppEncodingPrefix << 23) | (op << 16) | simm16;
}

/// @brief Encode an s_branch instruction.
///
/// @details The simm16 field is a signed dword offset relative to (PC + 4).
///
/// @param offset_dwords  Signed offset in dwords from (PC + 4) of the branch.
/// @returns The encoded 32-bit instruction word.
[[nodiscard]] inline constexpr uint32_t build_s_branch(int16_t offset_dwords) {
  return pack_sopp(kSoppOpBranch, static_cast<uint16_t>(offset_dwords));
}

/// @brief Encode an s_nop instruction.
///
/// @details The simm16 field specifies the number of additional stall cycles
/// (0 = 1 cycle nop).
///
/// @param cycles  Number of additional stall cycles (0-based).
/// @returns The encoded 32-bit instruction word.
[[nodiscard]] inline constexpr uint32_t build_s_nop(uint16_t cycles = 0) {
  return pack_sopp(kSoppOpNop, cycles);
}

} // namespace rocjitsu
