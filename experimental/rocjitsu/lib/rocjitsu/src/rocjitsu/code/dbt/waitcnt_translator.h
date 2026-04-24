// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file waitcnt_translator.h
/// @brief Utility functions for decoding and re-encoding s_waitcnt across ISA generations.
///
/// @details GFX9 (CDNA1-4) uses a monolithic s_waitcnt with vmcnt/lgkmcnt/expcnt packed
/// into a 16-bit simm16 field. GFX12 (RDNA4) splits this into separate s_wait_loadcnt,
/// s_wait_storecnt_dscnt, s_wait_kmcnt, and s_wait_expcnt instructions. These utility
/// functions handle the decode and re-encode; they are called by the waitcnt semantic
/// rule in semantic_translator.cpp.
///
/// Sentinel values indicate "don't wait" (no change to the counter):
/// - vmcnt:  0x3F (6-bit max)
/// - lgkmcnt: 0x0F (4-bit max for CDNA)
/// - expcnt: 0x07 (3-bit max)

#pragma once

#include <cstdint>
#include <vector>

namespace rocjitsu {

/// @brief Decoded wait-counter values from a GFX9 s_waitcnt simm16 field.
///
/// @details Sentinel values indicate "don't wait" for that counter. The binary
/// translator uses these to determine which GFX12 s_wait_* instructions to emit.
struct WaitcntValues {
  uint8_t vmcnt = 0x3F;   ///< VM count (loads + stores on GFX9). Sentinel: 0x3F.
  uint8_t lgkmcnt = 0x0F; ///< LDS/GDS/Kmem count. Sentinel: 0x0F.
  uint8_t expcnt = 0x07;  ///< Export count. Sentinel: 0x07.
};

/// @brief Decode a GFX9 s_waitcnt simm16 field into individual counter values.
///
/// @details GFX9 simm16 layout: vmcnt[3:0] at bits [3:0], expcnt[2:0] at bits [6:4],
/// lgkmcnt[3:0] at bits [11:8], vmcnt[5:4] at bits [15:14].
///
/// @param simm16  The raw 16-bit immediate from the s_waitcnt instruction.
/// @returns Decoded counter values with sentinels for "don't wait" counters.
[[nodiscard]] WaitcntValues decode_waitcnt_gfx9(uint16_t simm16);

/// @brief Encode wait-counter values as GFX12 split s_wait_* instruction words.
///
/// @details Emits one instruction per non-sentinel counter:
/// - vmcnt < sentinel → s_wait_loadcnt(vmcnt) + s_wait_storecnt_dscnt(vmcnt)
///   (GFX9 vmcnt covers both loads and stores; GFX12 splits them)
/// - lgkmcnt < sentinel → s_wait_kmcnt(lgkmcnt)
/// - expcnt < sentinel → s_wait_expcnt(expcnt)
///
/// If all counters are at sentinel (all relaxed), emits a single s_nop.
///
/// @param vals  The decoded counter values.
/// @returns Vector of GFX12 instruction words (1–4 words).
[[nodiscard]] std::vector<uint32_t> encode_waitcnt_gfx12(const WaitcntValues &vals);

} // namespace rocjitsu
