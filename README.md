# ractor-supervisor

A **pure-Rust** supervisor built atop the [`ractor`](https://github.com/slawlor/ractor) framework. It provides OTP-style supervision trees so you can define how child actors should be restarted under different failure conditions, along with meltdown logic to prevent runaway restart loops.

## Overview

`ractor-supervisor` is inspired by the way Erlang/Elixir OTP handles actor supervision. It helps you:

1. Define **how** you want to restart failing children—through different **supervision strategies**.
2. Configure **meltdown thresholds** so that if too many restarts occur in a short period, the supervisor itself shuts down abnormally.
3. Optionally add child-level **backoff** or meltdown reset intervals for even more granular control.

## Installation

Add the following to your `Cargo.toml`:
```toml
[dependencies]
ractor-supervisor = "0.1"
ractor = "0.14"
```

To get started, you should already have familiarity with [`ractor`](https://github.com/slawlor/ractor). This crate builds on top of `ractor`’s actor model.

## SupervisorOptions

These options control the **supervisor-wide** meltdown logic and overall restart behavior:

- **`strategy`**  
  Defines which children get restarted when any one child fails:
  - **OneForOne**: Only the failing child is restarted.
  - **OneForAll**: If any child fails, *all* children are stopped and restarted.
  - **RestForOne**: The failing child and all subsequent children (in definition order) are stopped and restarted.
  
- **`max_restarts`** + **`max_seconds`**  
  Meltdown window. If `max_restarts` is exceeded within `max_seconds`, the supervisor triggers a meltdown and stops abnormally.

- **`restart_counter_reset_after`**  
  If the supervisor sees no restarts for this many seconds, it **resets** its meltdown counter back to zero. This prevents old failures from accumulating indefinitely.

## ChildSpec

These specs define **how** each child actor is spawned and restarted. You provide:

- **`id`**: A unique identifier for the child (used in logs, meltdown tracking, etc.).
- **`restart`**: One of `Permanent` (always restart), `Transient` (only if fails abnormally), or `Temporary` (never restart).
- **`spawn_fn`**: A user-provided function that spawns (and links) the child actor; typically calls `Actor::spawn_linked`.
- **`backoff_fn`** (optional): A function returning an extra `Duration` delay before restarting this child (e.g., exponential backoff).
- **`restart_counter_reset_after`** (optional): If the child remains alive for that many seconds, its own restart count is reset next time it fails.

## Multi-Level Supervision Trees

Supervisors can manage other **supervisors** as children, forming a **hierarchical** or **tree** structure. This way, different subsystems can each have their own meltdown thresholds or strategies. A meltdown in one subtree doesn’t necessarily mean the entire application must go down, unless the top-level supervisor is triggered.

For example, you might have:
- **Root Supervisor** (OneForOne)
  - **Sub-supervisor A** (OneForAll)
    - Child actor #1
    - Child actor #2
  - **Sub-supervisor B** (RestForOne)
    - Child actor #3
    - Child actor #4

With nested supervision, you can isolate failures and keep the rest of your system running.


## Usage

Below is a **full** code snippet showing how to configure and spawn the supervisor. We skip demonstrating the child actor implementation itself—assuming you already have one. Notice how we pass in a custom `spawn_my_worker` function, define meltdown thresholds, and pick a specific restart strategy.

```rust
use ractor::Actor;
use ractor_supervisor::*; // assuming your crate is named ractor_supervisor
use std::{time::Duration, sync::Arc};
use tokio::time::Instant;
use futures_util::FutureExt;

// A minimal child actor that simply does some work in `handle`.
struct MyWorker;

#[ractor::async_trait]
impl Actor for MyWorker {
    type Msg = ();
    type State = ();
    type Arguments = ();

    // Called before the actor fully starts. We can set up the actor’s internal state here.
    async fn pre_start(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        _args: Self::Arguments,
    ) -> Result<Self::State, ractor::ActorProcessingErr> {
        Ok(())
    }

    // The main message handler. This is where you implement your actor’s behavior.
    async fn handle(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        _msg: Self::Msg,
        _state: &mut Self::State
    ) -> Result<(), ractor::ActorProcessingErr> {
        // do some work...
        Ok(())
    }
}

// A function to spawn the child actor. This will be used in ChildSpec::spawn_fn.
async fn spawn_my_worker(
    supervisor_cell: ractor::ActorCell,
    child_id: String
) -> Result<ractor::ActorCell, ractor::SpawnErr> {
    // We name the child actor using `child_spec.id` (though naming is optional).
    let (child_ref, _join) = MyWorker::spawn_linked(
        Some(child_id), // actor name
        MyWorker,                    // actor instance
        (),                          // arguments
        supervisor_cell             // link to the supervisor
    ).await?;
    Ok(child_ref.get_cell())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A child-level backoff function that implements exponential backoff after the second failure.
    // Return Some(delay) to make the supervisor wait before restarting this child.
    let my_backoff: ChildBackoffFn = Arc::new(
        |_child_id: &str, restart_count: usize, last_fail: Instant, child_reset_after: Option<u64>| {
            // On the first failure, restart immediately (None).
            // After the second failure, double the delay each time (exponential).
            if restart_count <= 1 {
                None
            } else {
                Some(Duration::from_secs(1 << restart_count))
            }
        }
    );

    // This specification describes exactly how to manage our single child actor.
    let child_spec = ChildSpec {
        id: "myworker".into(),  // Unique identifier for meltdown logs and debugging.
        restart: Restart::Transient, // Only restart if the child fails abnormally.
        spawn_fn: Box::new(|cell, id| spawn_my_worker(cell, id).boxed()),
        backoff_fn: Some(my_backoff), // Apply our custom exponential backoff on restarts.
        // If the child remains up for 60s, its individual failure counter resets to 0 next time it fails.
        restart_counter_reset_after: Some(60),
    };

    // Supervisor-level meltdown configuration. If more than 5 restarts occur within 10s, meltdown is triggered.
    // Also, if we stay quiet for 30s (no restarts), the meltdown log resets.
    let options = SupervisorOptions {
        strategy: Strategy::OneForOne,  // If one child fails, only that child is restarted.
        max_restarts: 5,               // Permit up to 5 restarts in the meltdown window.
        max_seconds: 10,               // The meltdown window (in seconds).
        restart_counter_reset_after: Some(30), // If no failures for 30s, meltdown log is cleared.
    };

    // Group all child specs and meltdown options together:
    let args = SupervisorArguments {
        child_specs: vec![child_spec], // We only have one child in this example
        options,
    };

    // Spawn the supervisor with our arguments.
    let (sup_ref, sup_handle) = Actor::spawn(
        None,        // no name for the supervisor
        Supervisor,  // the Supervisor actor
        args
    ).await?;

    let _ = sup_ref.kill();
    let _ = sup_handle.await;

    Ok(())
}
```

## License

This project is licensed under [MIT](LICENSE). It is heavily inspired by Elixir/Erlang OTP patterns, but implemented in pure Rust for the [`ractor`](https://github.com/slawlor/ractor) framework