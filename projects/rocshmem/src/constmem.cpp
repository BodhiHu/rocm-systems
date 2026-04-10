#include "constmem.hpp"
#include "backend_bc.hpp"
#include "envvar.hpp"
#if defined(USE_GDA)
#include "gda/backend_gda.hpp"
#endif

namespace rocshmem {

extern Backend *backend;

void init_constant_memory(void) {
  std::string envstr;
  constmem_t constmem_values;

  memset(&constmem_values, 0, sizeof(constmem_t));

  envstr = envvar::gda::alltoallv_wg_algo;

  if (envstr.empty() || envstr.find("GET") != std::string::npos) {
    constmem_values.alltoall_wg_algo = gda::ALLTOALLV_WG_ALGO_GET;
  } else {
    constmem_values.alltoall_wg_algo = gda::ALLTOALLV_WG_ALGO_COPY;
  }

  constmem_values.backend_type = backend->get_type();
#if defined(USE_GDA)
  constmem_values.gda_provider = static_cast<GDABackend*>(backend)->get_gda_provider();
#endif

  CHECK_HIP(hipMemcpyToSymbol(HIP_SYMBOL(constmem), &constmem_values, sizeof(constmem_t)));
}

}
