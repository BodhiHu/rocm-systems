//! Legacy re-exports for the Mirage daemon control protocol.
//!
//! The authoritative protocol definition now lives in [`crate::ctl`]. This
//! module remains as a compatibility layer for code that still imports request
//! and reply types from `mirage_schema::socket`.

pub use crate::ctl::{SessionSummary, SimulatorSummary, WorkloadSummary};
pub use crate::ctl::daemon::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, BootReply, BootRequest,
    CreateProfileReply, CreateProfileRequest, CreateSessionReply, CreateSessionRequest,
    CreateWorkloadReply, CreateWorkloadRequest, DeleteProfileReply, DeleteProfileRequest,
    DeleteSessionReply, DeleteSessionRequest, DeleteWorkloadReply, DeleteWorkloadRequest,
    ExecReply, ExecRequest, GetOverviewReply, GetOverviewRequest,
    HealthReply, HealthRequest, ListProfilesReply,
    ListProfilesRequest, ListSessionsReply, ListSessionsRequest, ListSimulatorsReply,
    ListSimulatorsRequest, ListWorkloadsReply, ListWorkloadsRequest, RegisterSimReply,
    RegisterSimRequest, ShowSimulatorReply, ShowSimulatorRequest,
    ShowWorkloadReply, ShowWorkloadRequest, ShutdownReply, ShutdownRequest,
    StatusReply, StatusRequest, TimeReply, TimeRequest,
};

/// Backward-compatible alias for the pre-refactor session-creation request.
pub type DashboardCreateSessionRequest = CreateSessionRequest;
/// Backward-compatible alias for the pre-refactor session-creation reply.
pub type DashboardCreateSessionReply = CreateSessionReply;
/// Backward-compatible alias for the pre-refactor session-deletion request.
pub type DashboardDeleteSessionRequest = DeleteSessionRequest;
/// Backward-compatible alias for the pre-refactor session-deletion reply.
pub type DashboardDeleteSessionReply = DeleteSessionReply;
