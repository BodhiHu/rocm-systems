// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file semantic_translator.cpp
/// @brief Semantic translator implementation and per-pair rule tables.

#include "rocjitsu/code/dbt/semantic_translator.h"

#include "rocjitsu/code/basic_block.h"
#include "rocjitsu/code/dbt/waitcnt_translator.h"
#include "rocjitsu/isa/arch/amdgpu/cdna4/machine_insts.h"
#include "rocjitsu/isa/instruction.h"

#include <cassert>

namespace rocjitsu {

namespace {

/// @brief Translate a GFX9 s_waitcnt to GFX12 split s_wait_* instructions.
///
/// @details Reinterprets the raw encoding as a CDNA4 SoppMachineInst to access
/// the simm16 field by name, decodes it into individual counter values via
/// decode_waitcnt_gfx9(), and re-encodes as GFX12 split wait instructions.
SemanticReplacement translate_waitcnt_gfx9_to_gfx12(const Instruction &inst, uint64_t offset,
                                                    rj_code_arch_t) {
  if (!inst.raw_encoding())
    return {};

  const auto &sopp = *reinterpret_cast<const cdna4::SoppMachineInst *>(inst.raw_encoding());
  auto vals = decode_waitcnt_gfx9(sopp.simm16);
  auto words = encode_waitcnt_gfx12(vals);
  return {offset, offset + inst.size(), std::move(words)};
}

/// @brief Semantic rule table for CDNA4 → RDNA4 translation.
constexpr SemanticRule kRules_cdna4_to_rdna4[] = {
    {"waitcnt_gfx9_to_gfx12", WAITCNT, translate_waitcnt_gfx9_to_gfx12},
};

} // namespace

SemanticTranslator::SemanticTranslator(rj_code_arch_t guest, rj_code_arch_t host)
    : host_arch_(host) {
  if (guest == ROCJITSU_CODE_ARCH_CDNA4 && host == ROCJITSU_CODE_ARCH_RDNA4)
    rules_ = kRules_cdna4_to_rdna4;
}

std::vector<SemanticReplacement> SemanticTranslator::translate(BasicBlock &block) const {
  std::vector<SemanticReplacement> result;
  uint64_t offset = block.start_offset();
  for (auto it = block.instructions().begin(); it != block.instructions().end(); ++it) {
    const auto &inst = *it;
    for (const auto &rule : rules_) {
      if (!(inst.flags() & rule.anchor_flags))
        continue;
      auto repl = rule.translate(inst, offset, host_arch_);
      if (repl.matched()) {
        result.push_back(std::move(repl));
        break;
      }
    }
    offset += inst.size();
  }
  return result;
}

} // namespace rocjitsu
