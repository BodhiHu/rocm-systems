// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file semantic_translator.h
/// @brief Data-driven semantic translation for instructions whose semantics
/// change across ISA generations.
///
/// @details The encoding translator handles the ~80% of instructions where
/// only the binary encoding differs between ISAs. The semantic translator
/// handles the remaining ~20% where the instruction semantics themselves
/// change: waitcnt counter models, barrier split/merge, MFMA→WMMA
/// decomposition, AccVGPR register topology, and transpose load replacement.
///
/// The translator runs per-basic-block before the per-instruction encoding
/// translation loop. It scans for anchor instructions (identified by InstFlags),
/// applies the first matching rule, and produces a SemanticReplacement that the
/// binary translator writes in-place or via code caves. Unmatched instructions
/// fall through to the encoding translation path.
///
/// Rules are data-driven: each SemanticRule is a (name, anchor_flags, translate_fn)
/// tuple. Adding a new translation means adding one rule entry to the per-pair
/// rule table — no modifications to the translator framework or the main loop.

#pragma once

#include <cassert>
#include <cstdint>
#include <span>
#include <vector>

#include "rocjitsu/code/rj_code.h"

namespace rocjitsu {

class BasicBlock;
class Instruction;

/// @brief Result of a successful semantic translation: the source byte range
/// and the target instruction words that replace it.
struct SemanticReplacement {
  uint64_t start_offset = 0;          ///< First byte of the matched source range.
  uint64_t end_offset = 0;            ///< One past the last byte of the source range.
  std::vector<uint32_t> target_words; ///< Replacement instruction words for the host ISA.

  /// @brief Whether this replacement represents a successful match.
  [[nodiscard]] bool matched() const { return !target_words.empty(); }
};

/// @brief A single semantic translation rule.
///
/// @details Each rule identifies anchor instructions via anchor_flags (tested
/// against Instruction::flags()) and provides a translate function that
/// attempts to match and produce the replacement. Returns a SemanticReplacement
/// with matched()==true on success, or an empty replacement on failure.
struct SemanticRule {
  const char *name;      ///< Human-readable rule name for diagnostics.
  uint64_t anchor_flags; ///< Required InstFlags bits on the anchor instruction.

  /// @brief Attempt to translate an anchor instruction.
  ///
  /// @param anchor        The decoded guest instruction that triggered this rule.
  /// @param anchor_offset Byte offset of the anchor within the .text section.
  /// @param host_arch     Target ISA architecture.
  /// @returns SemanticReplacement with matched()==true on success, empty on failure.
  using TranslateFn = SemanticReplacement (*)(const Instruction &anchor, uint64_t anchor_offset,
                                              rj_code_arch_t host_arch);
  TranslateFn translate; ///< The translate function for this rule.
};

/// @brief Per-basic-block semantic translator.
///
/// @details Constructed once per BinaryTranslator with a (guest_arch, host_arch)
/// pair. Selects the appropriate rule table for the pair. The translate() method
/// scans a basic block's instructions, tests each against the rule table's
/// anchor_flags, and calls the translate function for matching rules.
class SemanticTranslator {
public:
  /// @brief Construct a translator for the given (guest, host) ISA pair.
  /// @param guest_arch  Source ISA architecture.
  /// @param host_arch   Target ISA architecture.
  SemanticTranslator(rj_code_arch_t guest_arch, rj_code_arch_t host_arch);

  /// @brief Scan a basic block for instructions requiring semantic translation.
  ///
  /// @param block  The decoded basic block to scan.
  /// @returns A list of non-overlapping replacements, ordered by start_offset.
  [[nodiscard]] std::vector<SemanticReplacement> translate(BasicBlock &block) const;

  /// @brief Whether any semantic rules exist for this (guest, host) pair.
  [[nodiscard]] bool has_rules() const { return !rules_.empty(); }

private:
  std::span<const SemanticRule> rules_; ///< Rule table for this (guest, host) pair.
  rj_code_arch_t host_arch_;            ///< Target ISA, passed to TranslateFn.
};

} // namespace rocjitsu
