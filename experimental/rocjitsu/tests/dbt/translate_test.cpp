// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "rocjitsu/isa/dbt/encoding_translator.h"
#include "rocjitsu/isa/dbt/generated/encoding_cdna4_to_rdna4.h"
#include "rocjitsu/isa/dbt/generated/encoding_fields.h"
#include "rocjitsu/isa/dbt/generated/legalization_cdna4_to_rdna4.h"
#include "rocjitsu/isa/dbt/generated/legalization_types.h"

#include <gtest/gtest.h>

#include <bit>
#include <cstdint>
#include <cstring>

namespace rocjitsu {
namespace {

TEST(CoherencyRemap, Gfx940ToGfx12AgentScope) {
  auto coh = remap_gfx940_to_gfx12({1, 0, 0});
  EXPECT_EQ(coh.scope, 1);
  EXPECT_EQ(coh.th, 0);
}

TEST(CoherencyRemap, Gfx940ToGfx12SystemScope) {
  auto coh = remap_gfx940_to_gfx12({1, 1, 0});
  EXPECT_EQ(coh.scope, 3);
  EXPECT_EQ(coh.th, 0);
}

TEST(CoherencyRemap, Gfx940ToGfx12NonTemporal) {
  auto coh = remap_gfx940_to_gfx12({0, 0, 1});
  EXPECT_EQ(coh.scope, 0);
  EXPECT_EQ(coh.th, 3);
}

TEST(CoherencyRemap, Gfx9GlcToGfx12) {
  auto coh_glc1 = remap_gfx9_to_gfx12({1});
  EXPECT_EQ(coh_glc1.scope, 2);
  EXPECT_EQ(coh_glc1.th, 0);

  auto coh_glc0 = remap_gfx9_to_gfx12({0});
  EXPECT_EQ(coh_glc0.scope, 0);
  EXPECT_EQ(coh_glc0.th, 0);
}

TEST(EncodingTranslator, Sop1PreservesRegisters) {
  cdna4::Sop1MachineInst src{};
  src.ssrc0 = 42;
  src.sdst = 17;
  src.op = 3;
  src.encoding = 0x17D;
  uint32_t w0 = std::bit_cast<uint32_t>(src);

  auto result = translate_encoding_cdna4_to_rdna4(kEnc_SOP1, w0, 0, 0, 5);

  ASSERT_EQ(result.word_count, 1);
  auto dst = std::bit_cast<rdna4::Sop1MachineInst>(result.words[0]);
  EXPECT_EQ(dst.ssrc0, 42);
  EXPECT_EQ(dst.sdst, 17);
  EXPECT_EQ(dst.op, 5);
  EXPECT_EQ(dst.encoding, 0x17D);
}

TEST(EncodingTranslator, Sop2PreservesRegisters) {
  cdna4::Sop2MachineInst src{};
  src.ssrc0 = 10;
  src.ssrc1 = 20;
  src.sdst = 30;
  src.op = 7;
  src.encoding = 0x2;
  uint32_t w0 = std::bit_cast<uint32_t>(src);

  auto result = translate_encoding_cdna4_to_rdna4(kEnc_SOP2, w0, 0, 0, 7);

  ASSERT_EQ(result.word_count, 1);
  auto dst = std::bit_cast<rdna4::Sop2MachineInst>(result.words[0]);
  EXPECT_EQ(dst.ssrc0, 10);
  EXPECT_EQ(dst.ssrc1, 20);
  EXPECT_EQ(dst.sdst, 30);
  EXPECT_EQ(dst.op, 7);
}

TEST(EncodingTranslator, SoppPreservesSimm16) {
  cdna4::SoppMachineInst src{};
  src.simm16 = 0xABCD;
  src.op = 12;
  src.encoding = 0x17F;
  uint32_t w0 = std::bit_cast<uint32_t>(src);

  auto result = translate_encoding_cdna4_to_rdna4(kEnc_SOPP, w0, 0, 0, 12);

  ASSERT_EQ(result.word_count, 1);
  auto dst = std::bit_cast<rdna4::SoppMachineInst>(result.words[0]);
  EXPECT_EQ(dst.simm16, 0xABCD);
  EXPECT_EQ(dst.op, 12);
}

TEST(EncodingTranslator, SmemRemapsCoherency) {
  cdna4::SmemMachineInst src{};
  src.sbase = 5;
  src.sdata = 3;
  src.glc = 1;
  src.nv = 0;
  src.op = 0;
  src.offset = 0x100;
  src.soffset = 0x7F;
  src.encoding = 0x3D;
  uint32_t words[2];
  std::memcpy(words, &src, sizeof(src));

  auto result = translate_encoding_cdna4_to_rdna4(kEnc_SMEM, words[0], words[1], 0, 0);

  ASSERT_EQ(result.word_count, 2);
  rdna4::SmemMachineInst dst{};
  std::memcpy(&dst, result.words, sizeof(dst));
  EXPECT_EQ(dst.sbase, 5);
  EXPECT_EQ(dst.sdata, 3);
  EXPECT_EQ(dst.scope, 2);
  EXPECT_EQ(dst.th, 0);
  EXPECT_EQ(dst.nv, 0);
  EXPECT_EQ(dst.soffset, 0x7F);
}

TEST(EncodingTranslator, Vop3PreservesModifiers) {
  cdna4::Vop3MachineInst src{};
  src.vdst = 10;
  src.src0 = 100;
  src.src1 = 200;
  src.src2 = 50;
  src.clamp = 1;
  src.omod = 2;
  src.neg = 5;
  src.abs = 3;
  src.op = 100;
  src.encoding = 0x35;
  uint32_t words[2];
  std::memcpy(words, &src, sizeof(src));

  auto result = translate_encoding_cdna4_to_rdna4(kEnc_VOP3, words[0], words[1], 0, 100);

  ASSERT_EQ(result.word_count, 2);
  rdna4::Vop3MachineInst dst{};
  std::memcpy(&dst, result.words, sizeof(dst));
  EXPECT_EQ(dst.vdst, 10);
  EXPECT_EQ(dst.src0, 100);
  EXPECT_EQ(dst.src1, 200);
  EXPECT_EQ(dst.src2, 50);
  EXPECT_EQ(dst.clamp, 1);
  EXPECT_EQ(dst.omod, 2);
  EXPECT_EQ(dst.neg, 5);
  EXPECT_EQ(dst.abs, 3);
}

TEST(EncodingTranslator, UnknownEncodingReturnsEmpty) {
  auto result = translate_encoding_cdna4_to_rdna4(0xFFFF, 0, 0, 0, 0);
  EXPECT_EQ(result.word_count, 0);
}

TEST(EncodingTranslator, DecodeEncodeRoundTrip) {
  cdna4::Sop1MachineInst src{};
  src.ssrc0 = 55;
  src.sdst = 33;
  src.op = 4;
  src.encoding = 0x17D;
  uint32_t w0 = std::bit_cast<uint32_t>(src);

  auto fields = decode_sop1_cdna4(w0);
  EXPECT_EQ(fields.ssrc0, 55u);
  EXPECT_EQ(fields.sdst, 33u);
  EXPECT_EQ(fields.op, 4u);

  auto result = encode_sop1_rdna4(fields, 4);
  ASSERT_EQ(result.word_count, 1);
  auto dst = std::bit_cast<rdna4::Sop1MachineInst>(result.words[0]);
  EXPECT_EQ(dst.ssrc0, 55);
  EXPECT_EQ(dst.sdst, 33);
  EXPECT_EQ(dst.op, 4);
}

TEST(LegalizationLookup, FindsKnownInstruction) {
  const auto *entry = lookup(kLegalization_cdna4_to_rdna4, 0, 0);
  EXPECT_NE(entry, nullptr);
  if (entry) {
    EXPECT_NE(entry->action, Action::Illegal);
  }
}

TEST(LegalizationLookup, ReturnsNullForUnknown) {
  const auto *entry = lookup(kLegalization_cdna4_to_rdna4, 0xFFFF, 0xFFFF);
  EXPECT_EQ(entry, nullptr);
}

TEST(LegalizationTable, NoIllegalEntries) {
  for (const auto &e : kLegalization_cdna4_to_rdna4) {
    EXPECT_NE(e.action, Action::Illegal)
        << "Unexpected ILLEGAL at encoding_id=" << e.src_encoding_id << " opcode=" << e.src_opcode;
  }
}

} // namespace
} // namespace rocjitsu
