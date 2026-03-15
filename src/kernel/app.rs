//! App 构建器：各层通过 `register()` 注册命令、路由和后台任务。
//!
//! 设计目标：boot.rs 不再直接引用 logic / services 的具体类型。
//! 命令注册由 commands 层负责，路由+服务注册由 services 层负责。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

#[cfg(feature = "runtime-dispatcher")]
use crate::commands::Command;

#[cfg(feature = "runtime-api")]
use crate::runtime::api::ApiClient;

#[cfg(feature = "runtime-permission")]
use crate::runtime::permission::AccessControl;

#[cfg(feature = "runtime-pool")]
use crate::runtime::pool::Pool;

#[cfg(feature = "runtime-registry")]
use crate::runtime::registry::CommandRegistry;

#[cfg(feature = "runtime-ws")]
use crate::runtime::ws::WsManager;

pub struct App {
    #[cfg(feature = "runtime-registry")]
    registry: CommandRegistry,
    router: Router,
    tasks: Vec<Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>>,

    // ── 共享基础设施（由 runtime::init() 填充） ───────────────────────────────
    #[cfg(feature = "runtime-api")]
    pub api: Option<Arc<ApiClient>>,
    #[cfg(feature = "runtime-ws")]
    pub ws: Option<Arc<WsManager>>,
    #[cfg(feature = "runtime-pool")]
    pub pool: Option<Arc<Pool>>,
    #[cfg(feature = "runtime-permission")]
    pub access: Option<Arc<AccessControl>>,
}

impl App {
    /// 创建空的 App 管理器。
    /// runtime 模块由 `runtime::init()` 填充。
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "runtime-registry")]
            registry: CommandRegistry::new(),
            router: Router::new(),
            tasks: Vec::new(),
            #[cfg(feature = "runtime-api")]
            api: None,
            #[cfg(feature = "runtime-ws")]
            ws: None,
            #[cfg(feature = "runtime-pool")]
            pool: None,
            #[cfg(feature = "runtime-permission")]
            access: None,
        }
    }

    /// 设置 API 客户端（由 runtime::init() 调用）
    #[cfg(feature = "runtime-api")]
    pub fn set_api(&mut self, api: Arc<ApiClient>) {
        self.api = Some(api);
    }

    /// 设置 WebSocket 管理器（由 runtime::init() 调用）
    #[cfg(feature = "runtime-ws")]
    pub fn set_ws(&mut self, ws: Arc<WsManager>) {
        self.ws = Some(ws);
    }

    /// 设置消息池（由 runtime::init() 调用）
    #[cfg(feature = "runtime-pool")]
    pub fn set_pool(&mut self, pool: Arc<Pool>) {
        self.pool = Some(pool);
    }

    /// 设置权限控制（由 runtime::init() 调用）
    #[cfg(feature = "runtime-permission")]
    pub fn set_access(&mut self, access: Arc<AccessControl>) {
        self.access = Some(access);
    }

    /// 注册一条命令到内部 registry。
    /// 在注册时检查依赖，不满足的命令会被跳过。
    #[cfg(all(feature = "runtime-dispatcher", feature = "runtime-ws", feature = "runtime-pool"))]
    pub fn command(&mut self, cmd: Arc<dyn Command>) {
        self.registry.register(cmd, &self.pool, &self.ws);
    }

    #[cfg(all(feature = "runtime-dispatcher", not(feature = "runtime-ws"), feature = "runtime-pool"))]
    pub fn command(&mut self, cmd: Arc<dyn Command>) {
        self.registry.register(cmd, &self.pool);
    }

    #[cfg(all(feature = "runtime-dispatcher", feature = "runtime-ws", not(feature = "runtime-pool")))]
    pub fn command(&mut self, cmd: Arc<dyn Command>) {
        self.registry.register(cmd, &self.ws);
    }

    #[cfg(all(feature = "runtime-dispatcher", not(feature = "runtime-ws"), not(feature = "runtime-pool")))]
    pub fn command(&mut self, cmd: Arc<dyn Command>) {
        self.registry.register(cmd);
    }

    /// 合并一个已绑定 State 的子路由（调用方先 `.with_state(...)` 再 merge）。
    #[allow(dead_code)]
    pub fn merge(&mut self, router: Router) {
        let old = std::mem::replace(&mut self.router, Router::new());
        self.router = old.merge(router);
    }

    /// 注册后台任务，由 `into_router()` 时统一 spawn。
    #[allow(dead_code)]
    pub fn spawn(&mut self, task: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
        self.tasks.push(Box::pin(task));
    }

    /// 消耗 registry 返回 `Arc<CommandRegistry>`，用于创建 Dispatcher。
    /// 调用后内部 registry 重置为空。
    #[cfg(feature = "runtime-registry")]
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
