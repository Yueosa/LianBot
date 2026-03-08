//! App 构建器：各层通过 `register()` 注册命令、路由和后台任务。
//!
//! 设计目标：boot.rs 不再直接引用 logic / services 的具体类型。
//! 命令注册由 commands 层负责，路由+服务注册由 services 层负责。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use crate::commands::Command;
use crate::runtime::{
    api::ApiClient,
    permission::AccessControl,
    pool::Pool,
    registry::CommandRegistry,
    ws::WsManager,
};

pub struct App {
    registry: CommandRegistry,
    router: Router,
    tasks: Vec<Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>>,

    // ── 共享基础设施（注册函数可读取） ──────────────────────────────────────────
    pub api: Arc<ApiClient>,
    pub ws: Arc<WsManager>,
    pub pool: Option<Arc<Pool>>,
    pub access: Arc<AccessControl>,
}

impl App {
    pub fn new(
        api: Arc<ApiClient>,
        ws: Arc<WsManager>,
        pool: Option<Arc<Pool>>,
        access: Arc<AccessControl>,
    ) -> Self {
        Self {
            registry: CommandRegistry::new(),
            router: Router::new(),
            tasks: Vec::new(),
            api,
            ws,
            pool,
            access,
        }
    }

    /// 注册一条命令到内部 registry。
    pub fn command(&mut self, cmd: Arc<dyn Command>) {
        self.registry.register(cmd);
    }

    /// 合并一个已绑定 State 的子路由（调用方先 `.with_state(...)` 再 merge）。
    pub fn merge(&mut self, router: Router) {
        let old = std::mem::replace(&mut self.router, Router::new());
        self.router = old.merge(router);
    }

    /// 注册后台任务，由 `into_router()` 时统一 spawn。
    pub fn spawn(&mut self, task: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
        self.tasks.push(Box::pin(task));
    }

    /// 消耗 registry 返回 `Arc<CommandRegistry>`，用于创建 Dispatcher。
    /// 调用后内部 registry 重置为空。
    pub fn take_registry(&mut self) -> Arc<CommandRegistry> {
        Arc::new(std::mem::replace(&mut self.registry, CommandRegistry::new()))
    }

    /// 消耗自身：spawn 所有后台任务，返回最终 Router 和任务句柄。
    pub fn into_router(self) -> (Router, Vec<tokio::task::JoinHandle<()>>) {
        let handles: Vec<_> = self.tasks.into_iter().map(|task| {
            tokio::spawn(async move {
                if let Err(e) = task.await {
                    tracing::error!("后台任务异常退出: {e:#}");
                }
            })
        }).collect();
        (self.router, handles)
    }
}
