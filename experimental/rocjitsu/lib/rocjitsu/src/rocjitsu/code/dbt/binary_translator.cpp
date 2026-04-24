// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "rocjitsu/code/dbt/binary_translator.h"

#include "rocjitsu/code/amdgpu_code_object.h"
#include "rocjitsu/code/amdgpu_elf.h"
#include "rocjitsu/code/basic_block.h"
#include "rocjitsu/code/dbt/generated/encoding_cdna4_to_rdna4.h"
#include "rocjitsu/code/dbt/generated/legalization_cdna4_to_rdna4.h"
#include "rocjitsu/code/dbt/generated/legalization_types.h"
#include "rocjitsu/code/dbt/instruction_builder.h"
#include "rocjitsu/code/dbt/semantic_translator.h"
#include "rocjitsu/code/patch/code_object_patcher.h"
#include "rocjitsu/isa/decoder.h"
#include "rocjitsu/isa/instruction.h"

#include <cassert>
#include <climits>
#include <cstring>

namespace rocjitsu {

namespace {

EncodingTranslateFn select_encoding_translator(rj_code_arch_t guest, rj_code_arch_t host) {
  if (guest == ROCJITSU_CODE_ARCH_CDNA4 && host == ROCJITSU_CODE_ARCH_RDNA4)
    return translate_encoding_cdna4_to_rdna4;
  return nullptr;
}

LegalizationLookupFn select_legalization(rj_code_arch_t guest, rj_code_arch_t host) {
  if (guest == ROCJITSU_CODE_ARCH_CDNA4 && host == ROCJITSU_CODE_ARCH_RDNA4) {
    return [](uint16_t enc_id, uint16_t opcode) -> const InstructionLegalization * {
      return lookup(kLegalization_cdna4_to_rdna4, enc_id, opcode);
    };
  }
  return nullptr;
}

} // namespace

BinaryTranslator::~BinaryTranslator() = default;

BinaryTranslator::BinaryTranslator(rj_code_arch_t guest_arch, rj_code_arch_t host_arch)
    : guest_arch_(guest_arch), host_arch_(host_arch),
      encoding_translate_(select_encoding_translator(guest_arch, host_arch)),
      legalization_lookup_(select_legalization(guest_arch, host_arch)),
      semantic_translator_(std::make_unique<SemanticTranslator>(guest_arch, host_arch)) {}

TranslatedCodeObject BinaryTranslator::translate(const AmdGpuCodeObject &obj) {
  TranslatedCodeObject result;
  result.host_arch = host_arch_;
  warnings_ = &result.warnings;

  CodeObjectPatcher patcher(obj);
  auto text = patcher.text_bytes();
  if (text.empty()) {
    result.elf_bytes = patcher.emit();
    return result;
  }

  auto decoder = Decoder::create(guest_arch_);
  if (!decoder) {
    result.warnings.push_back("unsupported guest_arch: no decoder available");
    result.elf_bytes = patcher.emit();
    return result;
  }
  auto blocks = BasicBlock::build(obj, *decoder);

  std::vector<uint8_t> translated_text(text.size(), 0);

  for (const auto &block : blocks) {
    auto replacements = semantic_translator_->translate(*block);

    for (const auto &repl : replacements)
      apply_semantic(repl, translated_text, patcher);

    uint64_t offset = block->start_offset();
    for (auto it = block->instructions().begin(); it != block->instructions().end(); ++it) {
      const auto &inst = *it;
      const uint32_t inst_size = inst.size();

      bool consumed = false;
      for (const auto &repl : replacements) {
        if (offset >= repl.start_offset && offset < repl.end_offset) {
          consumed = true;
          break;
        }
      }
      if (consumed) {
        offset += inst_size;
        continue;
      }

      const uint32_t *raw = inst.raw_encoding();
      if (!raw) {
        std::memcpy(translated_text.data() + offset, text.data() + offset, inst_size);
        offset += inst_size;
        continue;
      }

      const InstructionLegalization *leg = nullptr;
      if (legalization_lookup_)
        leg = legalization_lookup_(inst.encoding_id(), inst.opcode());

      const uint16_t dst_opcode = leg ? leg->target_opcode : inst.opcode();

      if (leg && leg->action == Action::Expand) {
        result.warnings.push_back("EXPAND not yet implemented for " + std::string(inst.mnemonic()));
        std::memcpy(translated_text.data() + offset, raw, inst_size);
        offset += inst_size;
        continue;
      }

      handle_encoding(inst, offset, translated_text, dst_opcode);
      offset += inst_size;
    }
  }

  patcher.overwrite_text(translated_text);

  const uint32_t dst_mach = elf_mach_for_arch(host_arch_);
  if (dst_mach)
    patcher.update_elf_flags(dst_mach);

  result.elf_bytes = patcher.emit();
  warnings_ = nullptr;
  return result;
}

void BinaryTranslator::apply_semantic(const SemanticReplacement &repl, std::vector<uint8_t> &text,
                                      CodeObjectPatcher &patcher) {
  assert(repl.matched() && "apply_semantic called with unmatched replacement");
  assert(repl.start_offset < repl.end_offset && "invalid replacement range");
  assert(repl.end_offset <= text.size() && "replacement exceeds text bounds");

  const uint32_t source_size = repl.end_offset - repl.start_offset;
  const uint32_t target_size = repl.target_words.size() * 4;

  if (target_size <= source_size) {
    std::memcpy(text.data() + repl.start_offset, repl.target_words.data(), target_size);
    if (target_size < source_size)
      std::memset(text.data() + repl.start_offset + target_size, 0, source_size - target_size);
    return;
  }

  const uint64_t cave_byte_offset = text.size() + patcher.cave_offset();
  const uint64_t stub_next = repl.start_offset + source_size;

  const auto fwd_dwords = static_cast<int64_t>(cave_byte_offset - stub_next) / 4;
  assert(fwd_dwords >= INT16_MIN && fwd_dwords <= INT16_MAX &&
         "branch offset exceeds simm16 range");

  const uint32_t stub = build_s_branch(static_cast<int16_t>(fwd_dwords));
  std::memcpy(text.data() + repl.start_offset, &stub, 4);
  for (uint64_t off = repl.start_offset + 4; off < repl.end_offset; off += 4) {
    const uint32_t nop = build_s_nop();
    std::memcpy(text.data() + off, &nop, 4);
  }

  auto cave_words = repl.target_words;
  const auto ret_dwords = (static_cast<int64_t>(stub_next) -
                           static_cast<int64_t>(cave_byte_offset + cave_words.size() * 4 + 4)) /
                          4;
  assert(ret_dwords >= INT16_MIN && ret_dwords <= INT16_MAX &&
         "return branch offset exceeds simm16 range");
  cave_words.push_back(build_s_branch(static_cast<int16_t>(ret_dwords)));

  patcher.append_cave_body(cave_words);
}

void BinaryTranslator::handle_encoding(const Instruction &inst, uint64_t offset,
                                       std::vector<uint8_t> &text, uint16_t dst_opcode) {
  const uint32_t *raw = inst.raw_encoding();
  assert(raw && "handle_encoding called without raw encoding");
  if (!encoding_translate_) {
    std::memcpy(text.data() + offset, raw, inst.size());
    return;
  }

  const uint32_t w0 = raw[0];
  const uint32_t w1 = inst.size() > 4 ? raw[1] : 0;
  const uint32_t w2 = inst.size() > 8 ? raw[2] : 0;

  auto tr = encoding_translate_(inst.encoding_id(), w0, w1, w2, dst_opcode, 0);

  if (tr.word_count > 0 && tr.word_count * 4u <= static_cast<uint32_t>(inst.size())) {
    std::memcpy(text.data() + offset, tr.words, tr.word_count * 4u);
  } else {
    std::memcpy(text.data() + offset, raw, inst.size());
  }
}

} // namespace rocjitsu
