//! Device-local workspace state, split by responsibility: local identity and
//! workspace context (`local identity`), cross-process write locks (`locks`),
//! workspace CRUD over the readable event log (`workspace_store`), and
//! heatmap presentation of saved reports (`heatmap`).

mod heatmap;
mod identity;
mod locks;
mod portable_backup;
mod workspace_store;

pub use crate::context_documents::ContextFileReference;
pub use heatmap::{
    HeatmapCell, HeatmapCriterion, HeatmapWeek, ReportHistoryCell, ReportHistoryPage,
    ReportHistoryRun,
};
pub use identity::ProjectContext;
pub use workspace_store::{
    ApproveGoalRequest, BackupScheduleStatus, CreateWorkspaceRequest, DecisionFunnelSummary,
    LocalWorkspaceStore, OpenWorkspaceRequest, PortableBackupReceipt, PortableRestoreReceipt,
    ReadyActionRequest, RecentWorkspace, ReliabilitySummary, ReplaceGoalsRequest,
    ScheduledBackupRunReceipt, UpdateWorkspaceContextRequest,
};
