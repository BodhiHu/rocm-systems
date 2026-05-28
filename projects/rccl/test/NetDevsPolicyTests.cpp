/*************************************************************************
 * Copyright (c) 2026 Advanced Micro Devices, Inc. All rights reserved.
 *
 * See LICENSE.txt for license information
 ************************************************************************/

// Unit tests for NCCL_NETDEVS_POLICY.
//
// For each interesting policy value an isolated child process:
//   * sets NCCL_NETDEVS_POLICY (or clears it),
//   * calls ncclTopoGetNetDevsPolicy() -- the internal parser used by the
//     topology graph search (ncclTopoSelectNets) to consume the env var
//     during ncclCommInitRank / ncclCommInitAll,
//   * asserts the returned policy enum + policyNum match what the
//     getNetDevsPolicyOnce() implementation in src/graph/topo.cc promises.
//
// The test target is rccl-UnitTestsFixturesDebug (Debug only): the symbol
// ncclTopoGetNetDevsPolicy lives inside librccl.so with default-hidden
// visibility in Release builds, so it is only linkable from the Debug
// fixtures binary -- exactly like ParamTests::ncclLoadParam,
// ArgCheckTests' argcheck helpers, etc.

#include <gtest/gtest.h>

#include <cstdlib>

#include "graph.h"  // netDevsPolicy enum, ncclTopoGetNetDevsPolicy()

#include "common/ProcessIsolatedTestRunner.hpp"

namespace RcclUnitTesting
{

// Default (env unset) -- should resolve to AUTO and leave policyNum alone.
TEST(NetDevsPolicyTests, DefaultUnset_ResolvesToAuto)
{
    RUN_ISOLATED_TEST(
        "DefaultUnset_ResolvesToAuto",
        []()
        {
            // Defensive: scrub any value the parent shell may have set.
            ::unsetenv("NCCL_NETDEVS_POLICY");

            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
            EXPECT_EQ(num,    -1);  // num is only written for MAX
        });
}

// Explicit AUTO.
TEST(NetDevsPolicyTests, Auto_ResolvesToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "Auto_ResolvesToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
            EXPECT_EQ(num,    -1);
        },
        {{"NCCL_NETDEVS_POLICY", "AUTO"}});
}

// Case-insensitive AUTO ("auto") -- topo.cc uses strcasecmp.
TEST(NetDevsPolicyTests, Auto_CaseInsensitive)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "Auto_CaseInsensitive",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
        },
        {{"NCCL_NETDEVS_POLICY", "auto"}});
}

// ALL policy.
TEST(NetDevsPolicyTests, All_ResolvesToAll)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "All_ResolvesToAll",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_ALL);
            EXPECT_EQ(num,    -1);  // num is only written for MAX
        },
        {{"NCCL_NETDEVS_POLICY", "ALL"}});
}

// Case-insensitive ALL.
TEST(NetDevsPolicyTests, All_CaseInsensitive)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "All_CaseInsensitive",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_ALL);
        },
        {{"NCCL_NETDEVS_POLICY", "All"}});
}

// MAX:N -- policy=MAX, policyNum=N.
TEST(NetDevsPolicyTests, MaxN_ResolvesToMaxWithCorrectNum)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "MaxN_ResolvesToMaxWithCorrectNum",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_MAX);
            EXPECT_EQ(num,    4);
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:4"}});
}

// Case-insensitive MAX:N ("max:8") -- topo.cc uses strncasecmp on "MAX:".
TEST(NetDevsPolicyTests, MaxN_CaseInsensitive)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "MaxN_CaseInsensitive",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_MAX);
            EXPECT_EQ(num,    8);
        },
        {{"NCCL_NETDEVS_POLICY", "max:8"}});
}

// MAX:0 -- envNum is zero, the parser rejects it (the `if (envNum > 0)`
// branch never runs), policy stays UNDEF in the parser and falls back to
// AUTO before the function returns.  Guards ncclTopoGetNetDevsPolicy()
// from tripping its own NETDEVS_POLICY_MAX-with-non-positive-N WARN
// guard, because the parser already prevented us from getting there.
TEST(NetDevsPolicyTests, MaxZero_FallsBackToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "MaxZero_FallsBackToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
            EXPECT_EQ(num,    -1);  // MAX branch was never taken
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:0"}});
}

// MAX:-1 -- atoi returns -1, fails the `> 0` check, falls back to AUTO.
TEST(NetDevsPolicyTests, MaxNegative_FallsBackToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "MaxNegative_FallsBackToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:-1"}});
}

// "MAX:" with no number -- atoi("") == 0, fails the `> 0` check, AUTO.
TEST(NetDevsPolicyTests, MaxEmpty_FallsBackToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "MaxEmpty_FallsBackToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:"}});
}

// Unrecognized value -- log warns "Unable to recognize ...", parser falls
// back to AUTO; the function returns success (no crash).
TEST(NetDevsPolicyTests, InvalidValue_FallsBackToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "InvalidValue_FallsBackToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
        },
        {{"NCCL_NETDEVS_POLICY", "BOGUS"}});
}

// Empty string -- ncclGetEnv() returns the empty string; none of the
// strcasecmp branches match, parser logs "Unable to recognize" and falls
// back to AUTO.
TEST(NetDevsPolicyTests, EmptyString_FallsBackToAuto)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "EmptyString_FallsBackToAuto",
        []()
        {
            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            int                num    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, &num), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_AUTO);
        },
        {{"NCCL_NETDEVS_POLICY", ""}});
}

// Idempotency -- two back-to-back parser calls in the same process must
// return the same policy/num.  std::call_once guarantees this; a future
// refactor that accidentally re-parses every call would break here.
TEST(NetDevsPolicyTests, Idempotent_AcrossCallsInSameProcess)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "Idempotent_AcrossCallsInSameProcess",
        []()
        {
            enum netDevsPolicy policy1 = NETDEVS_POLICY_UNDEF;
            int                num1    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy1, &num1), ncclSuccess);

            // Mutate the env var after the cache is established; the next
            // call must return the cached value, NOT the new env value.
            ::setenv("NCCL_NETDEVS_POLICY", "ALL", 1);

            enum netDevsPolicy policy2 = NETDEVS_POLICY_UNDEF;
            int                num2    = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy2, &num2), ncclSuccess);

            EXPECT_EQ(policy2, policy1);
            EXPECT_EQ(num2,    num1);
            EXPECT_EQ(policy2, NETDEVS_POLICY_MAX);
            EXPECT_EQ(num2,    2);
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:2"}});
}

// Null-output safety -- caller is allowed to pass NULL for either out-param.
TEST(NetDevsPolicyTests, AcceptsNullOutputPointers)
{
    RUN_ISOLATED_TEST_WITH_ENV(
        "AcceptsNullOutputPointers",
        []()
        {
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(nullptr, nullptr), ncclSuccess);

            enum netDevsPolicy policy = NETDEVS_POLICY_UNDEF;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(&policy, nullptr), ncclSuccess);
            EXPECT_EQ(policy, NETDEVS_POLICY_MAX);

            int num = -1;
            ASSERT_EQ(ncclTopoGetNetDevsPolicy(nullptr, &num), ncclSuccess);
            EXPECT_EQ(num, 3);
        },
        {{"NCCL_NETDEVS_POLICY", "MAX:3"}});
}

}  // namespace RcclUnitTesting
