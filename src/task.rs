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

#[derive(Clone)]
pub struct TaskFn(Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>);

impl TaskFn {
    pub fn new<F, Fut>(factory: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        TaskFn(Arc::new(move || Box::pin(factory())))
    }
}

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
        myself: ActorRef<Self::Msg>,
        task: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        (task.0)().await;
        myself.stop(None);
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

    pub async fn spawn_task<F, Fut>(
        supervisor: ActorRef<TaskSupervisorMsg>,
        task: F,
        options: TaskOptions,
    ) -> Result<String, ActorProcessingErr>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let child_id = options.name;
        let task_wrapper = TaskFn::new(task);

        let spec = ChildSpec {
            id: child_id.clone(),
            spawn_fn: Arc::new({
                let task_wrapper = task_wrapper.clone();
                move |sup, id| spawn_task_actor(id, task_wrapper.clone(), sup).boxed()
            }),
            restart: options.restart,
            backoff_fn: options.backoff_fn,
            restart_counter_reset_after: options.restart_counter_reset_after,
        };

        DynamicSupervisor::spawn_child(supervisor, spec).await?;
        Ok(child_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ractor::{
        call,
        concurrency::{sleep, Duration},
        ActorStatus,
    };
    use serial_test::serial;
    use tokio::sync::mpsc;

    async fn before_each() {
        sleep(Duration::from_millis(10)).await;
    }

    #[ractor::concurrency::test]
    #[serial]
    async fn test_basic_task_execution() {
        before_each().await;

        let (supervisor, handle) = TaskSupervisor::spawn(
            "test-supervisor".into(),
            TaskSupervisorOptions {
                max_children: Some(10),
                max_restarts: 3,
                max_seconds: 1,
                restart_counter_reset_after: Some(1000),
            },
        )
        .await
        .unwrap();

        let (tx, mut rx) = mpsc::channel(1);

        let task_id = TaskSupervisor::spawn_task(
            supervisor.clone(),
            move || {
                // Clone again for each restart
                let tx = tx.clone();
                async move {
                    tx.send(()).await.unwrap();
                }
            },
            TaskOptions::new().name("background-task".into()),
        )
        .await
        .unwrap();

        rx.recv().await.expect("Task should have executed");
        sleep(Duration::from_millis(100)).await;
        let state = call!(supervisor, DynamicSupervisorMsg::InspectState).unwrap();

        assert!(!state.active_children.contains_key(&task_id));

        supervisor.stop(None);
        let _ = handle.await;
    }

    #[ractor::concurrency::test]
    #[serial]
    async fn test_task_termination() {
        before_each().await;

        let (supervisor, handle) = TaskSupervisor::spawn(
            "test-supervisor".into(),
            DynamicSupervisorOptions {
                max_children: Some(10),
                max_restarts: 3,
                max_seconds: 1,
                restart_counter_reset_after: Some(1000),
            },
        )
        .await
        .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        let task_id = TaskSupervisor::spawn_task(
            supervisor.clone(),
            move || {
                // Clone again for each restart
                let tx = tx.clone();
                async move {
                    sleep(Duration::from_secs(10)).await;
                    tx.send(()).await.unwrap();
                }
            },
            TaskOptions::new().restart_policy(Restart::Permanent),
        )
        .await
        .unwrap();

        // Terminate before completion
        TaskSupervisor::terminate_task(supervisor.clone(), task_id.clone())
            .await
            .unwrap();

        // Verify task didn't complete
        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(result.is_err(), "Task should have been terminated");

        supervisor.stop(None);
        let _ = handle.await;
    }

    #[ractor::concurrency::test]
    #[serial]
    async fn test_restart_policy() {
        before_each().await;

        let (supervisor, handle) = TaskSupervisor::spawn(
            "test-supervisor".into(),
            DynamicSupervisorOptions {
                max_children: Some(10),
                max_restarts: 3,
                max_seconds: 1,
                restart_counter_reset_after: Some(1000),
            },
        )
        .await
        .unwrap();

        let (tx, mut rx) = mpsc::channel(3);
        let _task_id = TaskSupervisor::spawn_task(
            supervisor.clone(),
            move || {
                // Clone again for each restart
                let tx = tx.clone();
                async move {
                    tx.send(()).await.unwrap();
                    panic!("Simulated failure");
                }
            },
            TaskOptions::new()
                .restart_policy(Restart::Transient)
                .name("restart-test".into()),
        )
        .await
        .unwrap();

        // Verify multiple restarts
        for _ in 0..4 {
            rx.recv().await.expect("Task should have restarted");
        }

        // Should be terminated after 3 failures (+1 initial)
        sleep(Duration::from_millis(100)).await;
        assert!(!supervisor
            .get_children()
            .iter()
            .any(|cell| cell.get_status() == ActorStatus::Running));

        supervisor.stop(None);
        let _ = handle.await;
    }
}
