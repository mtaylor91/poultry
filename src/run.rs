use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::Error;
use crate::server::{Server, ServerError};
use crate::tasks::{TaskSpec, TaskStatus, TaskState};


pub fn start_task(
    server: Arc<Server>,
    task_id: Uuid
) -> Pin<Box<dyn Future<Output = Result<TaskState, ServerError>> + Send>> {
    Box::pin(async move {
        if server.verbose {
            eprintln!("Starting task: {:?}", task_id);
        }

        let task = match server.tasks.lock().await.get(&task_id) {
            Some(task) => task.clone(),
            None => {
                return Err(ServerError::TaskNotFound(task_id));
            }
        };

        let mut task = task.lock().await;
        match task.spec {
            TaskSpec::Command { .. } => {
                task.status = TaskStatus::Running;
            }
            TaskSpec::TaskGroup { .. } => {
                task.status = TaskStatus::Waiting;
            }
            TaskSpec::TaskList { .. } => {
                task.status = TaskStatus::Waiting;
            }
        }

        tokio::spawn(async move {
            run_task(server, task_id).await;
        });

        Ok(TaskState {
            id: task_id,
            spec: task.spec.clone(),
            status: task.status.clone(),
        })
    })
}


async fn run_task(
    server: Arc<Server>,
    task_id: Uuid
) {
    let task = match server.tasks.lock().await.get(&task_id) {
        Some(task) => task.clone(),
        None => {
            return;
        }
    };

    let spec = task.lock().await.spec.clone();
    match spec {
        TaskSpec::Command { ref args } => {
            let result = tokio::process::Command::new(&args[0])
                .args(&args[1..])
                .spawn();
            match result {
                Ok(mut process) => {
                    let result = process.wait().await;
                    match result {
                        Ok(status) => {
                            if status.success() {
                                finish_task(server, task_id).await;
                            } else {
                                fail_task(server, task_id, Error::ExitFailure(status))
                                    .await;
                            }
                        }
                        Err(err) => {
                            fail_task(server, task_id,
                                Error::CommandFailed(Arc::new(err))).await;
                        }
                    }
                }
                Err(err) => {
                    fail_task(server, task_id, Error::CommandFailed(Arc::new(err)))
                        .await;
                }
            }
        }
        TaskSpec::TaskGroup { ref parallel } => {
            let mut tasks = vec![];
            for child_id in parallel {
                let server = server.clone();
                tasks.push(async move {
                    // Start the child task
                    start_task(server.clone(), *child_id).await
                        .map_err(|err| {
                            match err {
                                ServerError::TaskNotFound(_) => {
                                    Error::TaskNotFound(*child_id)
                                }
                            }
                        })?;

                    // Wait for the child task to finish
                    if let Err(_) = wait_task(server.clone(), *child_id).await {
                        Err(Error::TaskFailed(*child_id))
                    } else {
                        Ok(())
                    }
                });
            }

            for task in tasks {
                if let Err(err) = task.await {
                    fail_task(server.clone(), task_id, err).await;
                    return;
                }
            }

            finish_task(server, task_id).await;
        }
        TaskSpec::TaskList { ref serial } => {
            for child_id in serial {
                // Start the child task
                if let Err(err) = start_task(server.clone(), *child_id).await {
                    match err {
                        ServerError::TaskNotFound(_) => {
                            fail_task(server.clone(), task_id,
                                Error::TaskNotFound(*child_id)).await;
                            return;
                        }
                    }
                }

                // Wait for the child task to finish
                if let Err(_) = wait_task(server.clone(), *child_id).await {
                    fail_task(server.clone(), task_id, Error::TaskFailed(*child_id))
                        .await;
                    return;
                }
            }

            finish_task(server, task_id).await;
        }
    }
}


async fn finish_task(
    server: Arc<Server>,
    task_id: Uuid
) {
    if server.verbose {
        eprintln!("Finished task: {:?}", task_id);
    }

    if let Some(task) = server.tasks.lock().await.get(&task_id) {
        let mut task = task.lock().await;
        task.status = TaskStatus::Success;
        task.finished.notify_waiters();
    }
}


async fn fail_task(
    server: Arc<Server>,
    task_id: Uuid,
    error: Error
) {
    if server.verbose {
        eprintln!("Failed task {}: {:?}", task_id, error);
    }

    if let Some(task) = server.tasks.lock().await.get(&task_id) {
        let mut task = task.lock().await;
        task.error = Some(error);
        task.status = TaskStatus::Failure;
        task.finished.notify_waiters();
    }
}


async fn wait_task(
    server: Arc<Server>,
    task_id: Uuid
) -> Result<(), Error> {
    let finished = match server.tasks.lock().await.get(&task_id) {
        Some(task) => task.lock().await.finished.clone(),
        None => {
            return Err(Error::TaskNotFound(task_id));
        }
    };

    finished.notified().await;
    match server.tasks.lock().await.get(&task_id) {
        Some(task) => {
            let task = task.lock().await;
            match task.status {
                TaskStatus::Success => {
                    Ok(())
                }
                TaskStatus::Failure => {
                    Err(task.error.clone().unwrap())
                }
                _ => {
                    eprintln!("Task not finished: {:?}", task_id);
                    Err(Error::TaskFailed(task_id))
                }
            }
        }
        None => {
            Err(Error::TaskNotFound(task_id))
        }
    }
}
