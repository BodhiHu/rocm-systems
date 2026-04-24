// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#pragma once

#include <cstdint>
#include <span>
#include <string>
#include <vector>

namespace rocjitsu {

class AmdGpuCodeObject;

class CodeObjectPatcher {
public:
  explicit CodeObjectPatcher(const AmdGpuCodeObject &obj);

  std::span<uint8_t> text_bytes();
  std::span<const uint8_t> text_bytes() const;

  void overwrite_text(std::span<const uint8_t> new_text);

  void update_elf_flags(uint32_t new_flags);

  std::vector<uint8_t> emit() const;

private:
  std::vector<uint8_t> image_;
  uint64_t text_offset_;
  uint64_t text_size_;
};

} // namespace rocjitsu
