//! # ractor-supervisor
//!
//! An **OTP-style supervisor** for the [`ractor`](https://docs.rs/ractor) framework—helping you build **supervision trees** in a straightforward, Rust-centric way.
//!
//! Inspired by the Elixir/Erlang supervision concept, `ractor-supervisor` provides a robust mechanism for overseeing **one or more child actors** and automatically restarting them under configurable policies. If too many restarts happen in a brief time window—a “meltdown”—the supervisor itself shuts down abnormally, preventing errant restart loops.
//!
//! **Goal**: Make it easier to define, configure, and maintain supervision trees in your `ractor`-based applications. With multiple restart policies, flexible supervision strategies, custom backoff support, and meltdown counters, `ractor-supervisor` helps you keep your actor systems both **resilient** and **performant**.
//!
//! ## Overview
//!
//! ### Supervision Strategies
//! - **OneForOne**: Only the failing child is restarted.
//! - **OneForAll**: If any child fails, all children are stopped and restarted.
//! - **RestForOne**: The failing child and all subsequent children (as defined in order) are stopped and restarted.
//!
//! ### Restart Policies
//! - **Permanent**: Always restart, no matter how the child exited.
//! - **Transient**: Restart only if the child exited abnormally (panic or error).
//! - **Temporary**: Never restart, regardless of exit reason.
//!
//! ### Meltdown Logic
//! - **`max_restarts`** and **`max_seconds`**: The “time window” for meltdown counting. If more than `max_restarts` occur within `max_seconds`, the supervisor shuts down abnormally (meltdown).
//! - **`restart_counter_reset_after`**: If the supervisor sees no failures for this many seconds, it clears its meltdown log and effectively “resets” the meltdown counters.
//!
//! ### Child-Level Resets & Backoff
//! - **`restart_counter_reset_after`** (per child): If a specific child remains up for that many seconds, its own failure count is reset to zero on the next failure.
//! - **`backoff_fn`**: An optional function to delay a child’s restart. For instance, you might implement exponential backoff to prevent immediate thrashing restarts.
//!
//! ## Usage
//! 1. **Define** one or more child actors by implementing [`Actor`](ractor::Actor).
//! 2. For each child, create a [`ChildSpec`] with:
//!    - A [`Restart`](Restart) policy,
//!    - A `spawn_fn` that links the child to its supervisor,
//!    - Optional `backoff_fn` / meltdown resets.
//! 3. Configure [`SupervisorOptions`], specifying meltdown thresholds (`max_restarts`, `max_seconds`) and a supervision [`Strategy`].
//! 4. Pass those into [`SupervisorArguments`] and **spawn** your [`Supervisor`] via `Actor::spawn(...)`.
//!
//! You can also nest supervisors to build **multi-level supervision trees**—simply treat a supervisor as a “child” of another supervisor by specifying its own `ChildSpec`. This structure allows you to partition failure domains and maintain more complex actor systems in a structured, fault-tolerant manner.
//!
//! If meltdown conditions are reached, the supervisor stops itself abnormally to prevent runaway restart loops.
//!
//! ## Example
//! ```rust
//! use ractor::Actor;
//! use ractor_supervisor::*; // assuming your crate is named ractor_supervisor
//! use std::{time::Duration, sync::Arc};
//! use tokio::time::Instant;
//! use futures_util::FutureExt;
//!
//! // A minimal child actor that simply does some work in `handle`.
//! struct MyWorker;
//!
//! #[ractor::async_trait]
//! impl Actor for MyWorker {
//!     type Msg = ();
//!     type State = ();
//!     type Arguments = ();
//!
//!     // Called before the actor fully starts. We can set up the actor’s internal state here.
//!     async fn pre_start(
//!         &self,
//!         _myself: ractor::ActorRef<Self::Msg>,
//!         _args: Self::Arguments,
//!     ) -> Result<Self::State, ractor::ActorProcessingErr> {
//!         Ok(())
//!     }
//!
//!     // The main message handler. This is where you implement your actor’s behavior.
//!     async fn handle(
//!         &self,
//!         _myself: ractor::ActorRef<Self::Msg>,
//!         _msg: Self::Msg,
//!         _state: &mut Self::State
//!     ) -> Result<(), ractor::ActorProcessingErr> {
//!         // do some work...
//!         Ok(())
//!     }
//! }
//!
//! // A function to spawn the child actor. This will be used in ChildSpec::spawn_fn.
//! async fn spawn_my_worker(
//!     supervisor_cell: ractor::ActorCell,
//!     child_id: String
//! ) -> Result<ractor::ActorCell, ractor::SpawnErr> {
//!     // We name the child actor using `child_spec.id` (though naming is optional).
//!     let (child_ref, _join) = MyWorker::spawn_linked(
//!         Some(child_id), // actor name
//!         MyWorker,                    // actor instance
//!         (),                          // arguments
//!         supervisor_cell             // link to the supervisor
//!     ).await?;
//!     Ok(child_ref.get_cell())
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // A child-level backoff function that implements exponential backoff after the second failure.
//!     // Return Some(delay) to make the supervisor wait before restarting this child.
//!     let my_backoff: ChildBackoffFn = Arc::new(
//!         |_child_id: &str, restart_count: usize, last_fail: Instant, child_reset_after: Option<u64>| {
//!             // On the first failure, restart immediately (None).
//!             // After the second failure, double the delay each time (exponential).
//!             if restart_count <= 1 {
//!                 None
//!             } else {
//!                 Some(Duration::from_secs(1 << restart_count))
//!             }
//!         }
//!     );
//!
//!     // This specification describes exactly how to manage our single child actor.
//!     let child_spec = ChildSpec {
//!         id: "myworker".into(),  // Unique identifier for meltdown logs and debugging.
//!         restart: Restart::Transient, // Only restart if the child fails abnormally.
//!         spawn_fn: Box::new(|cell, id| spawn_my_worker(cell, id).boxed()),
//!         backoff_fn: Some(my_backoff), // Apply our custom exponential backoff on restarts.
//!         // If the child remains up for 60s, its individual failure counter resets to 0 next time it fails.
//!         restart_counter_reset_after: Some(60),
//!     };
//!
//!     // Supervisor-level meltdown configuration. If more than 5 restarts occur within 10s, meltdown is triggered.
//!     // Also, if we stay quiet for 30s (no restarts), the meltdown log resets.
//!     let options = SupervisorOptions {
//!         strategy: Strategy::OneForOne,  // If one child fails, only that child is restarted.
//!         max_restarts: 5,               // Permit up to 5 restarts in the meltdown window.
//!         max_seconds: 10,               // The meltdown window (in seconds).
//!         restart_counter_reset_after: Some(30), // If no failures for 30s, meltdown log is cleared.
//!     };
//!
//!     // Group all child specs and meltdown options together:
//!     let args = SupervisorArguments {
//!         child_specs: vec![child_spec], // We only have one child in this example
//!         options,
//!     };
//!
//!     // Spawn the supervisor with our arguments.
//!     let (sup_ref, sup_handle) = Actor::spawn(
//!         None,        // no name for the supervisor
//!         Supervisor,  // the Supervisor actor
//!         args
//!     ).await?;
//!
//!     let _ = sup_ref.kill();
//!     let _ = sup_handle.await;
//!
//!     Ok(())
//! }
//! ```
use if_chain::if_chain;
use ractor::concurrency::sleep;
use ractor::{
    Actor, ActorCell, ActorProcessingErr, ActorRef, RpcReplyPort, SpawnErr, SupervisionEvent,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::Instant;

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

/// The supervision strategy for this supervisor’s children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Only the failing child is restarted.
    OneForOne,
    /// If *any* child fails, all children are stopped and restarted.
    OneForAll,
    /// If one child fails, that child and all subsequently started children are stopped and restarted.
    RestForOne,
}

/// Supervisor-level meltdown policy.
///
/// - If more than `max_restarts` occur within `max_seconds`, meltdown occurs (supervisor stops abnormally).
/// - If `restart_counter_reset_after` is set, we clear the meltdown log if no restarts occur in that span.
pub struct SupervisorOptions {
    /// One of OneForOne, OneForAll, or RestForOne
    pub strategy: Strategy,
    /// The meltdown threshold for restarts.
    pub max_restarts: usize,
    /// The meltdown time window in seconds.
    pub max_seconds: usize,
    /// Optional: if no restarts for this many seconds, we clear meltdown log.
    pub restart_counter_reset_after: Option<u64>,
}

/// A function pointer for computing **child-level** backoff delays before re-spawning a child.
///
/// This function is invoked each time a child fails:
/// ```ignore
/// (child_id, current_restart_count, last_fail_instant, child_reset_after)
///    -> Option<Duration>
/// ```
/// If you return `Some(duration)`, the supervisor will wait that amount of time before actually re-spawning the child.
/// If `None`, it restarts immediately.
pub type ChildBackoffFn =
    Arc<dyn Fn(&str, usize, Instant, Option<u64>) -> Option<Duration> + Send + Sync>;

/// The future returned by a [`SpawnFn`].
pub type SpawnFuture = Pin<Box<dyn Future<Output = Result<ActorCell, SpawnErr>> + Send>>;

/// User-provided closure to spawn a child. You typically call `Actor::spawn_linked` here.
pub type SpawnFn = Box<dyn Fn(ActorCell, String) -> SpawnFuture + Send + Sync>;

/// Defines how to spawn and manage a single child actor.
pub struct ChildSpec {
    /// Unique child ID string (for logging, meltdown log, etc.).
    pub id: String,

    /// Restart policy for this child. [`Restart::Permanent`], [`Restart::Transient`], [`Restart::Temporary`].
    pub restart: Restart,

    /// The user-defined spawn closure. If this fails, meltdown is triggered if repeated too often.
    pub spawn_fn: SpawnFn,

    /// A child-level backoff function. If set, this can delay re-spawning the child after a crash.
    pub backoff_fn: Option<ChildBackoffFn>,

    /// Optional child-level meltdown “reset.” If the child hasn’t failed in `restart_counter_reset_after` seconds,
    /// we reset *this child’s* failure count to 0 next time it fails.  
    ///
    /// This is **separate** from the supervisor-level meltdown logic in [`SupervisorOptions`].
    pub restart_counter_reset_after: Option<u64>,
}

/// Internal tracking of a child’s failure count and the last time it failed.
#[derive(Debug, Clone)]
pub struct ChildFailureState {
    pub restart_count: usize,
    pub last_fail_instant: Instant,
}

/// Internal messages that instruct the supervisor to spawn a child, triggered by its meltdown logic.
pub enum SupervisorMsg {
    /// (OneForOne) Re-spawn just this child
    OneForOneSpawn { child_id: String },
    /// (OneForAll) Stop all children, re-spawn them
    OneForAllSpawn { child_id: String },
    /// (RestForOne) Stop this child and all subsequent children, re-spawn them
    RestForOneSpawn { child_id: String },
    /// Return the current state snapshot (for debugging/tests).
    InspectState(RpcReplyPort<InspectableState>),
}

/// The arguments needed to spawn the supervisor.
pub struct SupervisorArguments {
    /// The list of children to supervise.
    pub child_specs: Vec<ChildSpec>,
    /// Supervisor meltdown config + strategy.
    pub options: SupervisorOptions,
}

/// Possible errors from the supervisor’s logic.
#[derive(Error, Debug)]
pub enum SupervisorError {
    #[error("Child '{child_id}' not found in specs")]
    ChildNotFound { child_id: String },

    #[error("Meltdown: too many restarts => meltdown")]
    Meltdown,

    #[error("Spawn error '{child_id}': {source}")]
    SpawnError { child_id: String, source: SpawnErr },
}

/// Each time we restart a child, we store a record for meltdown counting: `(child_id, when)`.
#[derive(Clone)]
pub struct RestartLog {
    pub child_id: String,
    pub timestamp: Instant,
}

/// Holds the supervisor’s live state: which children are running, how many times each child has failed, etc.
pub struct SupervisorState {
    /// The original child specs (each child’s config).
    pub child_specs: Vec<ChildSpec>,

    /// The currently running children: `child_id -> ActorCell`.
    pub running: HashMap<String, ActorCell>,

    /// Tracks how many times each child has failed and the last time it failed.
    pub child_failure_state: HashMap<String, ChildFailureState>,

    /// Rolling log of all restarts in the meltdown window.
    pub restart_log: Vec<RestartLog>,

    /// Supervisor meltdown options.
    pub options: SupervisorOptions,
}

/// A snapshot of the supervisor’s state, used mainly for testing or debugging.  
/// Contains copies of the supervisor’s “running” children, child failure counters, and meltdown log.
#[derive(Clone)]
pub struct InspectableState {
    pub running: HashMap<String, ActorCell>,
    pub child_failure_state: HashMap<String, ChildFailureState>,
    pub restart_log: Vec<RestartLog>,
}

impl SupervisorState {
    /// Create a new [`SupervisorState`], from user-supplied [`SupervisorArguments`].
    fn new(args: SupervisorArguments) -> Self {
        Self {
            child_specs: args.child_specs,
            running: HashMap::new(),
            child_failure_state: HashMap::new(),
            restart_log: Vec::new(),
            options: args.options,
        }
    }

    /// Spawn all children in the order they were defined in [`SupervisorArguments::child_specs`].
    pub async fn spawn_all_children(
        &mut self,
        supervisor_cell: ActorCell,
    ) -> Result<(), ActorProcessingErr> {
        for spec in &self.child_specs {
            let child = (spec.spawn_fn)(supervisor_cell.clone(), spec.id.clone())
                .await
                .map_err(|e| SupervisorError::SpawnError {
                    child_id: spec.id.clone(),
                    source: e,
                })?;
            self.running.insert(spec.id.clone(), child);
            // Initialize child_failure_state for each child
            self.child_failure_state
                .entry(spec.id.clone())
                .or_insert_with(|| ChildFailureState {
                    restart_count: 0,
                    last_fail_instant: Instant::now(),
                });
        }
        Ok(())
    }

    /// Increments the failure count for a given child.  
    /// Resets the child’s `restart_count` to 0 if the time since last fail >= child’s `restart_counter_reset_after`.
    pub fn prepare_child_failure(&mut self, child_id: &str) -> Result<(), ActorProcessingErr> {
        let now = Instant::now();
        let entry = self
            .child_failure_state
            .entry(child_id.to_string())
            .or_insert_with(|| ChildFailureState {
                restart_count: 0,
                last_fail_instant: now,
            });

        let child_reset_after = self
            .child_specs
            .iter()
            .find(|s| s.id == child_id)
            .ok_or(SupervisorError::ChildNotFound {
                child_id: child_id.to_string(),
            })?
            .restart_counter_reset_after;

        if let Some(threshold_secs) = child_reset_after {
            let elapsed = now.duration_since(entry.last_fail_instant).as_secs();
            if elapsed >= threshold_secs {
                // If child was quiet long enough, reset its restart count.
                entry.restart_count = 0;
            }
        }

        entry.restart_count += 1;
        entry.last_fail_instant = now;

        Ok(())
    }

    /// Return the child ID for a given [ActorCell], if it’s in the `running` map.
    pub fn get_child_id_by_cell_id(&self, cell_id: ractor::ActorId) -> Option<String> {
        self.running.iter().find_map(|(id, c)| {
            if c.get_id() == cell_id {
                Some(id.clone())
            } else {
                None
            }
        })
    }

    /// Called when a child terminates or fails.  
    /// - If `abnormal == true`, we treat it like a panic or error exit.  
    /// - We remove the child from `running`.  
    /// - If the child’s [`Restart`](Restart) policy indicates a restart is needed, we do it.  
    ///
    /// Returns `Some(child_id)` if the supervisor should re-spawn the child, or `None` otherwise.
    pub fn handle_child_exit(
        &mut self,
        child_id: &str,
        abnormal: bool,
    ) -> Result<Option<String>, ActorProcessingErr> {
        self.running.remove(child_id);

        let spec = self.child_specs.iter().find(|s| s.id == child_id).ok_or(
            SupervisorError::ChildNotFound {
                child_id: child_id.to_string(),
            },
        )?;
        let policy = spec.restart;

        // Should we restart this child?
        let should_restart = match policy {
            Restart::Permanent => true,
            Restart::Transient => abnormal,
            Restart::Temporary => false,
        };

        if should_restart {
            self.prepare_child_failure(child_id)?;
            Ok(Some(child_id.to_owned()))
        } else {
            // Child does not restart
            Ok(None)
        }
    }

    /// Schedule a future spawn for a child, respecting any child-level `backoff_fn`.
    pub fn schedule_restart(
        &mut self,
        child_id: &String,
        strategy: Strategy,
        myself: ActorRef<SupervisorMsg>,
    ) -> Result<(), ActorProcessingErr> {
        let st = &self.child_failure_state[child_id];
        let spec = self.child_specs.iter().find(|s| s.id == *child_id).ok_or(
            SupervisorError::ChildNotFound {
                child_id: child_id.clone(),
            },
        )?;

        let spawn_msg = match strategy {
            Strategy::OneForOne => SupervisorMsg::OneForOneSpawn {
                child_id: child_id.clone(),
            },
            Strategy::OneForAll => SupervisorMsg::OneForAllSpawn {
                child_id: child_id.clone(),
            },
            Strategy::RestForOne => SupervisorMsg::RestForOneSpawn {
                child_id: child_id.clone(),
            },
        };

        // If the child has a backoff function, compute the optional delay
        let maybe_delay = spec.backoff_fn.as_ref().and_then(|cb| {
            (cb)(
                child_id,
                st.restart_count,
                st.last_fail_instant,
                spec.restart_counter_reset_after,
            )
        });

        // Either schedule a delayed message or send it immediately
        match maybe_delay {
            Some(delay) => {
                myself.send_after(delay, move || spawn_msg);
            }
            None => {
                let _ = myself.send_message(spawn_msg);
            }
        }

        Ok(())
    }

    /// Updates meltdown log and checks meltdown thresholds.  
    ///
    /// - If `restart_counter_reset_after` is set and we’ve been quiet longer than that, we clear the meltdown log.  
    /// - We add a new entry and drop entries older than `max_seconds`.  
    /// - If `len(restart_log) > max_restarts`, meltdown is triggered.
    fn track_global_restart(&mut self, restarted_child_id: &str) -> Result<(), ActorProcessingErr> {
        let now = Instant::now();

        // Possibly clear meltdown log if we’ve been quiet for too long
        if_chain! {
            if let Some(thresh) = self.options.restart_counter_reset_after;
            if let Some(latest) = self.restart_log.last();
            if now.duration_since(latest.timestamp).as_secs() >= thresh;
            then {
                self.restart_log.clear();
            }
        }

        // Add a new event
        self.restart_log.push(RestartLog {
            child_id: restarted_child_id.to_string(),
            timestamp: now,
        });

        // Drop events older than `max_seconds` from meltdown log
        let cutoff = self.options.max_seconds as u64;
        self.restart_log
            .retain(|t| now.duration_since(t.timestamp).as_secs() < cutoff);

        // meltdown if we exceed `max_restarts`
        if self.restart_log.len() > self.options.max_restarts {
            return Err(SupervisorError::Meltdown.into());
        }
        Ok(())
    }

    /// OneForOne: meltdown-check first, then spawn just the failing child.
    pub async fn perform_one_for_one_spawn(
        &mut self,
        child_id: &str,
        supervisor_cell: ActorCell,
    ) -> Result<(), ActorProcessingErr> {
        self.track_global_restart(child_id)?;
        if let Some(spec) = self.child_specs.iter().find(|s| s.id == child_id) {
            let new_child = (spec.spawn_fn)(supervisor_cell, spec.id.clone())
                .await
                .map_err(|e| SupervisorError::SpawnError {
                    child_id: spec.id.clone(),
                    source: e,
                })?;
            self.running.insert(child_id.to_string(), new_child);
        }
        Ok(())
    }

    /// OneForAll: meltdown-check first, then stop all children, re-spawn them all.
    pub async fn perform_one_for_all_spawn(
        &mut self,
        child_id: &str,
        myself: ActorRef<SupervisorMsg>,
        supervisor_cell: ActorCell,
    ) -> Result<(), ActorProcessingErr> {
        self.track_global_restart(child_id)?;

        // Kill all children. Must unlink to prevent confusion with them receiving further messages.
        for (_, cell) in self.running.drain() {
            cell.unlink(myself.get_cell());
            cell.kill();
        }

        // A short delay to allow the old children to fully unregister (avoid name collisions).
        sleep(Duration::from_millis(50)).await;

        self.spawn_all_children(supervisor_cell).await?;
        Ok(())
    }

    /// RestForOne: meltdown-check first, then stop the failing child and all subsequent children, re-spawn them.
    pub async fn perform_rest_for_one_spawn(
        &mut self,
        child_id: &str,
        myself: ActorRef<SupervisorMsg>,
        supervisor_cell: ActorCell,
    ) -> Result<(), ActorProcessingErr> {
        self.track_global_restart(child_id)?;

        if let Some(i) = self.child_specs.iter().position(|s| s.id == child_id) {
            // Kill children from i..end
            for spec in self.child_specs.iter().skip(i) {
                if let Some(cell) = self.running.remove(&spec.id) {
                    cell.unlink(myself.get_cell());
                    cell.kill();
                }
            }

            // Short delay so old names get unregistered
            sleep(Duration::from_millis(50)).await;

            // Re-spawn children from i..end
            for spec in self.child_specs.iter().skip(i) {
                let new_child = (spec.spawn_fn)(supervisor_cell.clone(), spec.id.clone())
                    .await
                    .map_err(|e| SupervisorError::SpawnError {
                        child_id: spec.id.clone(),
                        source: e,
                    })?;
                self.running.insert(spec.id.clone(), new_child);
            }
        }
        Ok(())
    }
}

/// The supervisor actor itself.  
/// Spawns its children in `post_start`, listens for child failures, and restarts them if needed.  
/// If meltdown occurs, it returns an error to end abnormally (thus skipping `post_stop`).
pub struct Supervisor;

/// A global map for test usage, storing final states after each handle call and in `post_stop`.
#[cfg(test)]
static SUPERVISOR_FINAL: std::sync::OnceLock<std::sync::Mutex<HashMap<String, InspectableState>>> =
    std::sync::OnceLock::new();

#[ractor::async_trait]
impl Actor for Supervisor {
    type Msg = SupervisorMsg;
    type State = SupervisorState;
    type Arguments = SupervisorArguments;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(SupervisorState::new(args))
    }

    async fn post_start(
        &self,
        myself: ActorRef<Self::Msg>,
        state: &mut SupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        let supervisor_cell = myself.get_cell();
        // Spawn all children initially
        state.spawn_all_children(supervisor_cell).await?;
        Ok(())
    }

    /// The main message handler: we respond to “spawn child X” or “inspect state”.
    /// Each time we finish, we store final state in a global map (test usage only).
    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        msg: SupervisorMsg,
        state: &mut SupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        let supervisor_cell = myself.get_cell();
        let result = match msg {
            SupervisorMsg::OneForOneSpawn { child_id } => {
                state
                    .perform_one_for_one_spawn(&child_id, supervisor_cell)
                    .await
            }
            SupervisorMsg::OneForAllSpawn { child_id } => {
                state
                    .perform_one_for_all_spawn(&child_id, myself.clone(), supervisor_cell)
                    .await
            }
            SupervisorMsg::RestForOneSpawn { child_id } => {
                state
                    .perform_rest_for_one_spawn(&child_id, myself.clone(), supervisor_cell)
                    .await
            }
            SupervisorMsg::InspectState(rpc_reply_port) => {
                let snap = InspectableState {
                    running: state.running.clone(),
                    child_failure_state: state.child_failure_state.clone(),
                    restart_log: state.restart_log.clone(),
                };
                rpc_reply_port.send(snap)?;
                Ok(())
            }
        };

        #[cfg(test)]
        store_final_state(myself, state);

        // Return any meltdown or spawn error
        result
    }

    /// Respond to supervision events from child actors.
    /// - `ActorTerminated` => treat as normal exit
    /// - `ActorFailed` => treat as abnormal
    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<Self::Msg>,
        evt: SupervisionEvent,
        state: &mut SupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        match evt {
            SupervisionEvent::ActorStarted(_child_ref) => { /* ignore for now */ }
            SupervisionEvent::ActorTerminated(cell, _final_state, _reason) => {
                // Normal exit => abnormal=false
                if_chain! {
                    if let Some(child_id) = state.get_child_id_by_cell_id(cell.get_id());
                    if let Some(restart_id) = state.handle_child_exit(&child_id, false)?;
                    then {
                        state.schedule_restart(&restart_id, state.options.strategy, myself.clone())?;
                    }
                }
            }
            SupervisionEvent::ProcessGroupChanged(_group) => {}
            SupervisionEvent::ActorFailed(cell, _reason) => {
                // Abnormal exit => abnormal=true
                if_chain! {
                    if let Some(child_id) = state.get_child_id_by_cell_id(cell.get_id());
                    if let Some(restart_id) = state.handle_child_exit(&child_id, true)?;
                    then {
                        state.schedule_restart(&restart_id, state.options.strategy, myself.clone())?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Called if the supervisor stops normally (e.g. `.stop(None)`).
    /// For meltdown stops, we skip this, but we still store final state for testing.
    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut SupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        #[cfg(test)]
        store_final_state(_myself, _state);
        Ok(())
    }
}

#[cfg(test)]
fn store_final_state(myself: ActorRef<SupervisorMsg>, state: &SupervisorState) {
    let mut map = SUPERVISOR_FINAL
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    if let Some(name) = myself.get_name() {
        map.insert(
            name,
            InspectableState {
                running: state.running.clone(),
                child_failure_state: state.child_failure_state.clone(),
                restart_log: state.restart_log.clone(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use ractor::{call_t, Actor, ActorCell, ActorRef, ActorStatus};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::time::{sleep, Duration, Instant};

    #[cfg(test)]
    static ACTOR_CALL_COUNT: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, u64>>,
    > = std::sync::OnceLock::new();

    fn before_each() {
        // Clear the final supervisor state map (test usage)
        match super::SUPERVISOR_FINAL.get() {
            None => {}
            Some(map) => {
                let mut map = map.lock().unwrap();
                map.clear();
            }
        }
        // Clear our actor call counts
        match ACTOR_CALL_COUNT.get() {
            None => {}
            Some(map) => {
                let mut map = map.lock().unwrap();
                map.clear();
            }
        }
    }

    fn increment_actor_count(child_id: &str) {
        let mut map = ACTOR_CALL_COUNT
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .unwrap();
        *map.entry(child_id.to_string()).or_default() += 1;
    }

    /// Utility to read the final state of a named supervisor after it stops.
    fn read_final_supervisor_state(sup_name: &str) -> InspectableState {
        let map = super::SUPERVISOR_FINAL
            .get()
            .expect("SUPERVISOR_FINAL not initialized!")
            .lock()
            .unwrap();

        map.get(sup_name)
            .cloned()
            .unwrap_or_else(|| panic!("No final state for supervisor '{sup_name}'"))
    }

    fn read_actor_call_count(child_id: &str) -> u64 {
        let map = ACTOR_CALL_COUNT
            .get()
            .expect("ACTOR_CALL_COUNT not initialized!")
            .lock()
            .unwrap();

        *map.get(child_id)
            .unwrap_or_else(|| panic!("No actor call count for child '{child_id}'"))
    }

    // Child behaviors for tests
    #[derive(Clone)]
    pub enum ChildBehavior {
        DelayedFail {
            ms: u64,
        },
        DelayedNormal {
            ms: u64,
        },
        ImmediateFail,
        ImmediateNormal,
        CountedFails {
            delay_ms: u64,
            fail_count: u64,
            current: Arc<AtomicU64>,
        },
        FailWaitFail {
            initial_fails: u64,
            wait_ms: u64,
            final_fails: u64,
            current: Arc<AtomicU64>,
        },
    }

    pub struct TestChild;

    #[ractor::async_trait]
    impl Actor for TestChild {
        type Msg = ();
        type State = ChildBehavior;
        type Arguments = ChildBehavior;

        async fn pre_start(
            &self,
            myself: ActorRef<Self::Msg>,
            arg: Self::Arguments,
        ) -> Result<Self::State, ractor::ActorProcessingErr> {
            // Track how many times this particular child-id was started
            increment_actor_count(myself.get_name().unwrap().as_str());
            Ok(arg)
        }

        async fn post_start(
            &self,
            myself: ActorRef<Self::Msg>,
            state: &mut Self::State,
        ) -> Result<(), ractor::ActorProcessingErr> {
            match state {
                ChildBehavior::DelayedFail { ms } => {
                    myself.send_after(Duration::from_millis(*ms), || ());
                }
                ChildBehavior::DelayedNormal { ms } => {
                    myself.send_after(Duration::from_millis(*ms), || ());
                }
                ChildBehavior::ImmediateFail => {
                    panic!("Immediate fail => ActorFailed");
                }
                ChildBehavior::ImmediateNormal => {
                    myself.stop(None);
                }
                ChildBehavior::CountedFails { delay_ms, .. } => {
                    myself.send_after(Duration::from_millis(*delay_ms), || ());
                }
                ChildBehavior::FailWaitFail { .. } => {
                    // Kick off our chain of fails by sending a first message
                    myself.cast(())?;
                }
            }
            Ok(())
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            _msg: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ractor::ActorProcessingErr> {
            match state {
                ChildBehavior::DelayedFail { .. } => {
                    panic!("Delayed fail => ActorFailed");
                }
                ChildBehavior::DelayedNormal { .. } => {
                    myself.stop(None);
                }
                ChildBehavior::ImmediateFail => {
                    panic!("ImmediateFail => ActorFailed");
                }
                ChildBehavior::ImmediateNormal => {
                    myself.stop(None);
                }
                ChildBehavior::CountedFails {
                    fail_count,
                    current,
                    ..
                } => {
                    let old = current.fetch_add(1, Ordering::SeqCst);
                    let newv = old + 1;
                    if newv <= *fail_count {
                        panic!("CountedFails => fail #{newv}");
                    }
                }
                ChildBehavior::FailWaitFail {
                    initial_fails,
                    wait_ms,
                    final_fails,
                    current,
                } => {
                    let so_far = current.fetch_add(1, Ordering::SeqCst) + 1;
                    if so_far <= *initial_fails {
                        panic!("FailWaitFail => initial fail #{so_far}");
                    } else if so_far == *initial_fails + 1 {
                        // wait some ms => schedule next message => final fails
                        myself.send_after(Duration::from_millis(*wait_ms), || ());
                    } else {
                        let n = so_far - (*initial_fails + 1);
                        if n <= *final_fails {
                            panic!("FailWaitFail => final fail #{n}");
                        }
                    }
                }
            }
            Ok(())
        }
    }

    fn get_running_children(sup_ref: &ActorRef<SupervisorMsg>) -> Vec<ActorCell> {
        sup_ref
            .get_children()
            .into_iter()
            .filter(|c| c.get_status() == ActorStatus::Running)
            .collect::<Vec<_>>()
    }

    // Helper for spawning our test child
    async fn spawn_test_child(
        sup_cell: ActorCell,
        id: String,
        behavior: ChildBehavior,
    ) -> Result<ActorCell, SpawnErr> {
        let (ch_ref, _join) = Actor::spawn_linked(Some(id), TestChild, behavior, sup_cell).await?;
        Ok(ch_ref.get_cell())
    }

    // Reusable child spec
    fn make_child_spec(id: &str, restart: Restart, behavior: ChildBehavior) -> ChildSpec {
        ChildSpec {
            id: id.to_string(),
            restart,
            spawn_fn: Box::new(move |sup_cell, child_id| {
                spawn_test_child(sup_cell, child_id, behavior.clone()).boxed()
            }),
            backoff_fn: None, // by default no per-child backoff
            restart_counter_reset_after: None,
        }
    }

    #[ractor::concurrency::test]
    async fn test_permanent_delayed_fail() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on the 2nd fail => max_restarts=1
        let child_spec = make_child_spec(
            "fail-delay",
            Restart::Permanent,
            ChildBehavior::DelayedFail { ms: 200 },
        );
        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 1, // meltdown on second fail
            max_seconds: 2,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let (sup_ref, sup_handle) =
            Actor::spawn(Some("test_permanent_delayed_fail".into()), Supervisor, args).await?;

        sleep(Duration::from_millis(100)).await;
        let st = call_t!(sup_ref, SupervisorMsg::InspectState, 500).unwrap();
        assert_eq!(st.running.len(), 1);
        assert_eq!(st.restart_log.len(), 0);

        // meltdown on second fail => wait for supervisor to stop
        let _ = sup_handle.await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        // final checks
        let final_st = read_final_supervisor_state("test_permanent_delayed_fail");
        assert_eq!(final_st.running.len(), 0);
        assert!(final_st.restart_log.len() >= 2);

        // We had exactly 2 spawns: one initial, one after 1st fail => meltdown on 2nd fail
        assert_eq!(read_actor_call_count("fail-delay"), 2);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_transient_delayed_normal() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // child does a delayed normal exit => no restarts
        let child_spec = make_child_spec(
            "normal-delay",
            Restart::Transient,
            ChildBehavior::DelayedNormal { ms: 300 },
        );
        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 5,
            max_seconds: 5,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_transient_delayed_normal".into()),
            Supervisor,
            args,
        )
        .await?;

        sleep(Duration::from_millis(150)).await;
        let st1 = call_t!(sup_ref, SupervisorMsg::InspectState, 500).unwrap();
        assert_eq!(st1.running.len(), 1);
        assert_eq!(st1.restart_log.len(), 0);

        // child exits normally => no meltdown => we stop
        sleep(Duration::from_millis(300)).await;
        sup_ref.stop(None);
        let _ = sup_handle.await;

        let final_state = read_final_supervisor_state("test_transient_delayed_normal");
        assert!(!final_state.running.contains_key("normal-delay"));
        assert_eq!(final_state.restart_log.len(), 0);

        // Only ever spawned once
        assert_eq!(read_actor_call_count("normal-delay"), 1);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_temporary_delayed_fail() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // Temporary => never restart even if fails
        let child_spec = make_child_spec(
            "temp-delay",
            Restart::Temporary,
            ChildBehavior::DelayedFail { ms: 200 },
        );
        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 10,
            max_seconds: 10,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let (sup_ref, sup_handle) =
            Actor::spawn(Some("test_temporary_delayed_fail".into()), Supervisor, args).await?;

        sleep(Duration::from_millis(100)).await;
        let st1 = call_t!(sup_ref, SupervisorMsg::InspectState, 500).unwrap();
        assert_eq!(st1.running.len(), 1);
        assert_eq!(st1.restart_log.len(), 0);

        // The child fails, but policy=Temporary => no restart
        sleep(Duration::from_millis(300)).await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Running);

        sup_ref.stop(None);
        let _ = sup_handle.await;

        let final_state = read_final_supervisor_state("test_temporary_delayed_fail");
        assert_eq!(final_state.running.len(), 0);
        assert_eq!(final_state.restart_log.len(), 0);

        // Only ever spawned once
        assert_eq!(read_actor_call_count("temp-delay"), 1);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_one_for_all_stop_all_on_failure() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on the 3rd fail => set max_restarts=2 => meltdown at fail #3
        let child1 = make_child_spec(
            "ofa-fail",
            Restart::Permanent,
            ChildBehavior::DelayedFail { ms: 200 },
        );
        let child2 = make_child_spec(
            "ofa-normal",
            Restart::Permanent,
            ChildBehavior::DelayedNormal { ms: 9999 },
        );

        let options = SupervisorOptions {
            strategy: Strategy::OneForAll,
            max_restarts: 2,
            max_seconds: 2,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child1, child2],
            options,
        };
        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_one_for_all_stop_all_on_failure".into()),
            Supervisor,
            args,
        )
        .await?;

        sleep(Duration::from_millis(100)).await;
        let running_children = get_running_children(&sup_ref);
        assert_eq!(running_children.len(), 2);

        let _ = sup_handle.await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        let final_state = read_final_supervisor_state("test_one_for_all_stop_all_on_failure");
        assert_eq!(sup_ref.get_children().len(), 0);
        assert_eq!(final_state.restart_log.len(), 3);

        // Because each time "ofa-fail" fails, OneForAll restarts *all* children:
        // meltdown occurs on the 3rd fail => that means "ofa-fail" was spawned 3 times
        assert_eq!(read_actor_call_count("ofa-fail"), 3);
        // "ofa-normal" also restarts each time, so also spawned 3 times
        assert_eq!(read_actor_call_count("ofa-normal"), 3);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_rest_for_one_restart_subset() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on 2nd fail => max_restarts=1 => meltdown at fail #2
        let child_a = make_child_spec(
            "A",
            Restart::Permanent,
            ChildBehavior::DelayedNormal { ms: 9999 },
        );
        let child_b = make_child_spec(
            "B",
            Restart::Permanent,
            ChildBehavior::DelayedFail { ms: 200 },
        );
        let child_c = make_child_spec(
            "C",
            Restart::Permanent,
            ChildBehavior::DelayedNormal { ms: 9999 },
        );

        let options = SupervisorOptions {
            strategy: Strategy::RestForOne,
            max_restarts: 1,
            max_seconds: 2,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_a, child_b, child_c],
            options,
        };
        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_rest_for_one_restart_subset".into()),
            Supervisor,
            args,
        )
        .await?;

        sleep(Duration::from_millis(100)).await;
        let running_children = get_running_children(&sup_ref);
        assert_eq!(running_children.len(), 3);

        let _ = sup_handle.await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        let final_state = read_final_supervisor_state("test_rest_for_one_restart_subset");
        assert_eq!(sup_ref.get_children().len(), 0);
        assert_eq!(final_state.restart_log.len(), 2);
        assert_eq!(final_state.restart_log[0].child_id, "B");
        assert_eq!(final_state.restart_log[1].child_id, "B");

        // "B" fails => triggers a restart for B (and everything after B: child C)
        // meltdown on 2nd fail => total spawns for B = 2, C = 2, A stays up from the start => 1
        assert_eq!(read_actor_call_count("A"), 1);
        assert_eq!(read_actor_call_count("B"), 2);
        assert_eq!(read_actor_call_count("C"), 2);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_max_restarts_in_time_window() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on 3 fails in <1s => max_restarts=2 => meltdown on fail #3
        let child_spec =
            make_child_spec("fastfail", Restart::Permanent, ChildBehavior::ImmediateFail);

        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 2,
            max_seconds: 1,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_max_restarts_in_time_window".into()),
            Supervisor,
            args,
        )
        .await?;

        let _ = sup_handle.await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        let final_state = read_final_supervisor_state("test_max_restarts_in_time_window");
        assert_eq!(
            final_state.restart_log.len(),
            3,
            "3 fails in <1s => meltdown"
        );

        // 3 fails => total spawns is 3
        assert_eq!(read_actor_call_count("fastfail"), 3);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_transient_abnormal_exit() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on 1st fail => max_restarts=0
        let child_spec = make_child_spec(
            "transient-bad",
            Restart::Transient,
            ChildBehavior::ImmediateFail,
        );

        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 0, // meltdown on the very first fail
            max_seconds: 5,
            restart_counter_reset_after: None,
        };

        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };
        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_transient_abnormal_exit".into()),
            Supervisor,
            args,
        )
        .await?;

        let _ = sup_handle.await;
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        let final_state = read_final_supervisor_state("test_transient_abnormal_exit");
        assert_eq!(
            final_state.restart_log.len(),
            1,
            "1 fail => meltdown with max_restarts=0"
        );

        // Only ever spawned once; meltdown immediately
        assert_eq!(read_actor_call_count("transient-bad"), 1);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_backoff_fn_delays_restart() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // meltdown on 2nd fail => set max_restarts=1 => meltdown on fail #2
        // but we do a 2s child-level backoff for the second spawn
        let child_backoff: ChildBackoffFn = Arc::new(|_id, count, _last, _child_reset| {
            if count <= 1 {
                None
            } else {
                Some(Duration::from_secs(2))
            }
        });

        let mut child_spec =
            make_child_spec("backoff", Restart::Permanent, ChildBehavior::ImmediateFail);
        child_spec.backoff_fn = Some(child_backoff);

        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 1, // meltdown on 2nd fail
            max_seconds: 10,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let before = Instant::now();
        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_backoff_fn_delays_restart".into()),
            Supervisor,
            args,
        )
        .await?;
        let _ = sup_handle.await;

        let elapsed = before.elapsed();
        assert!(
            elapsed >= Duration::from_secs(2),
            "2s delay on second fail due to child-level backoff"
        );
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);

        let final_st = read_final_supervisor_state("test_backoff_fn_delays_restart");
        assert_eq!(
            final_st.restart_log.len(),
            2,
            "first fail => immediate restart => second fail => meltdown"
        );

        // Exactly 2 spawns
        assert_eq!(read_actor_call_count("backoff"), 2);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_restart_counter_reset_after() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        // Child fails 2 times quickly => meltdown log=2
        // Wait 3s => meltdown log is cleared => final fail => meltdown log=1 => no meltdown
        // => total 3 fails => i.e. the child is started 4 times
        let behavior = ChildBehavior::FailWaitFail {
            initial_fails: 2,
            wait_ms: 3000,
            final_fails: 1,
            current: Arc::new(AtomicU64::new(0)),
        };

        let child_spec = ChildSpec {
            id: "reset-test".to_string(),
            restart: Restart::Permanent,
            spawn_fn: Box::new(move |sup_cell, id| {
                spawn_test_child(sup_cell, id, behavior.clone()).boxed()
            }),
            backoff_fn: None,
            restart_counter_reset_after: None, // no child-level reset
        };

        // meltdown if 3 fails happen within max_seconds=10 => max_restarts=2 => meltdown on #3
        // but if we wait 3s => meltdown log is cleared => final fail => meltdown log=1 => no meltdown
        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 2,
            max_seconds: 10,
            restart_counter_reset_after: Some(2), // if quiet >=2s => meltdown log cleared
        };

        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };
        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_restart_counter_reset_after_improved".into()),
            Supervisor,
            args,
        )
        .await?;

        // Wait for 2 quick fails => meltdown log=2 => then child is quiet 3s => meltdown log cleared => final fail => meltdown log=1 => no meltdown
        sleep(Duration::from_secs(4)).await;

        // forcibly stop => no meltdown
        sup_ref.stop(None);
        let _ = sup_handle.await;

        let final_st = read_final_supervisor_state("test_restart_counter_reset_after_improved");
        assert_eq!(sup_ref.get_status(), ActorStatus::Stopped);
        assert_eq!(
            final_st.restart_log.len(),
            1,
            "After clearing, we only see a single fail in meltdown log"
        );

        // The child was actually spawned 4 times:
        //  - Start #1 => fail #1 => restart => #2 => fail #2 => restart => #3
        //  - Then quiet 3s => meltdown log cleared => fail #3 => meltdown log=1 => restart => #4
        assert_eq!(read_actor_call_count("reset-test"), 4);

        Ok(())
    }

    #[ractor::concurrency::test]
    async fn test_child_level_restart_counter_reset_after() -> Result<(), Box<dyn std::error::Error>>
    {
        before_each();

        // The child fails 2 times => restarts => after 3s quiet, we reset
        // => final fail is treated like "fail #1" from the child's perspective
        // => no meltdown triggered
        // => total 3 fails => so the child is spawned 4 times
        let behavior = ChildBehavior::FailWaitFail {
            initial_fails: 2,
            wait_ms: 3000,
            final_fails: 1,
            current: Arc::new(AtomicU64::new(0)),
        };

        let mut child_spec = make_child_spec("child-reset", Restart::Permanent, behavior);
        // This time we do a child-level restart_counter_reset_after=2
        child_spec.restart_counter_reset_after = Some(2);

        // meltdown won't happen quickly because max_restarts=5
        let options = SupervisorOptions {
            strategy: Strategy::OneForOne,
            max_restarts: 5,
            max_seconds: 30,
            restart_counter_reset_after: None,
        };
        let args = SupervisorArguments {
            child_specs: vec![child_spec],
            options,
        };

        let (sup_ref, sup_handle) = Actor::spawn(
            Some("test_child_level_restart_counter_reset_after".into()),
            Supervisor,
            args,
        )
        .await?;

        // first 2 fails happen quickly => child is started 3 times so far
        sleep(Duration::from_millis(100)).await;
        let st1 = call_t!(sup_ref, SupervisorMsg::InspectState, 500).unwrap();
        let cfs1 = st1.child_failure_state.get("child-reset").unwrap();
        assert_eq!(cfs1.restart_count, 2);

        // After 3s quiet => child's restart_count is reset
        sleep(Duration::from_secs(3)).await;

        // final fail => from the child's perspective it's now fail #1 => no meltdown
        sup_ref.stop(None);
        let _ = sup_handle.await;

        let final_st = read_final_supervisor_state("test_child_level_restart_counter_reset_after");
        let cfs2 = final_st.child_failure_state.get("child-reset").unwrap();
        assert_eq!(
            cfs2.restart_count, 1,
            "child-level reset => next fail sees count=1"
        );

        // total spawns = 4
        assert_eq!(read_actor_call_count("child-reset"), 4);

        Ok(())
    }

    //
    // Demo: nested supervisors
    //
    #[ractor::concurrency::test]
    async fn test_nested_supervisors() -> Result<(), Box<dyn std::error::Error>> {
        before_each();

        async fn spawn_subsupervisor(
            sup_cell: ActorCell,
            id: String,
            args: SupervisorArguments,
        ) -> Result<ActorCell, SpawnErr> {
            let (sub_sup_ref, _join) =
                Actor::spawn_linked(Some(id.into()), Supervisor, args, sup_cell).await?;
            Ok(sub_sup_ref.get_cell())
        }

        // The "sub-sup" is itself a supervisor that spawns "leaf-worker"
        let sub_sup_spec = ChildSpec {
            id: "sub-sup".to_string(),
            restart: Restart::Permanent,
            spawn_fn: Box::new(move |cell, id| {
                let leaf_child = ChildSpec {
                    id: "leaf-worker".to_string(),
                    restart: Restart::Transient,
                    spawn_fn: Box::new(|c, i| {
                        // a child that fails once after 300ms
                        let bh = ChildBehavior::DelayedFail { ms: 300 };
                        spawn_test_child(c, i, bh).boxed()
                    }),
                    backoff_fn: None,
                    restart_counter_reset_after: None,
                };

                let sub_sup_args = SupervisorArguments {
                    child_specs: vec![leaf_child],
                    options: SupervisorOptions {
                        strategy: Strategy::OneForOne,
                        max_restarts: 1, // meltdown on 2nd fail
                        max_seconds: 2,
                        restart_counter_reset_after: None,
                    },
                };
                spawn_subsupervisor(cell, id, sub_sup_args).boxed()
            }),
            backoff_fn: None,
            restart_counter_reset_after: None,
        };

        // root supervisor that manages sub-sup
        let root_args = SupervisorArguments {
            child_specs: vec![sub_sup_spec],
            options: SupervisorOptions {
                strategy: Strategy::OneForOne,
                max_restarts: 1, // meltdown if "sub-sup" fails 2 times
                max_seconds: 5,
                restart_counter_reset_after: None,
            },
        };

        let (root_sup_ref, root_handle) =
            Actor::spawn(Some("root-sup".into()), Supervisor, root_args).await?;

        // Wait for "leaf-worker" to fail once
        sleep(Duration::from_millis(600)).await;
        assert_eq!(root_sup_ref.get_status(), ActorStatus::Running);

        // Stop the root
        root_sup_ref.stop(None);
        let _ = root_handle.await;

        let root_final = read_final_supervisor_state("root-sup");
        let sub_final = read_final_supervisor_state("sub-sup");

        assert_eq!(root_final.restart_log.len(), 0);
        assert_eq!(sub_final.restart_log.len(), 1);

        assert_eq!(read_actor_call_count("leaf-worker"), 2);

        Ok(())
    }
}
