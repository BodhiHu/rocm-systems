// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "rocjitsu/code/patch/code_object_patcher.h"

#include "rocjitsu/code/amdgpu_code_object.h"
#include "rocjitsu/code/amdgpu_elf.h"

#include <cassert>
#include <cstring>

namespace rocjitsu {

CodeObjectPatcher::CodeObjectPatcher(const AmdGpuCodeObject &obj)
    : image_(obj.image_data(), obj.image_data() + obj.image_size()), text_offset_(0),
      text_size_(0) {
  auto &text_secs = obj.text_sections();
  if (!text_secs.empty()) {
    text_offset_ = text_secs[0]->sectionOffset();
    text_size_ = text_secs[0]->size();
  }
}

std::span<uint8_t> CodeObjectPatcher::text_bytes() {
  return {image_.data() + text_offset_, text_size_};
}

std::span<const uint8_t> CodeObjectPatcher::text_bytes() const {
  return {image_.data() + text_offset_, text_size_};
}

void CodeObjectPatcher::overwrite_text(std::span<const uint8_t> new_text) {
  assert(new_text.size() == text_size_);
  std::memcpy(image_.data() + text_offset_, new_text.data(), new_text.size());
}

void CodeObjectPatcher::update_elf_flags(uint32_t new_flags) {
  auto *ehdr = reinterpret_cast<Elf64_Ehdr *>(image_.data());
  ehdr->e_flags = (ehdr->e_flags & ~EF_AMDGPU_MACH) | (new_flags & EF_AMDGPU_MACH);
}

void CodeObjectPatcher::append_cave_body(std::span<const uint32_t> words) {
  auto *bytes = reinterpret_cast<const uint8_t *>(words.data());
  cave_body_.insert(cave_body_.end(), bytes, bytes + words.size() * 4);
}

std::vector<uint8_t> CodeObjectPatcher::emit() const {
  if (cave_body_.empty())
    return image_;

  std::vector<uint8_t> result = image_;
  uint64_t cave_file_offset = result.size();

  result.insert(result.end(), cave_body_.begin(), cave_body_.end());

  // TODO: append a proper .rj_translations section header to the ELF section
  // header table and update e_shnum. For now the cave body is appended as raw
  // bytes after the image — the code cave stubs in .text use s_branch with
  // offsets computed relative to the text section end, so this works for
  // execution even without a formal section header.
  (void)cave_file_offset;

  return result;
}

} // namespace rocjitsu
