// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "aql_queue.h"
#include "embedded_schema.h"
#include "rocjitsu/config/config_loader.h"
#include "rocjitsu/vm/rj_vm.h"
#include "rocjitsu/vm/soc.h"
#include "simdojo/sim/simulation.h"
#include <gtest/gtest.h>
#include <cstdint>
#include <memory>
#include <string>

namespace {
using namespace rocjitsu;

// SMEM encoding (CDNA4).  sbase field indexes SGPR pairs.
constexpr uint32_t SMEM_ENC = 0x30u;
inline constexpr uint32_t smem_dw0(uint32_t op, uint32_t sdata, uint32_t sbase,
                                    bool glc, bool imm) {
  return (SMEM_ENC << 26) | ((op & 0xFFu) << 18) | ((imm ? 1u : 0u) << 17)
         | ((glc ? 1u : 0u) << 16) | ((sdata & 0x7Fu) << 6) | ((sbase / 2) & 0x3Fu);
}
inline constexpr uint32_t smem_dw1(uint32_t off) { return off & 0x1FFFFFu; }
inline constexpr uint32_t s_load(uint32_t sd, uint32_t sb, uint32_t, bool g=false) {
  return smem_dw0(0x00, sd, sb, g, true);
}
inline constexpr uint32_t s_load_x2(uint32_t sd, uint32_t sb, uint32_t, bool g=false) {
  return smem_dw0(0x04, sd, sb, g, true);
}
inline constexpr uint32_t s_store(uint32_t sd, uint32_t sb, uint32_t, bool g=false) {
  return smem_dw0(0x10, sd, sb, g, true);
}

inline constexpr uint32_t SG(uint32_t i) { return i; }
inline constexpr uint32_t IC(uint32_t v) { return 128 + v; }
inline constexpr uint32_t sop1(uint32_t op, uint32_t sd, uint32_t ss) {
  return (0x17Du << 23) | (sd << 16) | (op << 8) | ss;
}
inline constexpr uint32_t s_mov(uint32_t sd, uint32_t ss) { return sop1(0, sd, ss); }
inline constexpr uint32_t sopp(uint32_t op, uint16_t s=0) {
  return (0x17Fu << 23) | (op << 16) | s;
}
constexpr uint32_t S_WAIT = sopp(12, 0);
constexpr uint32_t S_END = sopp(1, 0);

struct F {
  std::unique_ptr<simdojo::SimulationEngine> eng;
  SoC* soc=nullptr;
  amdgpu::GpuMemory* mem=nullptr;
  F() {
    std::string j = R"({"max_ticks":10000,"num_threads":1,"vm":{"arch":"cdna4"},)"
      R"("topology":{"root":{"name":"soc","type":"soc","children":[)"
      R"({"name":"vram","type":"gpu_memory"},)"
      R"({"name":"xcd0","type":"xcd","children":[)"
      R"({"name":"l2","type":"l2_cache"},)"
      R"({"name":"cp","type":"command_processor"},)"
      R"({"name":"se0","type":"shader_engine","children":[)"
      R"({"name":"cu[0:1]","type":"compute_unit","config":[)"
      R"({"key":"num_wf_slots","value":"10"},)"
      R"({"key":"sgprs_per_wf","value":"104"},)"
      R"({"key":"vgprs_per_wf","value":"256"},)"
      R"({"key":"lds_size_kb","value":"64"})"
      R"(]}]}]}]},"links":[)"
      R"({"src":"xcd0.cp.req_0","dst":"xcd0.se0.cu0.cpl","latency":1,"weight":2},)"
      R"({"src":"xcd0.se0.cu0.req","dst":"xcd0.l2.cpl_0","latency":1,"weight":10})"
      R"(]}})";
    auto L = config::load_config_from_string(j, kEmbeddedSchema);
    soc=L.soc(); mem=L.memory();
    eng=std::make_unique<simdojo::SimulationEngine>(L.engine_config);
    eng->topology().set_root(L.take_root());
    L.wire_links(eng->topology()); eng->build();
  }
  amdgpu::ComputeUnitCore* cu() { return soc->xcd(0)->shader_engine(0)->compute_unit(0); }
  amdgpu::CommandProcessor* cp() { return soc->xcd(0)->command_processor(); }
  uint64_t wk(const uint32_t* c, size_t sz, uint64_t a=0x1000, uint32_t sg=104, uint32_t vg=256) {
    using namespace rocr::llvm::amdhsa;
    kernel_descriptor_t kd{};
    kd.kernel_code_entry_byte_offset = sizeof(kd);
    AMDHSA_BITS_SET(kd.compute_pgm_rsrc1, COMPUTE_PGM_RSRC1_GRANULATED_WORKITEM_VGPR_COUNT, (((vg)/8)-1));
    AMDHSA_BITS_SET(kd.compute_pgm_rsrc1, COMPUTE_PGM_RSRC1_GRANULATED_WAVEFRONT_SGPR_COUNT, (((sg)/8)-1));
    AMDHSA_BITS_SET(kd.compute_pgm_rsrc2, COMPUTE_PGM_RSRC2_USER_SGPR_COUNT, 2);
    mem->load_image(reinterpret_cast<const uint8_t*>(&kd), sizeof(kd), a);
    mem->load_image(reinterpret_cast<const uint8_t*>(c), sz, a+sizeof(kd));
    return a;
  }
};

TEST(ScalarLoadStoreTest, Load_NoGlc) {
  F f;
  f.mem->write32(0x8000, 0xDEADBEEF);
  const uint32_t c[]={s_mov(SG(4),255),0x8000,s_mov(SG(5),IC(0)),
                       s_load(79,4,0,false),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+79), 0xDEADBEEFu);
}

TEST(ScalarLoadStoreTest, Load_Glc) {
  F f;
  f.mem->write32(0x9000, 0xBEEFCAFE);
  const uint32_t c[]={s_mov(SG(4),255),0x9000,s_mov(SG(5),IC(0)),
                       s_load(80,4,0,true),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+80), 0xBEEFCAFEu);
}

TEST(ScalarLoadStoreTest, StoreThenLoad_NoGlc) {
  F f;
  uint32_t v=0x11223344;
  const uint32_t c[]={s_mov(SG(90),255),v,
                       s_mov(SG(4),255),0xA000,s_mov(SG(5),IC(0)),
                       s_store(90,4,0,false),smem_dw1(0),S_WAIT,
                       s_load(91,4,0,false),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+91), v);
  EXPECT_EQ(f.mem->read32(0xA000), v);
}

TEST(ScalarLoadStoreTest, StoreGlcThenLoadGlc) {
  F f;
  uint32_t v=0xAABBCCDD;
  const uint32_t c[]={s_mov(SG(90),255),v,
                       s_mov(SG(4),255),0xB000,s_mov(SG(5),IC(0)),
                       s_store(90,4,0,true),smem_dw1(0),S_WAIT,
                       s_load(91,4,0,true),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+91), v);
  EXPECT_EQ(f.mem->read32(0xB000), v);
}

TEST(ScalarLoadStoreTest, StoreGlc_LoadNoGlc) {
  F f;
  uint32_t v=0x51525354;
  const uint32_t c[]={s_mov(SG(90),255),v,
                       s_mov(SG(4),255),0xC000,s_mov(SG(5),IC(0)),
                       s_store(90,4,0,true),smem_dw1(0),S_WAIT,
                       s_load(92,4,0,false),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+92), v);
}

TEST(ScalarLoadStoreTest, LoadDwordx2) {
  F f;
  f.mem->write32(0xF000, 0xAAAA0000); f.mem->write32(0xF004, 0xBBBB0001);
  const uint32_t c[]={s_mov(SG(4),255),0xF000,s_mov(SG(5),IC(0)),
                       s_load_x2(93,4,0,false),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+93), 0xAAAA0000u);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+94), 0xBBBB0001u);
}

TEST(ScalarLoadStoreTest, LoadWithOffset) {
  F f;
  f.mem->write32(0x10010, 0xFACEFEED);
  const uint32_t c[]={s_mov(SG(4),255),0x10000,s_mov(SG(5),IC(0)),
                       s_load(95,4,0x10,false),smem_dw1(0x10),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+95), 0xFACEFEEDu);
}

TEST(ScalarLoadStoreTest, TwoAddresses_MixedGlc) {
  F f;
  uint32_t vr=0x1111, vc=0x2222;
  const uint32_t c[]={
    s_mov(SG(90),255),vr, s_mov(SG(4),255),0x11000,s_mov(SG(5),IC(0)),
    s_store(90,4,0,false),smem_dw1(0),S_WAIT,
    s_mov(SG(91),255),vc, s_mov(SG(6),255),0x12000,s_mov(SG(7),IC(0)),
    s_store(91,6,0,true),smem_dw1(0),S_WAIT,
    s_load(92,4,0,false),smem_dw1(0),S_WAIT,
    s_load(93,6,0,true),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  auto* w = f.cu()->wf(0); ASSERT_NE(w,nullptr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+92), vr);
  EXPECT_EQ(f.cu()->read_sgpr(w->sgpr_alloc().base+93), vc);
  EXPECT_EQ(f.mem->read32(0x11000), vr);
  EXPECT_EQ(f.mem->read32(0x12000), vc);
}

TEST(ScalarLoadStoreTest, StoreFlushReadback) {
  F f;
  uint32_t v=0xDEADBABE;
  const uint32_t c[]={s_mov(SG(90),255),v,
                       s_mov(SG(4),255),0xE000,s_mov(SG(5),IC(0)),
                       s_store(90,4,0,false),smem_dw1(0),S_WAIT,S_END};
  auto q = test::AqlQueue(f.mem, f.cp());
  q.dispatch(f.wk(c,sizeof(c)),64); f.eng->run();
  f.cu()->l1_scalar().writeback_all();
  f.cu()->l1_scalar().invalidate_all();
  uint32_t ld=0;
  f.cu()->l1_scalar().load(0xE000,1,&ld,0,std::nullopt);
  EXPECT_EQ(ld, v);
}

} // namespace
