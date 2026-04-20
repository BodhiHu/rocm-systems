//! Legacy re-exports for the Mirage daemon control protocol.
//!
//! The authoritative protocol definition now lives in [`crate::ctl`]. This
//! module remains as a compatibility layer for code that still imports request
//! and reply types from `mirage_schema::socket`.

pub use crate::ctl::{SessionSummary, SimulatorSummary, WorkloadSummary};
pub use crate::ctl::daemon::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, BootReply, BootRequest,
    CreateProfileReply, CreateProfileRequest,
    CreateWorkloadReply, CreateWorkloadRequest, DeleteProfileReply, DeleteProfileRequest,
    DeleteWorkloadReply, DeleteWorkloadRequest,
    ExecReply, ExecRequest, GetOverviewReply, GetOverviewRequest,
    HealthReply, HealthRequest, ListProfilesReply,
    ListProfilesRequest, ListSessionsReply, ListSessionsRequest, ListSimulatorsReply,
    ListSimulatorsRequest, ListWorkloadsReply, ListWorkloadsRequest, RegisterSimReply,
    RegisterSimRequest, ShowSimulatorReply, ShowSimulatorRequest,
    ShowWorkloadReply, ShowWorkloadRequest, ShutdownReply, ShutdownRequest,
    StatusReply, StatusRequest, TimeReply, TimeRequest,
};


