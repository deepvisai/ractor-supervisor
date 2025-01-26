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
//! Strategies apply to **all failure scenarios**, including:
//! - Spawn errors (failures in `pre_start`/`post_start`)
//! - Runtime panics
//! - Normal and abnormal exits
//!
//! Example: If spawning a child fails during pre_start, it will count as a restart and trigger strategy logic
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
//! ## Multi-Level Supervision Trees
//!
//! Supervisors can manage other **supervisors** as children, forming a **hierarchical** or **tree** structure. This way, different subsystems can each have their own meltdown thresholds or strategies. A meltdown in one subtree doesn’t necessarily mean the entire application must go down, unless the top-level supervisor is triggered.
//!
//! For example, you might have:
//! - **Root Supervisor** (OneForOne)
//!   - **Sub-supervisor A** (OneForAll)
//!     - Child actor #1
//!     - Child actor #2
//!   - **Sub-supervisor B** (RestForOne)
//!     - Child actor #3
//!     - Child actor #4
//!
//! With nested supervision, you can isolate failures and keep the rest of your system running.
//!
//! When creating supervisors ensure you use [`Supervisor::spawn_linked`] or [`Supervisor::spawn`] rather than the generic
//! [`Actor::spawn`] methods to maintain proper supervision links.
//!
//! ## Usage
//! 1. **Define** one or more child actors by implementing [`Actor`].
//! 2. For each child, create a [`ChildSpec`] with:
//!    - A [`Restart`] policy,
//!    - A `spawn_fn` that links the child to its supervisor,
//!    - Optional `backoff_fn` / meltdown resets.
//! 3. Configure [`SupervisorOptions`], specifying meltdown thresholds (`max_restarts`, `max_seconds`) and a supervision [`Strategy`].
//! 4. Pass those into [`SupervisorArguments`] and **spawn** your [`Supervisor`] via `Supervisor::spawn(...)`.
//!
//! ## Example
//! ```rust
//! use ractor::Actor;
//! use ractor_supervisor::*;
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
//!         spawn_fn: Arc::new(|cell, id| spawn_my_worker(cell, id).boxed()),
//!         backoff_fn: Some(my_backoff), // Apply our custom exponential backoff on restarts.
//!         // If the child remains up for 60s, its individual failure counter resets to 0 next time it fails.
//!         restart_counter_reset_after: Some(60),
//!     };
//!
//!     // Supervisor-level meltdown configuration. If more than 5 restarts occur within 10s, meltdown is triggered.
//!     // Also, if we stay quiet for 30s (no restarts), the meltdown log resets.
//!     let options = SupervisorOptions {
//!         strategy: SupervisorStrategy::OneForOne,  // If one child fails, only that child is restarted.
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
//!     let (sup_ref, sup_handle) = Supervisor::spawn(
//!         "root".into(), // name for the supervisor
//!         args
//!     ).await?;
//!
//!     let _ = sup_ref.kill();
//!     let _ = sup_handle.await;
//!
//!     Ok(())
//! }
//! ```
pub mod core;
pub mod dynamic;
pub mod supervisor;
pub mod task;

pub use core::*;
pub use dynamic::*;
pub use supervisor::*;
pub use task::*;
