//! Queue & dispatch — the `dispatcher` role. Claim queued tasks under a lease and launch one Job
//! each, reap finished/cancelled Jobs, reconcile data purges, and own the task state machine.

pub(crate) mod dispatcher;
pub(crate) mod index_sweeper;
pub(crate) mod lifecycle;
pub(crate) mod poller;
pub(crate) mod reaper;
pub(crate) mod tasks;
