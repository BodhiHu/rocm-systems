// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "rocjitsu/code/dbt/binary_translator.h"

#include "rocjitsu/code/amdgpu_code_object.h"
#include "rocjitsu/code/amdgpu_elf.h"
#include "rocjitsu/code/basic_block.h"
#include "rocjitsu/code/patch/code_object_patcher.h"
#include "rocjitsu/isa/dbt/generated/encoding_cdna4_to_rdna4.h"
#include "rocjitsu/isa/dbt/generated/legalization_cdna4_to_rdna4.h"
#include "rocjitsu/isa/dbt/generated/legalization_types.h"
#include "rocjitsu/isa/decoder.h"

#include <cassert>
#include <cstring>

namespace rocjitsu {

BinaryTranslator::BinaryTranslator(rj_code_arch_t guest_arch, rj_code_arch_t host_arch)
    : guest_arch_(guest_arch), host_arch_(host_arch) {}

TranslatedCodeObject BinaryTranslator::translate(const AmdGpuCodeObject &obj) {
  TranslatedCodeObject result;
  result.host_arch = host_arch_;

  CodeObjectPatcher patcher(obj);
  auto text = patcher.text_bytes();
  if (text.empty()) {
    result.elf_bytes = patcher.emit();
    return result;
  }

  auto decoder = Decoder::create(guest_arch_);
  auto blocks = BasicBlock::build(obj, *decoder);

  std::vector<uint8_t> translated_text(text.size(), 0);

  for (auto &block : blocks) {
    uint64_t offset = block->start_offset();
    for (auto it = block->instructions().begin(); it != block->instructions().end(); ++it) {
      auto &inst = *it;
      uint32_t inst_size = inst.size();

      const uint32_t *src_words = reinterpret_cast<const uint32_t *>(text.data() + offset);
      uint32_t w0 = src_words[0];
      uint32_t w1 = inst_size > 4 ? src_words[1] : 0;
      uint32_t w2 = inst_size > 8 ? src_words[2] : 0;

      uint16_t encoding_id = w0 >> 23;
      uint16_t opcode = (w0 >> 16) & 0x7F;

      const auto *leg = lookup(kLegalization_cdna4_to_rdna4, encoding_id, opcode);
      uint16_t dst_opcode = leg ? leg->target_opcode : opcode;

      if (leg && leg->action == Action::Expand) {
        // TODO: semantic expansion (MFMA→WMMA, AccVGPR remap)
        result.warnings.push_back("EXPAND not yet implemented for " + std::string(inst.mnemonic()));
        std::memcpy(translated_text.data() + offset, src_words, inst_size);
        offset += inst_size;
        continue;
      }

      auto tr = translate_encoding_cdna4_to_rdna4(encoding_id, w0, w1, w2, dst_opcode);

      if (tr.word_count > 0 && tr.word_count * 4u <= inst_size) {
        std::memcpy(translated_text.data() + offset, tr.words, tr.word_count * 4u);
        if (tr.word_count * 4u < inst_size) {
          // TODO: size-changing translations need code cave
          result.warnings.push_back("Size-changing translation at offset " +
                                    std::to_string(offset));
        }
      } else {
        std::memcpy(translated_text.data() + offset, src_words, inst_size);
      }

      offset += inst_size;
    }
  }

  patcher.overwrite_text(translated_text);

  uint32_t dst_mach = 0;
  switch (host_arch_) {
  case ROCJITSU_CODE_ARCH_RDNA4:
    dst_mach = 0x48;
    break;
  default:
    break;
  }
  if (dst_mach)
    patcher.update_elf_flags(dst_mach);

  result.elf_bytes = patcher.emit();
  return result;
}

} // namespace rocjitsu
