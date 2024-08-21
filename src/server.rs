use axum::routing::{get, post};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use crate::command::Command;
use crate::error::Error;
use crate::plans::Plan;
use crate::tasks::{TaskSpec, TaskStatus};

mod handlers;
mod plan;
mod run;


pub struct Server {
    pub plans: Mutex<HashMap<Uuid, Arc<Mutex<Plan>>>>,
    pub tasks: Mutex<HashMap<Uuid, Arc<Mutex<ServerTask>>>>,
    pub verbose: bool,
}

impl Server {
    pub fn new() -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            verbose: false,
        }
    }
}


pub enum ServerError {
    InternalServerError,
    PlanNotFound(Uuid),
    TaskNotFound(Uuid),
}

impl axum::response::IntoResponse for ServerError {
    fn into_response(self) -> axum::http::Response<axum::body::Body> {
        match self {
            ServerError::InternalServerError => {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error"
                ).into_response()
            }
            ServerError::PlanNotFound(id) => {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    format!("Plan not found: {:?}", id)
                ).into_response()
            }
            ServerError::TaskNotFound(id) => {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    format!("Task not found: {:?}", id)
                ).into_response()
            }
        }
    }
}


#[derive(Debug)]
pub struct ServerTask {
    pub spec: TaskSpec,
    pub status: TaskStatus,
    pub running: Option<Arc<Command>>,
    pub finished: Arc<Notify>,
    pub error: Option<Error>,
}


pub async fn serve(
    server: Arc<Server>,
    listener: tokio::net::TcpListener
) -> Result<(), std::io::Error> {
    let app = axum::Router::new()
        .route("/plan/:plan_id", get(handlers::get_plan).post(handlers::plan))
        .route("/plans", get(handlers::list_plans).post(handlers::create_plan))
        .route("/tasks", get(handlers::list_tasks).post(handlers::create_task))
        .route("/tasks/:task_id/output", get(handlers::task_output_stream))
        .route("/tasks/:task_id/start", post(handlers::start_task))
        .with_state(server);
    axum::serve(listener, app).await
}
