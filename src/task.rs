use futures_util::FutureExt;
use ractor::concurrency::JoinHandle;
use ractor::{Actor, ActorCell, ActorName, ActorProcessingErr, ActorRef, SpawnErr};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::ChildSpec;
use crate::{
    ChildBackoffFn, DynamicSupervisor, DynamicSupervisorMsg, DynamicSupervisorOptions, Restart,
};

pub struct TaskActor;

pub enum TaskActorMessage {
    Run { task: TaskFn },
}

pub type TaskFn = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[ractor::async_trait]
impl Actor for TaskActor {
    type Msg = TaskActorMessage;
    type State = TaskFn;
    type Arguments = TaskFn;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        task: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(task)
    }

    async fn post_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        task: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        (task)().await;

        Ok(())
    }
}

pub type TaskSupervisorMsg = DynamicSupervisorMsg;
pub type TaskSupervisorOptions = DynamicSupervisorOptions;

pub struct TaskSupervisor;

pub struct TaskOptions {
    pub name: ActorName,
    pub restart: Restart,
    pub backoff_fn: Option<ChildBackoffFn>,
    pub restart_counter_reset_after: Option<u64>,
}

impl Default for TaskOptions {
    fn default() -> Self {
        Self {
            name: Uuid::new_v4().to_string(),
            restart: Restart::Temporary,
            backoff_fn: None,
            restart_counter_reset_after: None,
        }
    }
}

impl TaskOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = name;
        self
    }

    pub fn restart_policy(mut self, restart: Restart) -> Self {
        self.restart = restart;
        self
    }

    pub fn backoff_fn(mut self, backoff_fn: ChildBackoffFn) -> Self {
        self.backoff_fn = Some(backoff_fn);
        self
    }

    pub fn restart_counter_reset_after(mut self, seconds: u64) -> Self {
        self.restart_counter_reset_after = Some(seconds);
        self
    }
}

impl TaskSupervisor {
    pub async fn spawn(
        name: ActorName,
        options: TaskSupervisorOptions,
    ) -> Result<(ActorRef<TaskSupervisorMsg>, JoinHandle<()>), SpawnErr> {
        DynamicSupervisor::spawn(name, options).await
    }

    pub async fn spawn_task<F>(
        task: fn() -> Pin<Box<dyn Future<Output = ()> + Send>>,
        supervisor: ActorRef<TaskSupervisorMsg>,
        options: TaskOptions,
    ) -> Result<(), ActorProcessingErr> {
        let child_id = options.name;
        let task_ref = Arc::new(task);
        let spec = ChildSpec {
            id: child_id.clone(),
            spawn_fn: Arc::new(move |sup, id| spawn_task_actor(id, task_ref.clone(), sup).boxed()),
            restart: options.restart,
            backoff_fn: options.backoff_fn,
            restart_counter_reset_after: options.restart_counter_reset_after,
        };

        DynamicSupervisor::spawn_child(supervisor, spec).await?;
        Ok(())
    }

    pub async fn terminate_task(
        supervisor: ActorRef<TaskSupervisorMsg>,
        task_id: String,
    ) -> Result<(), ActorProcessingErr> {
        DynamicSupervisor::terminate_child(supervisor, task_id).await
    }
}

async fn spawn_task_actor(id: String, task: TaskFn, sup: ActorCell) -> Result<ActorCell, SpawnErr> {
    let (child_ref, _join) = DynamicSupervisor::spawn_linked(id, TaskActor, task, sup).await?;

    Ok(child_ref.get_cell())
}
