use ractor::concurrency::{Duration, Instant};
use ractor::{ActorCell, ActorId, ActorProcessingErr, ActorRef, Message, SpawnErr};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;

/// Possible errors from the supervisor’s logic.
#[derive(Error, Debug, Clone)]
pub enum SupervisorError {
    #[error("Child '{child_id}' not found in specs")]
    ChildNotFound { child_id: String },

    #[error("Child '{pid}' does not have a name set")]
    ChildNameNotSet { pid: ActorId },

    #[error("Spawn error '{child_id}': {reason}")]
    ChildSpawnError { child_id: String, reason: String },

    #[error("Meltdown: {reason}")]
    Meltdown { reason: String },
}

/// A function pointer for computing **child-level** backoff delays before re-spawning a child.
///
/// This function is invoked each time a child fails:
/// ```ignore
/// (child_id, current_restart_count, last_fail_instant, reset_after)
///    -> Option<Duration>
/// ```
/// If you return `Some(duration)`, the supervisor will wait that amount of time before actually re-spawning the child.
/// If `None`, it restarts immediately.
#[derive(Clone)]
pub struct ChildBackoffFn(pub Arc<BackoffFn>);

type BackoffFn = dyn Fn(&str, usize, Instant, Option<Duration>) -> Option<Duration> + Send + Sync;

impl ChildBackoffFn {
    /// Create a new ChildBackoffFn from a closure.
    ///
    /// # Example
    /// ```rust, ignore
    /// let backoff_fn = ChildBackoffFn::new(|child_id, restart_count, last_restart, reset_after| {
    ///     // Your backoff logic here...
    ///     Some(Duration::from_millis(100))
    /// });
    /// ```
    pub fn new<F>(func: F) -> Self
    where
        F: Fn(&str, usize, Instant, Option<Duration>) -> Option<Duration> + Send + Sync + 'static,
    {
        Self(Arc::new(func))
    }

    /// Calls the inner backoff function.
    pub fn call(
        &self,
        child_id: &str,
        restart_count: usize,
        last_restart: Instant,
        reset_after: Option<Duration>,
    ) -> Option<Duration> {
        (self.0)(child_id, restart_count, last_restart, reset_after)
    }
}

/// The future returned by a [`SpawnFn`].
pub type SpawnFuture = Pin<Box<dyn Future<Output = Result<ActorCell, SpawnErr>> + Send>>;

/// User-provided closure to spawn a child. You typically call `Supervisor::spawn_linked` here.
#[derive(Clone)]
pub struct SpawnFn(pub Arc<dyn Fn(ActorCell, String) -> SpawnFuture + Send + Sync>);

impl SpawnFn {
    /// Create a new SpawnFn from a closure.
    ///
    /// # Example
    /// ```rust,ignore
    /// let spawn_fn = SpawnFn::new(move |cell, id| {
    ///     // Your actor-spawning logic here...
    ///     spawn_my_actor(cell, id).await
    /// });
    /// ```
    pub fn new<F, Fut>(func: F) -> Self
    where
        F: Fn(ActorCell, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ActorCell, SpawnErr>> + Send + 'static,
    {
        Self(Arc::new(move |cell, id| Box::pin(func(cell, id))))
    }

    /// Calls the inner spawn function.
    pub fn call(&self, cell: ActorCell, id: String) -> SpawnFuture {
        (self.0)(cell, id)
    }
}

/// Defines how a child actor is restarted after it exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// Always restart, no matter how the child terminates.
    Permanent,
    /// Restart only if the child terminates abnormally (a panic or error).
    /// If it exits normally, do not restart.
    Transient,
    /// Never restart, no matter what.
    Temporary,
}

/// Defines how to spawn and manage a single child actor.
#[derive(Clone)]
pub struct ChildSpec {
    /// Unique child ID string that **must be provided**. This will be used as:
    /// 1. The actor's global registry name
    /// 2. Key for failure tracking
    /// 3. Child specification identifier
    ///
    /// # Important
    /// This ID must be unique within the supervisor's child list and will be
    /// used to register the actor in the global registry via [`ractor::registry`].
    pub id: String,

    /// Restart policy for this child. [`Restart::Permanent`], [`Restart::Transient`], [`Restart::Temporary`].
    pub restart: Restart,

    /// The user-defined spawn closure. If this fails, meltdown is triggered if repeated too often.
    pub spawn_fn: SpawnFn,

    /// A child-level backoff function. If set, this can delay re-spawning the child after a crash.
    pub backoff_fn: Option<ChildBackoffFn>,

    /// Optional child-level meltdown “reset.” If the child hasn’t failed in the `reset_after` duration,
    /// we reset *this child’s* failure count to 0 next time it fails.
    pub reset_after: Option<Duration>,
}

impl std::fmt::Debug for ChildSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildSpec")
            .field("id", &self.id)
            .field("restart", &self.restart)
            .field("reset_after", &self.reset_after)
            .finish()
    }
}

/// Internal tracking of a child’s failure count and the last time it failed.
#[derive(Debug, Clone)]
pub struct ChildFailureState {
    pub restart_count: usize,
    pub last_fail_instant: Instant,
}

/// Each time we restart a child, we store a record for meltdown counting: `(child_id, when)`.
#[derive(Clone, Debug)]
pub struct RestartLog {
    pub child_id: String,
    pub timestamp: Instant,
}

pub trait CoreSupervisorOptions<Strategy> {
    fn max_restarts(&self) -> usize;
    fn max_window(&self) -> Duration;
    fn reset_after(&self) -> Option<Duration>;
    fn strategy(&self) -> Strategy;
}

#[derive(Debug)]
pub enum ExitReason {
    Normal,
    Reason(Option<String>),
    Error(Box<dyn std::error::Error + Send + Sync>),
}

pub trait SupervisorCore {
    type Message: Message;
    type Strategy;
    type Options: CoreSupervisorOptions<Self::Strategy>;

    fn child_failure_state(&mut self) -> &mut HashMap<String, ChildFailureState>;
    fn restart_log(&mut self) -> &mut Vec<RestartLog>;
    fn options(&self) -> &Self::Options;
    fn restart_msg(
        &self,
        child_spec: &ChildSpec,
        strategy: Self::Strategy,
        myself: ActorRef<Self::Message>,
    ) -> Self::Message;

    /// Increments the failure count for a given child.  
    /// Resets the child’s `restart_count` to 0 if the time since last fail >= child's `reset_after` duration.
    fn prepare_child_failure(&mut self, child_spec: &ChildSpec) -> Result<(), ActorProcessingErr> {
        let child_id = &child_spec.id;
        let now = Instant::now();
        let entry = self
            .child_failure_state()
            .entry(child_id.clone())
            .or_insert_with(|| ChildFailureState {
                restart_count: 0,
                last_fail_instant: now,
            });

        if let Some(threshold) = child_spec.reset_after {
            if now.duration_since(entry.last_fail_instant) >= threshold {
                entry.restart_count = 0;
            }
        }

        entry.restart_count += 1;
        entry.last_fail_instant = now;
        Ok(())
    }

    /// Called when a child terminates or fails.  
    /// - If `abnormal == true`, we treat it like a panic or error exit.  
    /// - If the child’s [`Restart`] policy indicates a restart is needed, we do it.  
    ///
    /// Returns `Some(child_id)` if the supervisor should re-spawn the child, or `None` otherwise.
    fn handle_child_exit(
        &mut self,
        child_spec: &ChildSpec,
        abnormal: bool,
    ) -> Result<bool, ActorProcessingErr> {
        let policy = child_spec.restart;

        // Should we restart this child?
        let should_restart = match policy {
            Restart::Permanent => true,
            Restart::Transient => abnormal,
            Restart::Temporary => false,
        };

        if should_restart {
            self.prepare_child_failure(child_spec)?;
        }

        Ok(should_restart)
    }

    /// Called when a child exits abnormally or normally.
    /// - If the child should be restarted, we schedule a future spawn for it.
    /// - If the supervisor should meltdown, we return an error to end abnormally.
    fn handle_child_restart(
        &mut self,
        child_spec: &ChildSpec,
        abnormal: bool,
        myself: ActorRef<Self::Message>,
        reason: &ExitReason,
    ) -> Result<(), ActorProcessingErr> {
        if self.handle_child_exit(child_spec, abnormal)? {
            log_child_restart(child_spec, abnormal, reason);
            self.schedule_restart(child_spec, self.options().strategy(), myself.clone())?;
        }

        Ok(())
    }

    /// Updates meltdown log and checks meltdown thresholds.  
    ///
    /// - If `reset_after` is set and we’ve been quiet longer than that, we clear the meltdown log.  
    /// - We add a new entry and drop entries older than `max_window`.  
    /// - If `len(restart_log) > max_restarts`, meltdown is triggered.
    fn track_global_restart(&mut self, child_id: &str) -> Result<(), ActorProcessingErr> {
        let now: Instant = Instant::now();

        let max_restarts = self.options().max_restarts();
        let max_window = self.options().max_window();
        let reset_after = self.options().reset_after();

        let restart_log = self.restart_log();

        if let (Some(thresh), Some(latest)) = (reset_after, restart_log.last()) {
            if now.duration_since(latest.timestamp) >= thresh {
                restart_log.clear();
            }
        }

        restart_log.push(RestartLog {
            child_id: child_id.to_string(),
            timestamp: now,
        });

        restart_log.retain(|t| now.duration_since(t.timestamp) < max_window);

        if restart_log.len() > max_restarts {
            Err(SupervisorError::Meltdown {
                reason: "max_restarts exceeded".to_string(),
            }
            .into())
        } else {
            Ok(())
        }
    }

    /// Schedule a future spawn for a child, respecting any child-level `backoff_fn`.
    fn schedule_restart(
        &mut self,
        child_spec: &ChildSpec,
        strategy: Self::Strategy,
        myself: ActorRef<Self::Message>,
    ) -> Result<(), ActorProcessingErr> {
        let child_id = &child_spec.id;

        let (restart_count, last_fail_instant) = {
            let failure_state = self.child_failure_state();
            let st = failure_state
                .get(child_id)
                .ok_or(SupervisorError::ChildNotFound {
                    child_id: child_id.clone(),
                })?;
            (st.restart_count, st.last_fail_instant)
        };
        let msg = self.restart_msg(child_spec, strategy, myself.clone());

        let delay = child_spec
            .backoff_fn
            .as_ref()
            .and_then(|cb: &ChildBackoffFn| {
                cb.call(
                    child_id,
                    restart_count,
                    last_fail_instant,
                    child_spec.reset_after,
                )
            });

        match delay {
            Some(delay) => {
                myself.send_after(delay, move || msg);
            }
            None => {
                myself.send_message(msg)?;
            }
        }

        Ok(())
    }
}

fn log_child_restart(child_spec: &ChildSpec, abnormal: bool, reason: &ExitReason) {
    match (abnormal, reason) {
        (true, ExitReason::Error(err)) => log::error!(
            "Restarting child: {}, exit: abnormal, error: {:?}",
            child_spec.id,
            err
        ),
        (false, ExitReason::Error(err)) => log::warn!(
            "Restarting child: {}, exit: normal, error: {:?}",
            child_spec.id,
            err
        ),
        (true, ExitReason::Reason(Some(reason))) => log::error!(
            "Restarting child: {}, exit: abnormal, reason: {}",
            child_spec.id,
            reason
        ),
        (false, ExitReason::Reason(Some(reason))) => log::warn!(
            "Restarting child: {}, exit: normal, reason: {}",
            child_spec.id,
            reason
        ),
        (true, ExitReason::Reason(None)) => {
            log::error!("Restarting child: {}, exit: abnormal", child_spec.id)
        }
        (false, ExitReason::Reason(None)) => {
            log::warn!("Restarting child: {}, exit: normal", child_spec.id)
        }
        (true, ExitReason::Normal) => {
            log::error!("Restarting child: {}, exit: abnormal", child_spec.id)
        }
        (false, ExitReason::Normal) => {
            log::warn!("Restarting child: {}, exit: normal", child_spec.id)
        }
    }
}
