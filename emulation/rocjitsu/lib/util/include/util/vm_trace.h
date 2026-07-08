// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#ifndef UTIL_VM_TRACE_H_
#define UTIL_VM_TRACE_H_

#include <cassert>
#include <cstdlib>
#include <optional>
#include <fstream>
#include <string_view>
#include <chrono>
#include <syncstream>
#include <mutex>
#include <atomic>
#include <string>
#include <filesystem>
#include <map>

namespace util {

class vm_trace {
public:

#define SYNCED_VM_DBG_PRINT
#ifdef SYNCED_VM_DBG_PRINT
  static constexpr bool synced_vm_dbg_print = true;
#else
  static constexpr bool synced_vm_dbg_print = false;
#endif

  inline static std::atomic<bool> __potential_barrier_dead_loop_detected{false};

  inline static const std::string wf_inst_trace_mode = []() {
    const char *env = std::getenv("WF_INST_TRACE_MODE");
    auto mode = std::string(env != nullptr ? env : "off");
    assert(mode == "off" || mode == "all" || mode == "critical");

    std::filesystem::create_directories("/tmp/rocivm_logs");

    std::cout << "[rocjit] wf_inst_trace_mode = " << mode << std::endl;
    return mode;
  }();

  /// @brief Helper to print at most once per second, with a timestamp.
  template <typename Func>
  static void synced_print_per_sec(
    std::chrono::system_clock::time_point& last_print,
    std::ostream& os, Func&& printer
  ) {
    auto now = std::chrono::system_clock::now();
    if (now - last_print >= std::chrono::seconds(1)) {
      auto now_t = std::chrono::system_clock::to_time_t(now);

      std::osyncstream sync_os(os);
      sync_os << std::put_time(std::localtime(&now_t), "[%H:%M:%S]") << ' ';

      printer(sync_os);
      sync_os << '\n' << std::flush;
      last_print = now;
    }
  }

  static std::string format_ts(const std::chrono::system_clock::time_point& tp) {
    std::string time = std::to_string(tp.time_since_epoch().count());
    for (int i = time.length() - 3; i > 0; i -= 3) {
        time.insert(i, ",");
    }
    return time;
  }

  template<bool Enable = synced_vm_dbg_print>
  static std::enable_if_t<Enable, void> trace_wf_instructions(
    uint32_t wf_id, uint64_t pc, std::string_view mnemonic, std::string inst
  ) {
    if (wf_inst_trace_mode == "all") {
      static std::unordered_map<uint32_t, std::ofstream> wf_log_files;
      auto& ofs = wf_log_files.try_emplace(
        wf_id,
        std::format("/tmp/rocivm_logs/wf_{}_all_insts.txt", wf_id)
      ).first->second;

      std::string time = format_ts(std::chrono::system_clock::now());

      ofs << "[" << time << "] "
          << "EXEC wf=" << std::format("{:04d}", wf_id) << ", pc=" << pc
          << ", inst= " << inst << '\n'
          << std::flush;
    } else if (wf_inst_trace_mode == "critical") {
      static constexpr size_t kHistorySize = 200;
      struct InstRecord {
        std::chrono::system_clock::time_point timestamp;
        uint32_t wf_id;
        uint64_t pc;
        std::string inst;
      };
      static std::map<uint32_t, std::vector<InstRecord>> inst_history;
      static std::map<uint32_t, size_t> hist_cnt;
      static std::unordered_map<uint32_t, std::ofstream> wf_log_files;

      size_t wf_hist_cnt = 0;

      static std::mutex history_mutex;
      {
        // LOCK HERE UNTIL THE END OF THIS BLOCK:
        // This is to ensure that the instruction history logging is thread-safe.
        std::lock_guard<std::mutex> lock(history_mutex);

        if (inst_history[wf_id].size() < kHistorySize) {
          inst_history[wf_id].resize(kHistorySize);
          hist_cnt[wf_id] = 0;
        }
        wf_hist_cnt = hist_cnt[wf_id];

        auto &rec = inst_history[wf_id][wf_hist_cnt];
        rec.wf_id = wf_id;
        rec.pc = pc;
        rec.inst = inst;
        rec.timestamp = std::chrono::system_clock::now();
      }

      bool is_trap = mnemonic.find("s_trap") != std::string_view::npos;
      bool deadlock = util::vm_trace::__potential_barrier_dead_loop_detected.load(std::memory_order_relaxed);

      // Flush when:
      //  - a trap fires
      //  - OR detected barrier dead-loop
      //  /*- OR the history window is full*/
      if (is_trap || deadlock /*|| wf_hist_cnt == (kHistorySize-1)*/) {

        auto &ofs = wf_log_files.try_emplace(
          wf_id,
          std::format("/tmp/rocivm_logs/wf_{}_insts.txt", wf_id)
        ).first->second;

        for (size_t i = 0; i <= wf_hist_cnt; ++i) {
          auto &r = inst_history[wf_id][i];
          std::string time = format_ts(r.timestamp);

          ofs << "[" << time << "] "
              << "EXEC wf=" << std::format("{:04d}", wf_id) << ", pc=" << r.pc
              << ", inst= " << r.inst << '\n';
        }
        ofs << std::flush;

        hist_cnt[wf_id] = 0;
      } else {
        hist_cnt[wf_id] = (hist_cnt[wf_id] + 1) % kHistorySize;
      }
    } else {
      // pass
    }
  }
};

} // namespace util

#endif // UTIL_VM_TRACE_H_
